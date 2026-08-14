use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Device {
    Microphone,
    Camera,
}

impl fmt::Display for Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Device::Microphone => write!(f, "microphone"),
            Device::Camera => write!(f, "camera"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StreamType {
    Audio,
    Video,
}

impl fmt::Display for StreamType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StreamType::Audio => write!(f, "audio"),
            StreamType::Video => write!(f, "video"),
        }
    }
}

pub const DEFAULT_CHANNEL_COUNT: u16 = 2;
pub const OPUS_SAMPLE_RATE: u32 = 48000;
pub const OPUS_FRAME_DURATION_MS: u32 = 20;
pub const OPUS_PAYLOAD_TYPE: u8 = 111;
pub const VP8_PAYLOAD_TYPE: u8 = 100;
