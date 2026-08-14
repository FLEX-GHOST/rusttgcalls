//! frameReader is the internal interface the Streamer pulls from. It
//! parses a byte stream (ogg or ivf) and yields one Sample per call.
//! Closing it must close the underlying byte stream.

use crate::models::OPUS_FRAME_DURATION_MS;
use crate::models::errors::RustTgCallsError;
use bytes::Bytes;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt};

/// Sample represents a single audio or video payload frame with duration.
#[derive(Debug, Clone)]
pub struct Sample {
    pub data: Bytes,
    pub duration: Duration,
}

/// OpusFrameReader parses Ogg containerized Opus audio packets according to RFC 3533 & RFC 7845.
pub struct OpusFrameReader<R: AsyncRead + Unpin + Send> {
    reader: R,
    skipped_header: bool,
    skipped_tags: bool,
    queue: std::collections::VecDeque<Bytes>,
    current_packet: bytes::BytesMut,
}

/// Helper to extract exact Opus packet duration from RFC 6716 TOC byte.
pub fn parse_opus_packet_duration(pkt: &[u8]) -> Duration {
    if pkt.is_empty() {
        return Duration::from_millis(OPUS_FRAME_DURATION_MS as u64);
    }
    let toc = pkt[0];
    let config = (toc >> 3) & 0x1F;
    let base_samples: u32 = match config {
        0 | 4 | 8 => 480,          // 10ms (SILK NB/MB/WB)
        1 | 5 | 9 => 960,          // 20ms
        2 | 6 | 10 => 1920,        // 40ms
        3 | 7 | 11 => 2880,        // 60ms
        12 | 14 => 480,            // 10ms (Hybrid SWB/FB)
        13 | 15 => 960,            // 20ms
        16 | 20 | 24 | 28 => 120,  // 2.5ms (CELT NB/WB/SWB/FB)
        17 | 21 | 25 | 29 => 240,  // 5ms
        18 | 22 | 26 | 30 => 480,  // 10ms
        19 | 23 | 27 | 31 => 960,  // 20ms
        _ => 960,
    };
    let count: u32 = match toc & 0x03 {
        0 => 1,
        1 | 2 => 2,
        3 => {
            if pkt.len() >= 2 {
                let c = (pkt[1] & 0x3F) as u32;
                if c > 0 { c } else { 1 }
            } else {
                1
            }
        }
        _ => 1,
    };
    let total_samples = base_samples * count;
    let nanos = (total_samples as u64 * 1_000_000_000) / 48000;
    Duration::from_nanos(nanos)
}

impl<R: AsyncRead + Unpin + Send> OpusFrameReader<R> {
    /// NewOpusFrameReader exposes the internal opus reader for callers that
    /// have a raw ogg byte stream.
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            skipped_header: false,
            skipped_tags: false,
            queue: std::collections::VecDeque::with_capacity(8),
            current_packet: bytes::BytesMut::with_capacity(1024),
        }
    }

    /// Next reads the next Opus sample page from Ogg container.
    pub async fn next_sample(&mut self) -> Result<Sample, RustTgCallsError> {
        loop {
            if let Some(pkt) = self.queue.pop_front() {
                let dur = parse_opus_packet_duration(&pkt);
                return Ok(Sample {
                    data: pkt,
                    duration: dur,
                });
            }

            // Read OggS magic header (27 bytes header)
            let mut header = [0u8; 27];
            if self.reader.read_exact(&mut header).await.is_err() {
                return Err(RustTgCallsError::File);
            }
            if &header[0..4] != b"OggS" {
                return Err(RustTgCallsError::Internal(
                    "invalid Ogg magic header".into(),
                ));
            }

            let page_segments = header[26] as usize;
            let mut segment_table = [0u8; 255];
            let seg_slice = &mut segment_table[..page_segments];
            if self.reader.read_exact(seg_slice).await.is_err() {
                return Err(RustTgCallsError::File);
            }

            let payload_size: usize = seg_slice.iter().map(|&x| x as usize).sum();
            let mut payload = bytes::BytesMut::zeroed(payload_size);
            if self.reader.read_exact(&mut payload).await.is_err() {
                return Err(RustTgCallsError::File);
            }

            let mut offset = 0;
            for &seg_len in seg_slice.iter() {
                let len = seg_len as usize;
                let end = offset + len;
                if end <= payload.len() {
                    self.current_packet.extend_from_slice(&payload[offset..end]);
                    offset = end;
                }
                if seg_len < 255 {
                    let pkt = self.current_packet.split().freeze();
                    if !self.skipped_header && pkt.starts_with(b"OpusHead") {
                        self.skipped_header = true;
                    } else if !self.skipped_tags && pkt.starts_with(b"OpusTags") {
                        self.skipped_tags = true;
                    } else if !pkt.is_empty() {
                        self.skipped_header = true;
                        self.skipped_tags = true;
                        self.queue.push_back(pkt);
                    }
                }
            }
        }
    }
}

/// VP8FrameReader parses IVF containerized VP8 video packets.
pub struct VP8FrameReader<R: AsyncRead + Unpin + Send> {
    reader: R,
    header_read: bool,
    frame_duration: Duration,
}

impl<R: AsyncRead + Unpin + Send> VP8FrameReader<R> {
    /// NewVP8FrameReader exposes the internal VP8 IVF reader.
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            header_read: false,
            frame_duration: Duration::from_nanos(33_333_333),
        }
    }

    /// Next reads the next IVF video frame.
    pub async fn next_sample(&mut self) -> Result<Sample, RustTgCallsError> {
        if !self.header_read {
            // IVF Header is 32 bytes
            let mut ivf_header = [0u8; 32];
            if self.reader.read_exact(&mut ivf_header).await.is_err() {
                return Err(RustTgCallsError::File);
            }
            if &ivf_header[0..4] != b"DKIF" {
                return Err(RustTgCallsError::Internal(
                    "invalid IVF magic header".into(),
                ));
            }
            let timebase_den = u32::from_le_bytes(ivf_header[16..20].try_into().unwrap_or([30, 0, 0, 0]));
            let timebase_num = u32::from_le_bytes(ivf_header[20..24].try_into().unwrap_or([1, 0, 0, 0]));
            if timebase_den > 0 && timebase_num > 0 {
                let nanos = (timebase_num as u64 * 1_000_000_000) / timebase_den as u64;
                self.frame_duration = Duration::from_nanos(nanos);
            }
            self.header_read = true;
        }

        // Frame Header is 12 bytes
        let mut frame_header = [0u8; 12];
        if self.reader.read_exact(&mut frame_header).await.is_err() {
            return Err(RustTgCallsError::File);
        }

        let frame_size = u32::from_le_bytes(frame_header[0..4].try_into().unwrap()) as usize;
        let mut frame_bytes = bytes::BytesMut::zeroed(frame_size);
        if self.reader.read_exact(&mut frame_bytes).await.is_err() {
            return Err(RustTgCallsError::File);
        }

        let is_key = if !frame_bytes.is_empty() {
            (frame_bytes[0] & 0x01) == 0
        } else {
            false
        };

        if is_key {
            tracing::trace!("[VP8FrameReader] Read KEYFRAME (size={} bytes)", frame_size);
        }

        Ok(Sample {
            data: frame_bytes.freeze(),
            duration: self.frame_duration,
        })
    }
}
