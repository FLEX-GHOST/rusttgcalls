//! Package instances holds the per-chat call state. One Call trait,
//! two implementations: GroupCall (WebRTC, default) and RTMPCall (FFmpeg
//! push to a Telegram-issued RTMP URL).

pub mod call;
pub mod group_call;
pub mod rtmp_call;

pub use call::Call;
pub use group_call::GroupCall;
pub use rtmp_call::RTMPCall;
