//! Track is the send-only RTP writer for one media kind (audio or video).

/// Kind disambiguates audio and video tracks at construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Audio,
    Video,
}

/// Track is the send-only RTP writer for one media kind. WriteSample
/// packetises a single encoded frame (Opus or VP8) into one or more RTP
/// packets, stamps required header extensions, and emits each via the
/// shared SRTP write stream.
///
/// Concurrency:
///  - Audio and video Tracks have independent state; the shared SRTP context
///    further down serializes encryption across both.
pub struct Track {
    kind: Kind,
    ssrc: u32,
    clock_rate: u32,
    pt: u8,
}

impl Track {
    /// NewTrack constructs the packetiser side of a track.
    pub fn new(kind: Kind, ssrc: u32) -> Self {
        let (pt, clock_rate) = match kind {
            Kind::Audio => (111, 48000),
            Kind::Video => (100, 90000),
        };
        Self {
            kind,
            ssrc,
            clock_rate,
            pt,
        }
    }

    /// SSRC reports the SSRC this track packetises into.
    pub fn ssrc(&self) -> u32 {
        self.ssrc
    }

    /// Kind reports whether this is an Audio or Video track.
    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// PayloadType reports the RTP payload type (111 for Opus, 100 for VP8).
    pub fn pt(&self) -> u8 {
        self.pt
    }

    /// ClockRate reports the RTP clock rate in Hz (48000 for Opus, 90000 for VP8).
    pub fn clock_rate(&self) -> u32 {
        self.clock_rate
    }
}
