//! Factory pools per-call inputs including the DTLS cert pool and SSRC counters.

use crate::models::errors::RustTgCallsError;
use crate::wrtc::native::certpool::CertPool;
use crate::wrtc::native::stack::Stack;
use rand::Rng;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use tokio::sync::Mutex;

pub struct Factory {
    cert_pool: Arc<Mutex<CertPool>>,
    ssrc_counter: AtomicU32,
    closed: AtomicBool,
}

pub struct FactoryOptions {
    pub cert_pool_size: usize,
    pub use_direct_transport: bool,
}

impl Default for FactoryOptions {
    fn default() -> Self {
        Self {
            cert_pool_size: 8,
            use_direct_transport: false,
        }
    }
}

impl Factory {
    pub fn new(opts: FactoryOptions) -> Result<Arc<Self>, RustTgCallsError> {
        let size = if opts.cert_pool_size == 0 {
            8
        } else {
            opts.cert_pool_size
        };
        let mut rng = rand::rng();
        let seed: u32 = rng.random();

        Ok(Arc::new(Self {
            cert_pool: Arc::new(Mutex::new(CertPool::new(size))),
            ssrc_counter: AtomicU32::new(seed | 1),
            closed: AtomicBool::new(false),
        }))
    }

    pub async fn new_stack(&self, with_video: bool) -> Result<Arc<Stack>, RustTgCallsError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(RustTgCallsError::Closed);
        }

        let generated = self
            .cert_pool
            .lock()
            .await
            .take()
            .await
            .map_err(|e| RustTgCallsError::Internal(e.to_string()))?;

        let audio_ssrc = self.allocate_ssrc();
        let video_ssrc = if with_video {
            let v = self.allocate_ssrc();
            let _ = self.allocate_ssrc();
            v
        } else {
            0
        };

        let direct = crate::wrtc::native::direct_transport::DirectTransport::new_with_ssrc(
            generated,
            audio_ssrc,
            video_ssrc,
        )
        .await?;

        Ok(Arc::new(Stack::from_direct(direct)))
    }

    pub async fn new_direct_transport(
        &self,
    ) -> Result<Arc<crate::wrtc::native::direct_transport::DirectTransport>, RustTgCallsError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(RustTgCallsError::Closed);
        }

        let generated = self
            .cert_pool
            .lock()
            .await
            .take()
            .await
            .map_err(|e| RustTgCallsError::Internal(e.to_string()))?;

        crate::wrtc::native::direct_transport::DirectTransport::new(generated).await
    }

    pub fn allocate_ssrc(&self) -> u32 {
        loop {
            let v = self.ssrc_counter.fetch_add(1, Ordering::SeqCst);
            if v != 0 {
                return v;
            }
        }
    }

    pub fn close(&self) {
        let _ = self.closed.swap(true, Ordering::SeqCst);
    }
}
