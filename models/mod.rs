//! Package models defines the public data structures, enums, error types,
//! network state definitions, and media payload constants.

pub mod errors;
pub mod media_description;
pub mod media_state;

pub use errors::RustTgCallsError;
pub use media_description::{
    DEFAULT_CHANNEL_COUNT, Device, OPUS_FRAME_DURATION_MS, OPUS_PAYLOAD_TYPE, OPUS_SAMPLE_RATE,
    StreamType, VP8_PAYLOAD_TYPE,
};
pub use media_state::{CallInfo, ConnState, MediaState, NetworkInfo};
