//! DirectTransport is a pure-Rust Telegram group-call transport engine
//! operating directly over ICE -> DTLS -> SRTP without SDP offer/answer abstractions.

use crate::models::errors::RustTgCallsError;
use crate::models::{ConnState, OPUS_PAYLOAD_TYPE, VP8_PAYLOAD_TYPE};
use crate::wrtc::native::certpool::GeneratedCert;
use crate::wrtc::native::keys::{
    DTLS_SRTP_EXPORTER_LABEL, ProtectionProfile, derive_srtp_keying_material_both,
};
use crate::wrtc::native::remote::parse_remote_json;
use crate::wrtc::native::signaling::build_local_params_json;
use bytes::{Bytes, BytesMut};
use rand::Rng;
use rtc::dtls::config::{ConfigBuilder as DtlsConfigBuilder, ExtendedMasterSecretType};
use rtc::dtls::endpoint::{Endpoint as DtlsEndpoint, EndpointEvent};
use rtc::ice::attributes::control::AttrControlled;
use rtc::ice::attributes::priority::PriorityAttr;
use rtc::rtcp::compound_packet::CompoundPacket;
use rtc::rtcp::sender_report::SenderReport;
use rtc::rtcp::source_description::{
    SdesType, SourceDescription, SourceDescriptionChunk, SourceDescriptionItem,
};
use rtc::rtp::codec::vp8::Vp8Payloader;
use rtc::rtp::header::Header;
use rtc::rtp::packet::Packet;
use rtc::rtp::packetizer::Payloader;
use rtc::shared::crypto::KeyingMaterialExporter;
use rtc::shared::marshal::Marshal;
use rtc::shared::TransportProtocol;
use rtc::srtp::context::Context as SrtpContext;
use rtc::srtp::protection_profile::ProtectionProfile as SrtpContextProtectionProfile;
use rtc::stun::attributes::ATTR_USERNAME;
use rtc::stun::fingerprint::FINGERPRINT;
use rtc::stun::integrity::MessageIntegrity;
use rtc::stun::message::{
    BINDING_REQUEST, BINDING_SUCCESS, Message as StunMessage, TransactionId as StunTransactionId,
};
use rtc::stun::textattrs::Username;
use rtc::stun::xoraddr::XorMappedAddress;
use parking_lot::Mutex;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU16, AtomicU32, Ordering};
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;

const BASE64URL_CHARSET: &[u8] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// DirectTransport encapsulates the direct wire-protocol engine for Telegram group calls.
pub struct DirectTransport {
    ufrag: String,
    pwd: String,
    audio_ssrc: u32,
    video_ssrc: u32,
    fingerprint_sha256: String,
    state: Arc<AtomicI32>,
    closed: Arc<AtomicBool>,
    _cert: GeneratedCert,
    udp_socket: Arc<Mutex<Option<Arc<UdpSocket>>>>,
    remote_addr: Arc<Mutex<Option<SocketAddr>>>,
    srtp_context: Arc<Mutex<Option<SrtpContext>>>,
    decrypt_srtp_context: Arc<Mutex<Option<SrtpContext>>>,
    audio_seq: AtomicU16,
    audio_ts: Arc<AtomicU32>,
    video_seq: AtomicU16,
    video_ts: Arc<AtomicU32>,
    video_payloader: Mutex<Vp8Payloader>,
    first_audio: AtomicBool,
    first_video: AtomicBool,
    audio_packets: Arc<AtomicU32>,
    audio_octets: Arc<AtomicU32>,
    video_packets: Arc<AtomicU32>,
    video_octets: Arc<AtomicU32>,
    media_epoch: Arc<Mutex<Option<(Instant, u64, u32, u32)>>>,
    last_keyframe: Arc<Mutex<Option<Bytes>>>,
    pli_requested: Arc<AtomicBool>,
    connected_notify: Arc<tokio::sync::Notify>,
}

impl DirectTransport {
    /// new instantiates a DirectTransport instance with 144-bit entropy ICE credentials and random SSRCs.
    pub async fn new(cert: GeneratedCert) -> Result<Arc<Self>, RustTgCallsError> {
        let (audio_ssrc, video_ssrc) = {
            let mut rng = rand::rng();
            (
                rng.random_range(0x1000_0000..=0x7FFF_FFFF),
                rng.random_range(0x1000_0000..=0x7FFF_FFFF),
            )
        };
        Self::new_with_ssrc(cert, audio_ssrc, video_ssrc).await
    }

    /// new_with_ssrc instantiates a DirectTransport with explicit SSRCs.
    pub async fn new_with_ssrc(
        cert: GeneratedCert,
        audio_ssrc: u32,
        video_ssrc: u32,
    ) -> Result<Arc<Self>, RustTgCallsError> {
        let (ufrag, pwd, audio_seq, video_seq) = {
            let mut rng = rand::rng();
            let ufrag: String = (0..16)
                .map(|_| {
                    let idx = rng.random_range(0..BASE64URL_CHARSET.len());
                    BASE64URL_CHARSET[idx] as char
                })
                .collect();
            let pwd: String = (0..32)
                .map(|_| {
                    let idx = rng.random_range(0..BASE64URL_CHARSET.len());
                    BASE64URL_CHARSET[idx] as char
                })
                .collect();
            (
                ufrag,
                pwd,
                rng.random_range(1000..5000),
                rng.random_range(1000..5000),
            )
        };

        let fingerprint_sha256 = cert.fingerprint_sha256.clone();
        tracing::debug!(
            "[DirectTransport] Local ICE credentials: ufrag={}, pwd={}, fp={}",
            ufrag, pwd, fingerprint_sha256
        );

        let transport = Arc::new(Self {
            ufrag,
            pwd,
            audio_ssrc,
            video_ssrc,
            fingerprint_sha256,
            state: Arc::new(AtomicI32::new(ConnState::Connecting as i32)),
            closed: Arc::new(AtomicBool::new(false)),
            _cert: cert,
            udp_socket: Arc::new(Mutex::new(None)),
            remote_addr: Arc::new(Mutex::new(None)),
            srtp_context: Arc::new(Mutex::new(None)),
            decrypt_srtp_context: Arc::new(Mutex::new(None)),
            audio_seq: AtomicU16::new(audio_seq),
            audio_ts: Arc::new(AtomicU32::new(0)),
            video_seq: AtomicU16::new(video_seq),
            video_ts: Arc::new(AtomicU32::new(0)),
            video_payloader: Mutex::new({
                let mut p = Vp8Payloader::default();
                p.enable_picture_id = true;
                p
            }),
            first_audio: AtomicBool::new(true),
            first_video: AtomicBool::new(true),
            audio_packets: Arc::new(AtomicU32::new(0)),
            audio_octets: Arc::new(AtomicU32::new(0)),
            video_packets: Arc::new(AtomicU32::new(0)),
            video_octets: Arc::new(AtomicU32::new(0)),
            media_epoch: Arc::new(Mutex::new(None)),
            last_keyframe: Arc::new(Mutex::new(None)),
            pli_requested: Arc::new(AtomicBool::new(false)),
            connected_notify: Arc::new(tokio::sync::Notify::new()),
        });

        Ok(transport)
    }

    /// state returns the high-level connection state.
    pub fn state(&self) -> ConnState {
        match self.state.load(Ordering::SeqCst) {
            0 => ConnState::Connecting,
            1 => ConnState::Connected,
            2 => ConnState::Disconnected,
            3 => ConnState::Failed,
            4 => ConnState::Closed,
            _ => ConnState::Failed,
        }
    }

    /// ufrag returns the ICE username fragment.
    pub fn ufrag(&self) -> &str {
        &self.ufrag
    }

    /// pwd returns the ICE password.
    pub fn pwd(&self) -> &str {
        &self.pwd
    }

    /// audio_ssrc returns the audio SSRC.
    pub fn audio_ssrc(&self) -> u32 {
        self.audio_ssrc
    }

    /// video_ssrc returns the video SSRC.
    pub fn video_ssrc(&self) -> u32 {
        self.video_ssrc
    }

    /// build_local_params generates Telegram signaling JSON directly without SDP detour.
    pub fn build_local_params(&self) -> Result<String, RustTgCallsError> {
        build_local_params_json(
            &self.ufrag,
            &self.pwd,
            &self.fingerprint_sha256,
            self.audio_ssrc,
            self.video_ssrc,
        )
    }

    /// connect consumes remote Telegram signaling JSON directly and establishes DTLS-SRTP transport.
    pub async fn connect(&self, remote_json: &str) -> Result<(), RustTgCallsError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(RustTgCallsError::Internal("transport is closed".into()));
        }

        self.state
            .store(ConnState::Connecting as i32, Ordering::SeqCst);

        let remote_params = parse_remote_json(remote_json)?;
        let first_fp = remote_params.fingerprints.first().ok_or_else(|| {
            RustTgCallsError::InvalidParams("remote transport missing DTLS fingerprint".into())
        })?;

        if first_fp.fingerprint.is_empty() {
            return Err(RustTgCallsError::InvalidParams(
                "empty remote DTLS fingerprint".into(),
            ));
        }

        // Locate remote host UDP candidate
        let mut target_candidate = None;
        for c in &remote_params.candidates {
            if c.protocol.eq_ignore_ascii_case("udp") {
                if let Ok(ip) = c.ip.parse::<std::net::IpAddr>() {
                    let port = c.port.parse::<u16>().unwrap_or(0);
                    if port > 0 {
                        // Prefer IPv4 candidate
                        if ip.is_ipv4() {
                            target_candidate = Some(SocketAddr::new(ip, port));
                            break;
                        }
                        if target_candidate.is_none() {
                            target_candidate = Some(SocketAddr::new(ip, port));
                        }
                    }
                }
            }
        }

        let remote_socket_addr = target_candidate.ok_or_else(|| {
            RustTgCallsError::InvalidParams("no valid remote UDP candidate found".into())
        })?;

        tracing::debug!(
            "[DirectTransport] Selected remote Telegram SFU candidate: {}",
            remote_socket_addr
        );

        let socket = match remote_socket_addr {
            SocketAddr::V4(_) => UdpSocket::bind("0.0.0.0:0").await,
            SocketAddr::V6(_) => UdpSocket::bind("[::]:0").await,
        }
        .map_err(|e| RustTgCallsError::Internal(format!("failed to bind UDP socket: {:?}", e)))?;

        let local_addr = socket
            .local_addr()
            .map_err(|e| RustTgCallsError::Internal(format!("failed to get local addr: {:?}", e)))?;

        let socket_arc = Arc::new(socket);
        *self.udp_socket.lock() = Some(socket_arc.clone());
        *self.remote_addr.lock() = Some(remote_socket_addr);

        let handshake_config = DtlsConfigBuilder::default()
            .with_certificates(vec![self._cert.dtls_certificate.clone()])
            .with_extended_master_secret(ExtendedMasterSecretType::Require)
            .with_srtp_protection_profiles(vec![
                rtc::dtls::extension::extension_use_srtp::SrtpProtectionProfile::Srtp_Aead_Aes_128_Gcm,
                rtc::dtls::extension::extension_use_srtp::SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_80,
            ])
            .with_insecure_skip_verify(true)
            .build(false, Some(remote_socket_addr))
            .map_err(|e| {
                RustTgCallsError::Internal(format!("failed to build DTLS config: {:?}", e))
            })?;

        let dtls_endpoint = DtlsEndpoint::new(
            local_addr,
            TransportProtocol::UDP,
            Some(Arc::new(handshake_config)),
        );
        let dtls_endpoint_mutex = Arc::new(Mutex::new(dtls_endpoint));

        // Immediate STUN check without unnecessary sleep delay

        tracing::debug!(
            "[DirectTransport] Sending full ICE STUN binding check to {}...",
            remote_socket_addr
        );

        // Build valid ICE STUN Binding Request with USERNAME, PRIORITY, ICE-CONTROLLED, MESSAGE-INTEGRITY, FINGERPRINT
        let mut stun_req = StunMessage::new();
        let username_str = format!("{}:{}", remote_params.ufrag, self.ufrag);
        let priority = PriorityAttr(2130706431);
        let tie_breaker: u64 = rand::rng().random();
        let controlled = AttrControlled(tie_breaker);
        let integrity = MessageIntegrity(remote_params.pwd.as_bytes().to_vec());
        let tx_id = StunTransactionId::new();

        stun_req
            .build(&[
                Box::new(BINDING_REQUEST),
                Box::new(tx_id),
                Box::new(Username::new(ATTR_USERNAME, username_str)),
                Box::new(priority),
                Box::new(controlled),
                Box::new(integrity),
                Box::new(FINGERPRINT),
            ])
            .map_err(|e| {
                RustTgCallsError::Internal(format!("failed to build STUN binding req: {:?}", e))
            })?;

        let _ = socket_arc.send_to(&stun_req.raw, remote_socket_addr).await;

        // Spawn async event loop worker
        let worker_socket = socket_arc.clone();
        let worker_closed = self.closed.clone();
        let worker_state = self.state.clone();
        let worker_local_pwd = self.pwd.clone();
        let worker_dtls = dtls_endpoint_mutex.clone();
        let worker_srtp_holder = self.srtp_context.clone();
        let worker_dec_srtp_holder = self.decrypt_srtp_context.clone();
        let worker_epoch_holder = self.media_epoch.clone();
        let worker_pli_requested = self.pli_requested.clone();
        let worker_notify = self.connected_notify.clone();
        let worker_audio_ssrc = self.audio_ssrc;
        let worker_video_ssrc = self.video_ssrc;
        let worker_audio_ts = self.audio_ts.clone();
        let worker_video_ts = self.video_ts.clone();
        let worker_audio_pkts = self.audio_packets.clone();
        let worker_audio_octs = self.audio_octets.clone();
        let worker_video_pkts = self.video_packets.clone();
        let worker_video_octs = self.video_octets.clone();

        tokio::spawn(async move {
            let mut buf = vec![0u8; 4096];
            let mut stun_timer = tokio::time::interval(Duration::from_millis(1500));
            let mut rtcp_timer = tokio::time::interval(Duration::from_millis(500));
            let mut dtls_timeout_timer = tokio::time::interval(Duration::from_millis(50));

            while !worker_closed.load(Ordering::SeqCst) {
                tokio::select! {
                    _ = dtls_timeout_timer.tick() => {
                        if worker_state.load(Ordering::SeqCst) != (ConnState::Connected as i32) {
                            let transmits = {
                                let mut dtls_guard = worker_dtls.lock();
                                let _ = dtls_guard.handle_timeout(remote_socket_addr, Instant::now());
                                let mut txs = Vec::new();
                                while let Some(tx) = dtls_guard.poll_transmit() {
                                    txs.push((tx.message.freeze(), tx.transport.peer_addr));
                                }
                                txs
                            };
                            for (msg, target) in transmits {
                                let _ = worker_socket.send_to(&msg, target).await;
                                tracing::trace!("[DirectTransport] Retransmitted DTLS packet to {} (len={})", target, msg.len());
                            }
                        }
                    }
                    _ = stun_timer.tick() => {
                        let _ = worker_socket.send_to(&stun_req.raw, remote_socket_addr).await;
                    }
                    _ = rtcp_timer.tick() => {
                        // RFC 3550 RTCP Compound Sender Report Emission for instantaneous A/V Lip-Sync lock
                        let epoch_opt = *worker_epoch_holder.lock();
                        if epoch_opt.is_some() {
                            let ntp_time = get_current_ntp_time();
                            let cur_a_ts = worker_audio_ts.load(Ordering::Relaxed);
                            let cur_v_ts = worker_video_ts.load(Ordering::Relaxed);

                            let audio_sr = build_compound_sender_report(
                                worker_audio_ssrc,
                                ntp_time,
                                cur_a_ts,
                                worker_audio_pkts.load(Ordering::Relaxed),
                                worker_audio_octs.load(Ordering::Relaxed),
                            );
                            let video_sr = build_compound_sender_report(
                                worker_video_ssrc,
                                ntp_time,
                                cur_v_ts,
                                worker_video_pkts.load(Ordering::Relaxed),
                                worker_video_octs.load(Ordering::Relaxed),
                            );
                            let (enc_a, enc_v) = {
                                let mut srtp_guard = worker_srtp_holder.lock();
                                if let Some(ref mut srtp_ctx) = *srtp_guard {
                                    (
                                        audio_sr.as_deref().and_then(|b| srtp_ctx.encrypt_rtcp(b).ok()),
                                        video_sr.as_deref().and_then(|b| srtp_ctx.encrypt_rtcp(b).ok()),
                                    )
                                } else {
                                    (None, None)
                                }
                            };
                            if let Some(enc) = enc_a {
                                let _ = worker_socket.send_to(&enc, remote_socket_addr).await;
                            }
                            if let Some(enc) = enc_v {
                                let _ = worker_socket.send_to(&enc, remote_socket_addr).await;
                            }
                        }
                    }
                    recv_res = worker_socket.recv_from(&mut buf) => {
                        let (n, peer_addr) = match recv_res {
                            Ok((n, addr)) if n > 0 => (n, addr),
                            _ => continue,
                        };
                        let data = &buf[..n];

                        // 1. STUN Binding Request (magic cookie 0x2112A442)
                        if n >= 20 && data[4..8] == [0x21, 0x12, 0xa4, 0x42] {
                            if let Some(resp_bytes) = handle_stun_packet(data, peer_addr, &worker_local_pwd) {
                                let _ = worker_socket.send_to(&resp_bytes, peer_addr).await;
                            }
                            continue;
                        }

                        // 2. DTLS Handshake / Datagrams (types 20..=23)
                        if data[0] >= 20 && data[0] <= 23 {
                            let (transmits, _just_connected) = {
                                let mut dtls_guard = worker_dtls.lock();
                                handle_dtls_packet(
                                    data,
                                    peer_addr,
                                    &mut dtls_guard,
                                    &worker_srtp_holder,
                                    &worker_dec_srtp_holder,
                                    &worker_state,
                                    &worker_notify,
                                )
                            };

                            for (msg, target) in transmits {
                                let _ = worker_socket.send_to(&msg, target).await;
                                tracing::trace!("[DirectTransport] Sent DTLS reply packet to {} (len={})", target, msg.len());
                            }
                            continue;
                        }

                        // 3. Incoming SRTP/SRTCP Packets (types >= 128)
                        if data[0] >= 128 {
                            let rtcp_decrypted = {
                                let mut dec_guard = worker_dec_srtp_holder.lock();
                                if let Some(ref mut dec_ctx) = *dec_guard {
                                    dec_ctx.decrypt_rtcp(data).ok()
                                } else {
                                    None
                                }
                            };
                            if let Some(ref rtcp_bytes) = rtcp_decrypted {
                                let mut slice = &rtcp_bytes[..];
                                if let Ok(pkts) = rtc::rtcp::packet::unmarshal(&mut slice) {
                                    for pkt in pkts {
                                        let hdr = pkt.header();
                                        if hdr.packet_type == rtc::rtcp::header::PacketType::PayloadSpecificFeedback {
                                            tracing::debug!("[DirectTransport] Received PLI/FIR feedback from Telegram SFU!");
                                            worker_pli_requested.store(true, Ordering::SeqCst);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        // Instant non-polling wait for handshake completion
        if self.state.load(Ordering::SeqCst) == (ConnState::Connected as i32) {
            return Ok(());
        }

        let notify = self.connected_notify.clone();
        tokio::select! {
            _ = notify.notified() => Ok(()),
            _ = async {
                while self.state.load(Ordering::SeqCst) != (ConnState::Connected as i32) {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            } => Ok(()),
            _ = tokio::time::sleep(Duration::from_secs(10)) => {
                if self.state.load(Ordering::SeqCst) == (ConnState::Connected as i32) {
                    Ok(())
                } else {
                    Err(RustTgCallsError::Internal(
                        "timeout waiting for DTLS handshake to complete with Telegram SFU".into(),
                    ))
                }
            }
        }
    }

    /// emit_rtcp_sender_reports generates and sends compound RTCP SR + SDES packets for A/V Lip-Sync.
    pub async fn emit_rtcp_sender_reports(&self) {
        let epoch_opt = *self.media_epoch.lock();
        if epoch_opt.is_some() {
            let ntp_time = get_current_ntp_time();
            let cur_a_ts = self.audio_ts.load(Ordering::Relaxed);
            let cur_v_ts = self.video_ts.load(Ordering::Relaxed);

            let audio_sr = build_compound_sender_report(
                self.audio_ssrc,
                ntp_time,
                cur_a_ts,
                self.audio_packets.load(Ordering::Relaxed),
                self.audio_octets.load(Ordering::Relaxed),
            );
            let video_sr = build_compound_sender_report(
                self.video_ssrc,
                ntp_time,
                cur_v_ts,
                self.video_packets.load(Ordering::Relaxed),
                self.video_octets.load(Ordering::Relaxed),
            );
            let (enc_a, enc_v) = {
                let mut srtp_guard = self.srtp_context.lock();
                if let Some(ref mut srtp_ctx) = *srtp_guard {
                    (
                        audio_sr.as_deref().and_then(|b| srtp_ctx.encrypt_rtcp(b).ok()),
                        video_sr.as_deref().and_then(|b| srtp_ctx.encrypt_rtcp(b).ok()),
                    )
                } else {
                    (None, None)
                }
            };
            let sock_opt = self.udp_socket.lock().clone();
            let addr_opt = *self.remote_addr.lock();
            if let (Some(sock), Some(addr)) = (sock_opt, addr_opt) {
                if let Some(enc) = enc_a {
                    let _ = sock.send_to(&enc, addr).await;
                }
                if let Some(enc) = enc_v {
                    let _ = sock.send_to(&enc, addr).await;
                }
            }
        }
    }

    /// send_rtp_bytes encrypts and transmits raw payload over the SRTP context to the ICE wire.
    /// Resets per-track streaming state when switching tracks (e.g. on skip / set_source).
    pub fn reset_media_track_state(&self) {
        self.first_audio.store(true, Ordering::SeqCst);
        self.first_video.store(true, Ordering::SeqCst);
        *self.last_keyframe.lock() = None;
        self.pli_requested.store(false, Ordering::SeqCst);
        *self.media_epoch.lock() = None;
        let mut p = Vp8Payloader::default();
        p.enable_picture_id = true;
        *self.video_payloader.lock() = p;
    }

    pub async fn send_rtp_bytes(
        &self,
        payload_type: u8,
        payload: Bytes,
        duration: Duration,
    ) -> Result<(), RustTgCallsError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(RustTgCallsError::Internal("transport is closed".into()));
        }

        let just_started = {
            let mut epoch_guard = self.media_epoch.lock();
            if epoch_guard.is_none() {
                let now_inst = Instant::now();
                let now_ntp = get_current_ntp_time();
                let a_ts = self.audio_ts.load(Ordering::Relaxed);
                let v_ts = self.video_ts.load(Ordering::Relaxed);
                *epoch_guard = Some((now_inst, now_ntp, a_ts, v_ts));
                tracing::debug!("[DirectTransport] Media epoch established at T=0 (a_ts={}, v_ts={})", a_ts, v_ts);
                true
            } else {
                false
            }
        };

        if just_started {
            self.emit_rtcp_sender_reports().await;
        }

        let is_audio = payload_type == OPUS_PAYLOAD_TYPE;
        let ntp_24 = ((get_current_ntp_time() >> 14) & 0x00FF_FFFF) as u32;
        let abs_time_bytes = [(ntp_24 >> 16) as u8, (ntp_24 >> 8) as u8, ntp_24 as u8];

        if is_audio {
            let seq = self.audio_seq.fetch_add(1, Ordering::SeqCst);
            let nanos = duration.as_nanos();
            let ts_inc = ((nanos * 48000) / 1_000_000_000) as u32;
            let ts = self.audio_ts.fetch_add(ts_inc, Ordering::SeqCst);
            let marker = self.first_audio.swap(false, Ordering::SeqCst);

            let mut header = Header {
                version: 2,
                payload_type,
                sequence_number: seq,
                timestamp: ts,
                ssrc: self.audio_ssrc,
                marker,
                extension_profile: rtc::rtp::header::EXTENSION_PROFILE_ONE_BYTE,
                ..Default::default()
            };
            let _ = header.set_extension(1, Bytes::from_static(&[0x80]));
            let _ = header.set_extension(2, Bytes::copy_from_slice(&abs_time_bytes));
            let _ = header.set_extension(4, Bytes::from_static(b"0"));

            let payload_len = payload.len() as u32;
            let packet = Packet { header, payload };
            let raw_bytes = packet
                .marshal()
                .map_err(|e| RustTgCallsError::Internal(e.to_string()))?;

            let encrypted_opt = {
                let mut srtp_guard = self.srtp_context.lock();
                if let Some(ref mut srtp_ctx) = *srtp_guard {
                    Some(
                        srtp_ctx
                            .encrypt_rtp_with_header(&raw_bytes, &packet.header)
                            .map_err(|e| RustTgCallsError::Internal(e.to_string()))?,
                    )
                } else {
                    None
                }
            };

            if let Some(encrypted) = encrypted_opt {
                let sock_opt = self.udp_socket.lock().clone();
                let addr_opt = *self.remote_addr.lock();
                if let (Some(sock), Some(addr)) = (sock_opt, addr_opt) {
                    let _ = sock.send_to(&encrypted, addr).await;
                    let pkts = self.audio_packets.fetch_add(1, Ordering::Relaxed) + 1;
                    self.audio_octets.fetch_add(payload_len, Ordering::Relaxed);
                    if pkts % 250 == 0 {
                        tracing::trace!("[DirectTransport Audio] Streamed {} audio packets successfully (seq={}, ts={})", pkts, seq, ts);
                    }
                }
            }
        } else {
            // VP8 Video Packetization using verified rtc Vp8Payloader
            let nanos = duration.as_nanos();
            let ts_inc = ((nanos * 90000 + 500_000_000) / 1_000_000_000) as u32;
            let ts = self.video_ts.fetch_add(ts_inc, Ordering::SeqCst);
            let _is_first_frame = self.first_video.swap(false, Ordering::SeqCst);

            const MAX_VP8_PAYLOAD: usize = 1180;
            let chunks = {
                let mut payloader = self.video_payloader.lock();
                payloader
                    .payload(MAX_VP8_PAYLOAD, &payload)
                    .map_err(|e| RustTgCallsError::Internal(e.to_string()))?
            };

            if chunks.is_empty() {
                return Ok(());
            }

            let sock_opt = self.udp_socket.lock().clone();
            let addr_opt = *self.remote_addr.lock();
            let (Some(sock), Some(addr)) = (sock_opt, addr_opt) else {
                return Ok(());
            };

            let num_chunks = chunks.len();
            let mut encrypted_packets = Vec::with_capacity(num_chunks);
            {
                let mut srtp_guard = self.srtp_context.lock();
                let Some(ref mut srtp_ctx) = *srtp_guard else {
                    return Ok(());
                };

                for (idx, chunk) in chunks.into_iter().enumerate() {
                    let is_last = idx == num_chunks - 1;
                    let seq = self.video_seq.fetch_add(1, Ordering::SeqCst);

                    let mut header = Header {
                        version: 2,
                        payload_type: VP8_PAYLOAD_TYPE,
                        sequence_number: seq,
                        timestamp: ts,
                        ssrc: self.video_ssrc,
                        marker: is_last,
                        extension_profile: rtc::rtp::header::EXTENSION_PROFILE_ONE_BYTE,
                        ..Default::default()
                    };
                    let _ = header.set_extension(2, Bytes::copy_from_slice(&abs_time_bytes));
                    let _ = header.set_extension(4, Bytes::from_static(b"1"));
                    let _ = header.set_extension(5, Bytes::from_static(&[0x00]));

                    let chunk_len = chunk.len();
                    let packet = Packet {
                        header,
                        payload: chunk,
                    };

                    let raw_bytes = packet
                        .marshal()
                        .map_err(|e| RustTgCallsError::Internal(e.to_string()))?;

                    let encrypted = srtp_ctx
                        .encrypt_rtp_with_header(&raw_bytes, &packet.header)
                        .map_err(|e| RustTgCallsError::Internal(e.to_string()))?;

                    encrypted_packets.push((encrypted, chunk_len));
                }
            }

            for (encrypted, chunk_len) in encrypted_packets {
                let _ = sock.send_to(&encrypted, addr).await;
                self.video_packets.fetch_add(1, Ordering::Relaxed);
                self.video_octets.fetch_add(chunk_len as u32, Ordering::Relaxed);
            }
        }

        Ok(())
    }

    /// is_closed returns whether transport is closed.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// close terminates transport handles idempotently.
    pub async fn close(&self) -> Result<(), RustTgCallsError> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }

        self.state.store(ConnState::Closed as i32, Ordering::SeqCst);
        *self.srtp_context.lock() = None;
        *self.decrypt_srtp_context.lock() = None;
        *self.media_epoch.lock() = None;
        *self.last_keyframe.lock() = None;
        self.pli_requested.store(false, Ordering::SeqCst);
        *self.udp_socket.lock() = None;

        Ok(())
    }
}

/// Helper to extract and derive SRTP cryptographic contexts for both sending and receiving upon DTLS handshake completion.
fn derive_srtp_from_dtls(
    dtls_endpoint: &DtlsEndpoint,
    peer_addr: SocketAddr,
) -> Option<(SrtpContext, SrtpContext, SrtpContextProtectionProfile)> {
    let dtls_state = dtls_endpoint.get_connection_state(peer_addr)?;
    let (keys_prof, srtp_prof) = match dtls_state.srtp_protection_profile() {
        rtc::dtls::extension::extension_use_srtp::SrtpProtectionProfile::Srtp_Aead_Aes_128_Gcm => {
            (ProtectionProfile::AeadAes128Gcm, SrtpContextProtectionProfile::AeadAes128Gcm)
        }
        _ => {
            (ProtectionProfile::Aes128CmHmacSha1_80, SrtpContextProtectionProfile::Aes128CmHmacSha1_80)
        }
    };

    let keying_material = dtls_state
        .export_keying_material(DTLS_SRTP_EXPORTER_LABEL, &[], 60)
        .ok()?;
    let (server_key, server_salt, client_key, client_salt) =
        derive_srtp_keying_material_both(keying_material.as_ref(), keys_prof).ok()?;
    let encrypt_ctx =
        SrtpContext::new(&server_key, &server_salt, srtp_prof, None, None).ok()?;
    let decrypt_ctx =
        SrtpContext::new(&client_key, &client_salt, srtp_prof, None, None).ok()?;

    Some((encrypt_ctx, decrypt_ctx, srtp_prof))
}

/// Helper to decode and respond to inbound STUN Binding Requests.
fn handle_stun_packet(
    data: &[u8],
    peer_addr: SocketAddr,
    local_pwd: &str,
) -> Option<Vec<u8>> {
    let mut msg = StunMessage::new();
    msg.raw = data.to_vec();
    msg.decode().ok()?;

    if msg.typ != BINDING_REQUEST {
        return None;
    }

    let mut resp = StunMessage::new();
    let local_integrity = MessageIntegrity(local_pwd.as_bytes().to_vec());
    let xor_addr = XorMappedAddress {
        ip: peer_addr.ip(),
        port: peer_addr.port(),
    };

    resp.build(&[
        Box::new(BINDING_SUCCESS),
        Box::new(msg.transaction_id),
        Box::new(xor_addr),
        Box::new(local_integrity),
        Box::new(FINGERPRINT),
    ])
    .ok()?;

    Some(resp.raw)
}

/// Helper to feed inbound DTLS bytes into the state machine and collect outbound packets.
fn handle_dtls_packet(
    data: &[u8],
    peer_addr: SocketAddr,
    dtls_guard: &mut DtlsEndpoint,
    srtp_holder: &Arc<Mutex<Option<SrtpContext>>>,
    decrypt_srtp_holder: &Arc<Mutex<Option<SrtpContext>>>,
    state: &Arc<AtomicI32>,
    notify: &Arc<tokio::sync::Notify>,
) -> (Vec<(Bytes, SocketAddr)>, bool) {
    tracing::trace!("[DirectTransport] Received DTLS datagram from {} (len={})", peer_addr, data.len());
    let mut transmits = Vec::new();
    let mut just_connected = false;
    let bytes_mut = BytesMut::from(data);

    let events = match dtls_guard.read(Instant::now(), peer_addr, None, bytes_mut) {
        Ok(ev) => ev,
        Err(e) => {
            tracing::warn!("[DirectTransport] DTLS read warning: {:?}", e);
            return (transmits, false);
        }
    };

    while let Some(tx) = dtls_guard.poll_transmit() {
        transmits.push((tx.message.freeze(), tx.transport.peer_addr));
    }

    for event in events {
        if let EndpointEvent::HandshakeComplete = event {
            if let Some((enc_ctx, dec_ctx, srtp_prof)) = derive_srtp_from_dtls(dtls_guard, peer_addr) {
                let mut srtp_guard = srtp_holder.lock();
                let mut dec_guard = decrypt_srtp_holder.lock();
                if srtp_guard.is_none() {
                    *srtp_guard = Some(enc_ctx);
                    *dec_guard = Some(dec_ctx);
                    state.store(ConnState::Connected as i32, Ordering::SeqCst);
                    notify.notify_waiters();
                    just_connected = true;
                    tracing::debug!("[DirectTransport] DTLS handshake COMPLETED! Connected with {:?}!", srtp_prof);
                }
            }
        }
    }

    (transmits, just_connected)
}

/// Helper to compute standard 64-bit NTP timestamp from system clock.
fn get_current_ntp_time() -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let ntp_secs = (now.as_secs() + 2_208_988_800) as u32;
    let ntp_frac = ((now.subsec_nanos() as u64 * (1 << 32)) / 1_000_000_000) as u32;
    ((ntp_secs as u64) << 32) | (ntp_frac as u64)
}

/// Helper to generate standard RFC 3550 RTCP Compound Sender Report (SR + SDES CNAME) for A/V Lip-Sync.
fn build_compound_sender_report(
    ssrc: u32,
    ntp_time: u64,
    rtp_ts: u32,
    packet_count: u32,
    octet_count: u32,
) -> Option<Vec<u8>> {
    let sr = SenderReport {
        ssrc,
        ntp_time,
        rtp_time: rtp_ts,
        packet_count,
        octet_count,
        reports: vec![],
        profile_extensions: Bytes::new(),
    };

    let sdes = SourceDescription {
        chunks: vec![SourceDescriptionChunk {
            source: ssrc,
            items: vec![SourceDescriptionItem {
                sdes_type: SdesType::SdesCname,
                text: Bytes::from_static(b"rusttgcalls"),
            }],
        }],
    };

    let compound = CompoundPacket(vec![Box::new(sr), Box::new(sdes)]);
    compound.marshal().ok().map(|b| b.to_vec())
}


