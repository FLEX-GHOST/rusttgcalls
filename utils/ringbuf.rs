//! RingBuffer is a fixed-capacity byte ring used to capture the tail of
//! a stream (e.g. ffmpeg stderr) without growing without bound.

use parking_lot::Mutex;

struct Inner {
    buf: Vec<u8>,
    head: usize,
    full: bool,
}

/// RingBuffer is a fixed-capacity byte ring used to capture the tail of
/// a stream (e.g. ffmpeg stderr) without growing without bound.
pub struct RingBuffer {
    mu: Mutex<Inner>,
}

impl RingBuffer {
    pub fn new(capacity: usize) -> Self {
        let cap = if capacity == 0 { 4096 } else { capacity };
        Self {
            mu: Mutex::new(Inner {
                buf: vec![0u8; cap],
                head: 0,
                full: false,
            }),
        }
    }

    /// Snapshot returns a copy of the buffered tail bytes in order.
    pub fn snapshot(&self) -> Vec<u8> {
        let inner = self.mu.lock();
        if !inner.full {
            let mut out = vec![0u8; inner.head];
            out.copy_from_slice(&inner.buf[..inner.head]);
            return out;
        }
        let mut out = vec![0u8; inner.buf.len()];
        let tail = inner.buf.len() - inner.head;
        out[..tail].copy_from_slice(&inner.buf[inner.head..]);
        out[tail..].copy_from_slice(&inner.buf[..inner.head]);
        out
    }
}

impl std::io::Write for RingBuffer {
    fn write(&mut self, p: &[u8]) -> std::io::Result<usize> {
        let mut inner = self.mu.lock();
        let n = p.len();
        if n == 0 {
            return Ok(0);
        }
        let cap = inner.buf.len();
        if n >= cap {
            inner.buf.copy_from_slice(&p[n - cap..]);
            inner.head = 0;
            inner.full = true;
            return Ok(n);
        }
        let head = inner.head;
        let written = {
            let space = &mut inner.buf[head..];
            let to_copy = p.len().min(space.len());
            space[..to_copy].copy_from_slice(&p[..to_copy]);
            to_copy
        };
        if written < n {
            inner.buf[..n - written].copy_from_slice(&p[written..]);
        }
        inner.head = (inner.head + n) % cap;
        if !inner.full && inner.head < n {
            inner.full = true;
        }
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
