#![deny(unused_must_use)]

#[macro_use]
mod macros;
mod error;
mod statistic;

use async_std::{
    net::UdpSocket,
    task::{self, sleep},
};
use error::Error;
use futures::try_join;
use libc;
use log::{error, info, warn};
use rand::{self, rngs::SmallRng, RngCore, SeedableRng};
use simple_logger;
use std::cell::RefCell;
use std::convert::TryInto;
use std::net::SocketAddr;
use std::os::unix::io::AsRawFd;
use std::time::{Duration, Instant};
use std::{cmp, io, mem, process};

const PKT_LEN: usize = 256;
const RANDOM_DATA_LEN: usize = 2000;

fn main() {
    let exit_code = match task::block_on(main_impl()) {
        Ok(()) => 0,
        Err(e) => {
            error!("Error: {}", e);
            1
        }
    };

    process::exit(exit_code);
}

async fn main_impl() -> Result<(), Error> {
    simple_logger::init().unwrap();

    let mut server = Server::new("0.0.0.0:8044").await?;
    let (mut recv, mut send) = server.split()?;

    try_join!(recv.listen(), send.send_loop())?;
    Ok(())
}

struct Server {
    socket: UdpSocket,
    clients: Clients,
    random_data: Vec<u8>,
    start: Instant,
}

struct ServerRecv<'a> {
    socket: &'a UdpSocket,
    clients: &'a Clients,
    start: &'a Instant,
    statistics: statistic::Delays,
}

struct ServerSend<'a> {
    socket: &'a UdpSocket,
    pkt_cnt: u32,
    clients: &'a Clients,
    start: &'a Instant,
    random_data: &'a [u8],
    random_data_idx: usize,
}

struct Clients {
    clients: RefCell<Vec<SocketAddr>>,
}

struct ClientsIterator<'a> {
    clients: &'a RefCell<Vec<SocketAddr>>,
    idx: usize,
}

impl Server {
    async fn new(addr: &str) -> Result<Self, Error> {
        let addr: SocketAddr = addr.parse()?;
        let socket = UdpSocket::bind(addr).await?;
        set_voice_data_priority(&socket)?;

        Ok(Self {
            socket,
            clients: Default::default(),
            random_data: Self::gen_random_data()?,
            start: Instant::now(),
        })
    }

    fn split(&mut self) -> Result<(ServerRecv, ServerSend), Error> {
        Ok((
            ServerRecv {
                socket: &self.socket,
                clients: &self.clients,
                start: &self.start,
                statistics: Default::default(),
            },
            ServerSend {
                socket: &self.socket,
                pkt_cnt: 0,
                clients: &self.clients,
                start: &self.start,
                random_data: &self.random_data,
                random_data_idx: 0,
            },
        ))
    }

    fn gen_random_data() -> Result<Vec<u8>, Error> {
        let mut res = vec![0; RANDOM_DATA_LEN];
        SmallRng::from_rng(rand::thread_rng())?.fill_bytes(&mut res);
        Ok(res)
    }
}

impl<'a> ServerRecv<'a> {
    async fn listen(&mut self) -> Result<(), Error> {
        const BUF_LEN: usize = 65535;
        let mut buf = vec![0; BUF_LEN];
        loop {
            let (len, addr) = self.socket.recv_from(&mut buf).await?;

            let r = self.on_new_pkt(addr, &buf[..len]);
            if let Err(e) = r {
                warn!("Error handling packet: {}", e);
            }
        }
    }

    fn on_new_pkt(&mut self, addr: SocketAddr, buf: &[u8]) -> Result<(), Error> {
        let pkt_type = buf.get(0);
        match pkt_type {
            Some(b'l') => self.clients.add_new_client(addr),
            Some(b's') => self.clients.remove_client(&addr),
            Some(b'r') => self.on_replay_pkt(&buf)?,
            Some(x) => warn!("Unexpected packet type: {}. len: {}", x, buf.len()),
            None => warn!("Received an empty packet"),
        }

        Ok(())
    }

    fn on_replay_pkt(&mut self, buf: &[u8]) -> Result<(), Error> {
        if buf.len() < 13 {
            return Err(Error::new(format!(
                "Received too short replay packet, len: {}",
                buf.len()
            )));
        }

        let pkt_time = Duration::from_millis(u64::from_be_bytes(buf[5..13].try_into().unwrap()));
        let now = self.start.elapsed();
        let rtt = now
            .checked_sub(pkt_time)
            .ok_or_else(|| Error::new("Replay packet time is bigger than now"))?;

        self.statistics.new_event(rtt);

        Ok(())
    }
}

impl<'a> ServerSend<'a> {
    async fn send_loop(&mut self) -> Result<(), Error> {
        const INTERVAL: Duration = Duration::from_millis(20);

        let mut buf = Vec::with_capacity(PKT_LEN);
        loop {
            let pkt_send_time = Instant::now();

            if !self.clients.is_empty() {
                self.gen_next_pkt(&mut buf)?;
                for addr in self.clients {
                    self.socket.send_to(&buf, &addr).await?;
                }
            }

            let sleep_dur = INTERVAL
                .checked_sub(pkt_send_time.elapsed())
                .unwrap_or(Duration::from_millis(0));

            sleep(sleep_dur).await;
        }
    }

    fn gen_next_pkt(&mut self, buf: &mut Vec<u8>) -> Result<(), Error> {
        buf.clear();
        buf.reserve(PKT_LEN);
        buf.push(b'd');

        self.pkt_cnt += 1;
        buf.extend_from_slice(&self.pkt_cnt.to_be_bytes());

        let time_ms = self.start.elapsed().as_millis() as u64;
        buf.extend_from_slice(&time_ms.to_be_bytes());

        self.fill_with_random(buf);

        Ok(())
    }

    fn fill_with_random(&mut self, buf: &mut Vec<u8>) {
        let mut to_fill = PKT_LEN - buf.len();
        let mut left_data_size = self.random_data.len() - self.random_data_idx;

        while to_fill > 0 {
            let to_copy = cmp::min(to_fill, left_data_size);
            buf.extend_from_slice(
                &self.random_data[self.random_data_idx..self.random_data_idx + to_copy],
            );

            self.random_data_idx += to_copy;
            to_fill -= to_copy;
            left_data_size -= to_copy;

            if self.random_data_idx >= self.random_data.len() {
                self.random_data_idx = 0;
                left_data_size = self.random_data.len();
            }
        }
    }
}

impl Default for Clients {
    fn default() -> Self {
        Self {
            clients: RefCell::new(vec![]),
        }
    }
}

impl Clients {
    fn add_new_client(&self, addr: SocketAddr) {
        let mut clients = self.clients.borrow_mut();
        if !clients.contains(&addr) {
            info!("New client connected: {}", addr);
            clients.push(addr);
        } else {
            info!("Connected is already in the list: {}", addr);
        }
    }

    fn remove_client(&self, addr: &SocketAddr) {
        info!("Client disconnected: {}", addr);
        self.clients.borrow_mut().retain(|v| v != addr);
    }

    #[allow(dead_code)]
    fn len(&self) -> usize {
        self.clients.borrow().len()
    }

    #[allow(dead_code)]
    fn is_empty(&self) -> bool {
        self.clients.borrow().is_empty()
    }
}

impl<'a> IntoIterator for &'a Clients {
    type Item = <ClientsIterator<'a> as Iterator>::Item;
    type IntoIter = ClientsIterator<'a>;

    fn into_iter(self) -> Self::IntoIter {
        ClientsIterator {
            clients: &self.clients,
            idx: 0,
        }
    }
}

impl<'a> Iterator for ClientsIterator<'a> {
    type Item = SocketAddr;

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.clients.borrow().get(self.idx).map(|a| a.clone());
        self.idx += 1;
        item
    }
}

fn set_voice_data_priority(s: &UdpSocket) -> Result<(), Error> {
    const IPTOS_DSCP_EF: libc::c_int = 0x2E << 2;
    let res = unsafe {
        libc::setsockopt(
            s.as_raw_fd(),
            libc::IPPROTO_IP,
            libc::IP_TOS,
            &IPTOS_DSCP_EF as *const _ as *const libc::c_void,
            mem::size_of_val(&IPTOS_DSCP_EF) as u32,
        )
    };

    if res == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error().into())
    }
}
