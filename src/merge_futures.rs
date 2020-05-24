//! Structs to `.await` on multiple futures with reusing memory allocation

use futures::task::{Context, Poll};
use futures::Future;
use std::alloc::Layout;
use std::error::Error as StdError;
use std::fmt;
use std::mem::{self, ManuallyDrop};
use std::pin::Pin;
use std::ptr::NonNull;

type RawVoidPtr = Option<NonNull<u8>>;

/// `FuturesMergerMemoryOwner`, `FuturesMerger` allows you to `.await` on multiple futures
/// without unnecessary memory allocations.
/// Memory is initially allocated to store futures in an array, but later can be reused.
///
/// `FuturesMergerMemoryOwner` is just for keeping memory allocated for futures between
/// different future executions.
/// If you want to use it to await on multiple futures,
/// you should get `FuturesMerger` by calling `borrow`.
#[derive(Debug)]
pub struct FuturesMergerMemoryOwner {
    data: RawVoidPtr,
    capacity: usize,
    to_poll: Vec<usize>,
    layout: Option<Layout>,
    drop_fn: Option<fn(RawVoidPtr, usize) -> ()>,
}

/// `FuturesMerger` allows to await on multiple futures.
/// To get that object please call `FuturesMergerMemoryOwner::borrow`
/// You first need to add all futures by calling `push`.
/// Ones you've added all futures you can call `run` method, and `.await` on the result:
///
/// # Examples:
///
/// ```
/// let mut memory_owner = FuturesMergerMemoryOwner::default();
/// loop {
///     let events = get_new_events().await;
///     let mut future_merger = memory_owner.borrow()?;
///     for event in events {
///         future_merger.push(async_exec_event(event));
///     }
///     future_merger.run().await?;
/// }
/// ```
///
#[derive(Debug)]
pub struct FuturesMerger<'a, F: Future<Output = Result<(), E>>, E: StdError> {
    top: &'a mut FuturesMergerMemoryOwner,
    futures: ManuallyDrop<Vec<F>>,
}

#[must_use = "It does nothing unless you `.await` or poll it"]
#[derive(Debug)]
pub struct FuturesMergerAwait<'a, F: Future<Output = Result<(), E>>, E: StdError> {
    futures: &'a mut Vec<F>,
    to_poll: &'a mut Vec<usize>,
}

#[derive(Debug)]
pub struct WrongLayoutError {
    pub old_layout: Layout,
    pub new_layout: Layout,
}

impl FuturesMergerMemoryOwner {
    pub fn borrow<F: Future<Output = Result<(), E>>, E: StdError>(
        &mut self,
    ) -> Result<FuturesMerger<F, E>, WrongLayoutError> {
        if let Some(layout) = &self.layout {
            let new_layout = get_layout::<F>();
            if *layout != new_layout {
                return Err(WrongLayoutError::new(*layout, new_layout));
            }
        }

        let futures = match self.data.take() {
            None => Vec::new(),
            Some(ptr) => unsafe { Vec::from_raw_parts(ptr.as_ptr() as *mut _, 0, self.capacity) },
        };

        Ok(FuturesMerger {
            top: self,
            futures: ManuallyDrop::new(futures),
        })
    }
}

impl Default for FuturesMergerMemoryOwner {
    fn default() -> Self {
        Self {
            data: None,
            capacity: 0,
            to_poll: vec![],
            layout: None,
            drop_fn: None,
        }
    }
}

impl Drop for FuturesMergerMemoryOwner {
    fn drop(&mut self) {
        if let Some(drop_fn) = self.drop_fn.take() {
            drop_fn(self.data, self.capacity);
        }
    }
}

impl<'a, F: Future<Output = Result<(), E>>, E: StdError> FuturesMerger<'a, F, E> {
    pub fn push(&mut self, fut: F) {
        self.futures.push(fut);
        self.top.to_poll.push(self.futures.len() - 1);
    }

    pub fn reserve(&mut self, additional: usize) {
        self.top.to_poll.reserve(additional);
        self.futures.reserve(additional);
    }

    pub fn run(&mut self) -> FuturesMergerAwait<F, E> {
        FuturesMergerAwait {
            futures: &mut self.futures,
            to_poll: &mut self.top.to_poll,
        }
    }
}

impl<'a, F: Future<Output = Result<(), E>>, E: StdError> Drop for FuturesMerger<'a, F, E> {
    fn drop(&mut self) {
        self.futures.clear();
        self.top.to_poll.clear();

        let cap = self.futures.capacity();
        if cap == 0 {
            unsafe { ManuallyDrop::drop(&mut self.futures) };
            return;
        }

        self.top.data = NonNull::new(self.futures.as_mut_ptr() as *mut _);
        self.top.capacity = cap;

        if self.top.drop_fn.is_none() {
            self.top.layout = Some(get_layout::<F>());
            self.top.drop_fn = Some(drop_vec::<F>);
        }
    }
}

impl<'a, F: Future<Output = Result<(), E>>, E: StdError> Future for FuturesMergerAwait<'a, F, E> {
    type Output = Result<(), E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };

        let mut pending = false;
        let mut i = 0;
        while i < this.to_poll.len() {
            let idx = unsafe { *this.to_poll.get_unchecked(i) };
            let fut = unsafe { this.futures.get_unchecked_mut(idx) };
            let fut = unsafe { Pin::new_unchecked(fut) };
            match fut.poll(cx) {
                Poll::Ready(Ok(())) => {
                    this.to_poll.swap_remove(i);
                }
                Poll::Ready(Err(e)) => {
                    this.to_poll.clear();
                    this.futures.clear();
                    return Poll::Ready(Err(e));
                }
                Poll::Pending => {
                    pending = true;
                    i += 1;
                }
            }
        }

        if pending {
            Poll::Pending
        } else {
            this.to_poll.clear();
            this.futures.clear();
            Poll::Ready(Ok(()))
        }
    }
}

impl<'a, F: Future<Output = Result<(), E>>, E: StdError> Drop for FuturesMergerAwait<'a, F, E> {
    fn drop(&mut self) {
        self.futures.clear();
        self.to_poll.clear();
    }
}

impl WrongLayoutError {
    fn new(old_layout: Layout, new_layout: Layout) -> Self {
        Self {
            old_layout,
            new_layout,
        }
    }
}

impl fmt::Display for WrongLayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "New layout: {{{}:{}}} doesn't match previous one: {{{}:{}}}",
            self.new_layout.size(),
            self.new_layout.align(),
            self.old_layout.size(),
            self.old_layout.align(),
        )
    }
}
impl StdError for WrongLayoutError {}

fn drop_vec<T>(ptr: RawVoidPtr, cap: usize) {
    if let Some(ptr) = ptr {
        unsafe {
            let v: Vec<T> = Vec::from_raw_parts(ptr.as_ptr() as *mut _, 0, cap);
            drop(v);
        }
    }
}

fn get_layout<T>() -> Layout {
    let size = mem::size_of::<T>();
    let align = mem::align_of::<T>();
    Layout::from_size_align(size, align).unwrap()
}
