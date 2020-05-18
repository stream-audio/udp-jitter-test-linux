use std::collections::VecDeque;
use std::fmt::Write;
use std::time::{Duration, Instant};

const QUEUE_LEN: usize = 150;
const DISPLAY_INTERVAL: Duration = Duration::from_secs(2);
const PERCENTILES: [f64; 9] = [0.80, 0.90, 0.95, 0.98, 0.985, 0.99, 0.995, 0.998, 0.999];

pub struct Delays {
    delays: VecDeque<Duration>,
    last_display: Instant,
    sorted_delays: Vec<Duration>,
    last_new_lines: usize,
}

impl Delays {
    pub fn new_event(&mut self, dur: Duration) {
        while self.delays.len() >= QUEUE_LEN {
            self.delays.pop_front();
        }
        self.delays.push_back(dur);

        self.display_statistic();
    }

    fn display_statistic(&mut self) {
        if self.last_display.elapsed() < DISPLAY_INTERVAL || self.delays.is_empty() {
            return;
        }

        self.last_display = Instant::now();

        self.clear_last_output();
        self.last_new_lines = 0;

        eprintln!("Avg: {:.2}ms.", self.calculate_avg());
        self.last_new_lines += 1;

        let percentiles = self.calculate_percentiles();
        eprintln!("{}", self.percentiles_to_str(&percentiles));
        self.last_new_lines += 1;
    }

    fn calculate_avg(&self) -> f64 {
        (self.delays.iter().sum::<Duration>().as_millis() as f64) / self.delays.len() as f64
    }

    fn calculate_percentiles(&mut self) -> Vec<(f64, Duration)> {
        self.sorted_delays.clear();
        self.sorted_delays.extend(self.delays.iter());
        self.sorted_delays.sort_unstable();

        let mut per_dur = Vec::with_capacity(PERCENTILES.len());
        for p in &PERCENTILES {
            let idx = (self.sorted_delays.len() as f64 * p) as usize;
            per_dur.push((*p, self.sorted_delays[idx]));
        }

        per_dur
    }

    fn percentiles_to_str(&mut self, percentiles: &[(f64, Duration)]) -> String {
        let mut per_str = String::new();
        for (i, (p, d)) in percentiles.iter().enumerate() {
            if i > 0 {
                if i % 4 == 0 {
                    per_str.push('\n');
                    self.last_new_lines += 1;
                } else {
                    per_str.push('\t');
                }
            }
            write!(per_str, "{:.1}%: {}ms.", *p * 100., d.as_millis() as u64).unwrap();
        }

        per_str
    }

    fn clear_last_output(&self) {
        const MOVE_UP: &'static str = "\x1b[1A";
        const DEL_LINE: &'static str = "\x1b[K";

        for _ in 0..self.last_new_lines {
            eprint!("{}{}", MOVE_UP, DEL_LINE);
        }
    }
}

impl Default for Delays {
    fn default() -> Self {
        Self {
            delays: VecDeque::with_capacity(QUEUE_LEN),
            last_display: Instant::now(),
            sorted_delays: Vec::with_capacity(QUEUE_LEN),
            last_new_lines: 0,
        }
    }
}
