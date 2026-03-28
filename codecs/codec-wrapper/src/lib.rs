//! Shared codec wrapper — compiled with emscripten, linked with each codec library.
//! Handles YUV 10-bit→8-bit conversion, stride stripping, ring-buffered plane copies.
//! Each codec implements `codec_decode_init/push/decode/collect` and this crate
//! provides the common `codec_*` API that the generic JS worker calls.

pub mod yuv;
pub mod ring_buffer;
