//! Pure Rust library for streaming audio and video into Telegram group calls.
//! The public API mirrors standard Telegram VoIP method names so bot code translates one-to-one.
//!
//! The library is blob-only: signaling JSON is exchanged through your
//! own MTProto client. Two calls are required:
//!
//! ```ignore
//! let params = client.create_call(chat_id).await?;
//! // Exchange JSON with Telegram SFU via MTProto...
//! client.connect(chat_id, &remote_json).await?;
//! client.set_stream_sources(chat_id, from_file("song.mp3", EncodeOptions::default())).await?;
//! ```
//!
//! See README.md for the full pattern.

#[cfg(feature = "jemalloc")]
#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[cfg(feature = "jemalloc")]
#[unsafe(no_mangle)]
pub static _rjem_malloc_conf: &[u8] = b"background_thread:true,dirty_decay_ms:0,muzzy_decay_ms:0\0";

pub mod instances;
pub mod io;
pub mod media;
pub mod models;
pub mod utils;
pub mod wrtc;

pub use instances::{Call, GroupCall, RTMPCall};
pub use media::{
    EncodeOptions, RawBytesSource, SeekableSource, Source, Streams, TRACK_AUDIO, TRACK_VIDEO,
    Track, from_file, from_file_offset, from_raw_audio, from_shell, from_shells, from_url,
    from_url_offset,
};
pub use models::{
    CallInfo, ConnState, Device, MediaState, NetworkInfo, RustTgCallsError, StreamType,
};
pub use wrtc::{Candidate, LocalParams, PeerFactory, RemoteParams, Transport};

use papaya::HashMap as PapayaMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub type OnStreamEndCb =
    Arc<dyn Fn(i64, StreamType, Device, Option<RustTgCallsError>) + Send + Sync>;
pub type OnConnectionChangeCb = Arc<dyn Fn(i64, NetworkInfo) + Send + Sync>;
pub type OnUpgradeCb = Arc<dyn Fn(i64, MediaState) + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkType {
    UDP4 = 1,
    UDP6 = 2,
    TCP4 = 3,
    TCP6 = 4,
}

pub const NETWORK_TYPE_UDP4: NetworkType = NetworkType::UDP4;
pub const NETWORK_TYPE_UDP6: NetworkType = NetworkType::UDP6;
pub const NETWORK_TYPE_TCP4: NetworkType = NetworkType::TCP4;
pub const NETWORK_TYPE_TCP6: NetworkType = NetworkType::TCP6;

#[derive(Clone, Debug)]
pub struct ClientOptions {
    pub ffmpeg_path: String,
    pub connect_timeout_secs: u64,
    pub dispatch_buf: usize,
    pub cert_pool_size: usize,
    pub shared_udp_mux: bool,
    pub ffmpeg_stderr_log: bool,
    pub use_direct_transport: bool,
    pub network_types: Vec<NetworkType>,
    pub debug_logs: bool,
    pub verbose_connection_logs: bool,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            ffmpeg_path: "ffmpeg".to_string(),
            connect_timeout_secs: 10,
            dispatch_buf: 256,
            cert_pool_size: 8,
            shared_udp_mux: false,
            ffmpeg_stderr_log: false,
            use_direct_transport: false,
            network_types: vec![NetworkType::UDP4, NetworkType::UDP6],
            debug_logs: false,
            verbose_connection_logs: false,
        }
    }
}

impl ClientOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_ffmpeg_path(mut self, path: impl Into<String>) -> Self {
        self.ffmpeg_path = path.into();
        self
    }

    pub fn with_debug_logs(mut self) -> Self {
        self.debug_logs = true;
        self
    }

    pub fn with_ffmpeg_stderr_log(mut self) -> Self {
        self.ffmpeg_stderr_log = true;
        self
    }

    pub fn with_shared_udp_mux(mut self) -> Self {
        self.shared_udp_mux = true;
        self
    }

    pub fn with_dtls_cert_pool(mut self, size: usize) -> Self {
        self.cert_pool_size = size;
        self
    }

    pub fn with_dispatch_buffer(mut self, size: usize) -> Self {
        self.dispatch_buf = size;
        self
    }

    pub fn with_network_types(mut self, types: Vec<NetworkType>) -> Self {
        self.network_types = types;
        self
    }

    pub fn with_connect_timeout(mut self, secs: u64) -> Self {
        self.connect_timeout_secs = secs;
        self
    }

    pub fn with_direct_transport(mut self, enabled: bool) -> Self {
        self.use_direct_transport = enabled;
        self
    }

    pub fn with_verbose_connection_logs(mut self) -> Self {
        self.verbose_connection_logs = true;
        self
    }
}

pub struct Client {
    options: ClientOptions,
    factory: Arc<PeerFactory>,
    calls: Arc<PapayaMap<i64, Arc<dyn Call>>>,
    on_stream_end: RwLock<Option<OnStreamEndCb>>,
    on_connection_change: RwLock<Option<OnConnectionChangeCb>>,
    on_upgrade: RwLock<Option<OnUpgradeCb>>,
}

impl Client {
    pub fn new() -> Result<Self, RustTgCallsError> {
        Self::new_with_options(ClientOptions::default())
    }

    pub fn new_with_options(options: ClientOptions) -> Result<Self, RustTgCallsError> {
        let factory_opts = crate::wrtc::peer_factory::FactoryOptions {
            network_types: options
                .network_types
                .iter()
                .copied()
                .map(|n| match n {
                    NetworkType::UDP4 => crate::wrtc::peer_factory::NetworkType::UDP4,
                    NetworkType::UDP6 => crate::wrtc::peer_factory::NetworkType::UDP6,
                    NetworkType::TCP4 => crate::wrtc::peer_factory::NetworkType::TCP4,
                    NetworkType::TCP6 => crate::wrtc::peer_factory::NetworkType::TCP6,
                })
                .collect(),
            cert_pool_size: options.cert_pool_size,
            shared_udp_mux: options.shared_udp_mux,
        };
        let factory = Arc::new(PeerFactory::new_with_options(factory_opts)?);
        Ok(Self {
            options,
            factory,
            calls: Arc::new(PapayaMap::new()),
            on_stream_end: RwLock::new(None),
            on_connection_change: RwLock::new(None),
            on_upgrade: RwLock::new(None),
        })
    }

    pub fn options(&self) -> &ClientOptions {
        &self.options
    }

    pub async fn on_stream_end<F>(&self, cb: F)
    where
        F: Fn(i64, StreamType, Device, Option<RustTgCallsError>) + Send + Sync + 'static,
    {
        *self.on_stream_end.write().await = Some(Arc::new(cb));
    }

    pub async fn on_connection_change<F>(&self, cb: F)
    where
        F: Fn(i64, NetworkInfo) + Send + Sync + 'static,
    {
        *self.on_connection_change.write().await = Some(Arc::new(cb));
    }

    pub async fn on_upgrade<F>(&self, cb: F)
    where
        F: Fn(i64, MediaState) + Send + Sync + 'static,
    {
        *self.on_upgrade.write().await = Some(Arc::new(cb));
    }

    pub async fn create_call(&self, chat_id: i64) -> Result<String, RustTgCallsError> {
        if self.calls.pin().contains_key(&chat_id) {
            return Err(RustTgCallsError::ConnectionExists);
        }
        let on_end_opt = self.on_stream_end.read().await.clone();
        let events = crate::instances::group_call::GroupCallEvents {
            on_stream_end: on_end_opt.map(|cb| {
                Arc::new(move |st, dev, err| {
                    cb(chat_id, st, dev, err);
                }) as crate::instances::group_call::OnStreamEndFn
            }),
            on_connection_change: None,
            on_upgrade: None,
        };
        let call = GroupCall::new_with_events(
            chat_id,
            &self.factory,
            None,
            std::time::Duration::from_secs(self.options.connect_timeout_secs),
            events,
        ).await?;
        let local_json = call.create_local_params()?;
        self.calls.pin().insert(chat_id, call);
        Ok(local_json)
    }

    pub async fn create_rtmp_call(
        &self,
        chat_id: i64,
        rtmp_url: &str,
    ) -> Result<(), RustTgCallsError> {
        if self.calls.pin().contains_key(&chat_id) {
            return Err(RustTgCallsError::ConnectionExists);
        }
        let call = RTMPCall::new(chat_id, rtmp_url);
        self.calls.pin().insert(chat_id, call);
        Ok(())
    }

    pub async fn connect(&self, chat_id: i64, remote_json: &str) -> Result<(), RustTgCallsError> {
        let call = self
            .calls
            .pin()
            .get(&chat_id)
            .cloned()
            .ok_or(RustTgCallsError::ConnectionNotFound)?;
        call.connect(remote_json).await
    }

    pub async fn set_stream_sources(
        &self,
        chat_id: i64,
        streams: Streams,
    ) -> Result<(), RustTgCallsError> {
        let call = self
            .calls
            .pin()
            .get(&chat_id)
            .cloned()
            .ok_or(RustTgCallsError::ConnectionNotFound)?;
        call.set_source(streams).await
    }

    pub async fn pause(&self, chat_id: i64) -> Result<bool, RustTgCallsError> {
        let call = self
            .calls
            .pin()
            .get(&chat_id)
            .cloned()
            .ok_or(RustTgCallsError::ConnectionNotFound)?;
        let changed = call.pause()?;
        if changed && let Some(ref cb) = *self.on_upgrade.read().await {
            cb(chat_id, call.state());
        }
        Ok(changed)
    }

    pub async fn resume(&self, chat_id: i64) -> Result<bool, RustTgCallsError> {
        let call = self
            .calls
            .pin()
            .get(&chat_id)
            .cloned()
            .ok_or(RustTgCallsError::ConnectionNotFound)?;
        let changed = call.resume()?;
        if changed && let Some(ref cb) = *self.on_upgrade.read().await {
            cb(chat_id, call.state());
        }
        Ok(changed)
    }

    pub async fn mute(&self, chat_id: i64) -> Result<bool, RustTgCallsError> {
        let call = self
            .calls
            .pin()
            .get(&chat_id)
            .cloned()
            .ok_or(RustTgCallsError::ConnectionNotFound)?;
        let changed = call.mute()?;
        if changed && let Some(ref cb) = *self.on_upgrade.read().await {
            cb(chat_id, call.state());
        }
        Ok(changed)
    }

    pub async fn unmute(&self, chat_id: i64) -> Result<bool, RustTgCallsError> {
        let call = self
            .calls
            .pin()
            .get(&chat_id)
            .cloned()
            .ok_or(RustTgCallsError::ConnectionNotFound)?;
        let changed = call.unmute()?;
        if changed && let Some(ref cb) = *self.on_upgrade.read().await {
            cb(chat_id, call.state());
        }
        Ok(changed)
    }

    pub async fn seek_by(&self, chat_id: i64, delta_ms: i64) -> Result<(), RustTgCallsError> {
        let call = self
            .calls
            .pin()
            .get(&chat_id)
            .cloned()
            .ok_or(RustTgCallsError::ConnectionNotFound)?;
        call.seek_by(delta_ms)
    }

    pub fn audio_ssrc(&self, chat_id: i64) -> Result<u32, RustTgCallsError> {
        let call = self
            .calls
            .pin()
            .get(&chat_id)
            .cloned()
            .ok_or(RustTgCallsError::ConnectionNotFound)?;
        call.audio_ssrc()
    }

    pub fn time(&self, chat_id: i64) -> Result<u64, RustTgCallsError> {
        let call = self
            .calls
            .pin()
            .get(&chat_id)
            .cloned()
            .ok_or(RustTgCallsError::ConnectionNotFound)?;
        Ok(call.elapsed_ms())
    }

    pub fn state(&self, chat_id: i64) -> Result<MediaState, RustTgCallsError> {
        let call = self
            .calls
            .pin()
            .get(&chat_id)
            .cloned()
            .ok_or(RustTgCallsError::ConnectionNotFound)?;
        Ok(call.state())
    }

    pub async fn stop(&self, chat_id: i64) -> Result<(), RustTgCallsError> {
        if let Some(call) = self.calls.pin().remove(&chat_id) {
            call.stop()?;
        }
        Ok(())
    }

    pub async fn close(&self) -> Result<(), RustTgCallsError> {
        let pin = self.calls.pin();
        for (_, call) in pin.iter() {
            let _ = call.stop();
        }
        pin.clear();
        Ok(())
    }
}
