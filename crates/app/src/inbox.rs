//! Thread-safe queue that background threads push into and a gpui task
//! awaits, waking the UI only when there is something to process.

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};

struct State<T> {
    items: Vec<T>,
    waker: Option<Waker>,
    closed: bool,
}

pub struct Inbox<T>(Arc<Mutex<State<T>>>);

impl<T> Clone for Inbox<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> Default for Inbox<T> {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(State {
            items: Vec::new(),
            waker: None,
            closed: false,
        })))
    }
}

impl<T> Inbox<T> {
    pub fn push(&self, item: T) {
        let mut state = self.0.lock().unwrap();
        state.items.push(item);
        if let Some(waker) = state.waker.take() {
            waker.wake();
        }
    }

    /// Takes whatever is queued right now without waiting.
    pub fn take_all(&self) -> Vec<T> {
        std::mem::take(&mut self.0.lock().unwrap().items)
    }

    /// Ends `next_batch` loops once the pending items are drained.
    pub fn close(&self) {
        let mut state = self.0.lock().unwrap();
        state.closed = true;
        if let Some(waker) = state.waker.take() {
            waker.wake();
        }
    }

    /// Resolves with all queued items, or `None` once closed and empty.
    pub fn next_batch(&self) -> impl Future<Output = Option<Vec<T>>> + '_ {
        std::future::poll_fn(move |cx| {
            let mut state = self.0.lock().unwrap();
            if !state.items.is_empty() {
                return Poll::Ready(Some(std::mem::take(&mut state.items)));
            }
            if state.closed {
                return Poll::Ready(None);
            }
            state.waker = Some(cx.waker().clone());
            Poll::Pending
        })
    }
}
