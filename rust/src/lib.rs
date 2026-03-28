#![allow(clippy::too_many_arguments)]

pub mod demuxer;
pub mod frame_buffer;
pub mod clock;
pub mod renderer;
#[allow(dead_code, clippy::all)]
mod matroska;
pub mod mkv;
pub mod streaming;
pub mod streaming_mkv;
pub mod subtitle_engine;
pub mod player_state;
pub mod format_detect;
pub mod media_source;
pub mod stream_loader;

pub use demuxer::*;
pub use frame_buffer::*;
pub use clock::*;
pub use renderer::*;
pub use mkv::*;
pub use streaming::*;
pub use streaming_mkv::*;
pub use subtitle_engine::*;
pub use player_state::*;
pub use format_detect::*;
pub use media_source::*;
pub use stream_loader::*;
