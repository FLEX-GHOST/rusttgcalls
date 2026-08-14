//! FactoryMonitor is a single per-Factory background task that:
//! 1. Generates a VP8 padding packet every keepaliveTickInterval on every
//!    registered PC's video Track, keeping Telegram's SFU video SSRC binding warm.
//! 2. Force-closes PCs stuck in Connecting beyond iceCheckingTimeout.

use crate::models::{ConnState, VP8_PAYLOAD_TYPE};
use crate::wrtc::native::stack::Stack;
use papaya::HashMap as PapayaMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use tokio::time::{Duration, interval};

/// Keepalive cadence — short enough that the SFU never GCs the SSRC binding.
const KEEPALIVE_TICK_INTERVAL: Duration = Duration::from_secs(2);

/// Monitor poll granularity.
const MONITOR_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// iceCheckingTimeout — how long a PC that has NEVER reached Connected
/// may stay in setup before the monitor force-closes it.
const ICE_CHECKING_TIMEOUT: Duration = Duration::from_secs(35);

struct PcMonitorEntry {
    stack: Arc<Stack>,
    checking_ns: AtomicI64,
    ever_connected: AtomicBool,
}

impl PcMonitorEntry {
    fn new(stack: Arc<Stack>) -> Arc<Self> {
        Arc::new(Self {
            stack,
            checking_ns: AtomicI64::new(0),
            ever_connected: AtomicBool::new(false),
        })
    }

    fn tick(&self, do_keepalive: bool) {
        let state = self.stack.state();

        if state != ConnState::Connected {
            if self.ever_connected.load(Ordering::SeqCst) {
                return;
            }
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as i64;
            let start = self.checking_ns.load(Ordering::SeqCst);
            if start == 0 {
                self.checking_ns.store(now, Ordering::SeqCst);
            } else if Duration::from_nanos((now - start).unsigned_abs()) > ICE_CHECKING_TIMEOUT {
                let stack = self.stack.clone();
                tokio::spawn(async move {
                    let _ = stack.close().await;
                });
            }
            return;
        }

        self.ever_connected.store(true, Ordering::SeqCst);
        self.checking_ns.store(0, Ordering::SeqCst);

        if do_keepalive && self.stack.video_ssrc() != 0 {
            let stack = self.stack.clone();
            tokio::spawn(async move {
                // Send 5-byte VP8 padding payload to keep SSRC binding warm on Telegram SFU
                let _ = stack
                    .send_rtp_bytes(
                        VP8_PAYLOAD_TYPE,
                        bytes::Bytes::from_static(&[0x90, 0x80, 0xe0, 0x00, 0x00]),
                        Duration::from_millis(66),
                    )
                    .await;
            });
        }
    }
}

/// FactoryMonitor is a single per-Factory background task.
pub struct FactoryMonitor {
    entries: Arc<PapayaMap<usize, Arc<PcMonitorEntry>>>,
    started: AtomicBool,
    stopped: AtomicBool,
    cancel_tx: tokio::sync::watch::Sender<bool>,
}

impl FactoryMonitor {
    pub fn new() -> Arc<Self> {
        let (cancel_tx, _) = tokio::sync::watch::channel(false);
        Arc::new(Self {
            entries: Arc::new(PapayaMap::new()),
            started: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            cancel_tx,
        })
    }

    /// Start kicks off the monitor background task. Call exactly once.
    pub fn start(self: &Arc<Self>) {
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let entries = self.entries.clone();
        let mut cancel_rx = self.cancel_tx.subscribe();

        tokio::spawn(async move {
            let keepalive_every =
                (KEEPALIVE_TICK_INTERVAL.as_millis() / MONITOR_POLL_INTERVAL.as_millis()) as u64;
            let mut tick_count: u64 = 0;
            let mut ticker = interval(MONITOR_POLL_INTERVAL);

            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        tick_count += 1;
                        let do_keepalive = tick_count.is_multiple_of(keepalive_every);
                        let pin = entries.pin();
                        for (_, entry) in pin.iter() {
                            entry.tick(do_keepalive);
                        }
                    }
                    _ = cancel_rx.changed() => {
                        if *cancel_rx.borrow() {
                            break;
                        }
                    }
                }
            }
        });
    }

    /// Stop cancels the monitor task.
    pub fn stop(&self) {
        if self.stopped.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = self.cancel_tx.send(true);
    }

    /// Register adds a Stack to the monitor's working set.
    pub async fn register(&self, stack: Arc<Stack>) {
        let key = Arc::as_ptr(&stack) as usize;
        let entry = PcMonitorEntry::new(stack);
        self.entries.pin().insert(key, entry);
    }

    /// Unregister removes a Stack from the monitor's working set.
    pub async fn unregister(&self, stack: &Arc<Stack>) {
        let key = Arc::as_ptr(stack) as usize;
        self.entries.pin().remove(&key);
    }
}
