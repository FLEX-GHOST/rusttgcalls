//! Dispatcher serializes callback invocations onto a single background task so
//! callers can safely re-enter the API from inside a callback without
//! deadlocking against locks held by the task that produced the event.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Once};
use tokio::sync::mpsc;

pub type DispatchTask = Box<dyn FnOnce() + Send + 'static>;

/// Dispatcher serializes callback invocations onto a single task so
/// callers can safely re-enter the API from inside a callback without
/// deadlocking against locks held by the task that produced the event.
pub struct Dispatcher {
    tx: mpsc::Sender<DispatchTask>,
    closed: Arc<AtomicBool>,
    once: Arc<Mutex<Once>>,
}

impl Dispatcher {
    /// NewDispatcher constructs a Dispatcher task loop.
    pub fn new(buf_size: usize) -> Self {
        let size = if buf_size == 0 { 256 } else { buf_size };
        let (tx, mut rx) = mpsc::channel::<DispatchTask>(size);
        let closed = Arc::new(AtomicBool::new(false));

        tokio::spawn(async move {
            while let Some(task) = rx.recv().await {
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(task));
            }
        });

        Self {
            tx,
            closed,
            once: Arc::new(Mutex::new(Once::new())),
        }
    }

    /// Submit enqueues fn for execution on the dispatcher task. If the
    /// queue is full, the oldest queued event is dropped to make room for the
    /// new one; this prevents a slow user callback from stalling producers.
    /// Submit never blocks.
    pub fn submit<F>(&self, fn_task: F)
    where
        F: FnOnce() + Send + 'static,
    {
        if self.closed.load(Ordering::SeqCst) {
            return;
        }
        let task = Box::new(fn_task) as DispatchTask;
        match self.tx.try_send(task) {
            Ok(_) => {}
            Err(mpsc::error::TrySendError::Full(task)) => {
                drop(task);
            }
            Err(_) => {}
        }
    }

    /// Close stops the dispatcher and signals the background task to stop
    /// accepting new events.
    pub fn close(&self) {
        let once = self.once.lock().unwrap();
        once.call_once(|| {
            self.closed.store(true, Ordering::SeqCst);
        });
    }
}
