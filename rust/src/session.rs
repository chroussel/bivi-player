//! MediaSession — owns StreamLoader + MediaSource.
//! Single Rust object that handles streaming, demuxing, and buffering.

use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use crate::demuxer::Sample;
use crate::media_source::MediaSource;
use crate::mkv::{SubtitleEvent, TrackInfo};
use crate::stream_loader::StreamLoader;

#[wasm_bindgen]
pub struct MediaSession {
    loader: StreamLoader,
    source: MediaSource,
    last_fetched_sample: u32,
}

#[wasm_bindgen]
impl MediaSession {
    /// Create a session: probes URL, detects format, creates demuxer.
    #[wasm_bindgen(constructor)]
    pub async fn new(url: String) -> Result<MediaSession, JsValue> {
        console_error_panic_hook::set_once();
        let loader = StreamLoader::new(url).await?;
        let mut source = MediaSource::new();
        source.init_from_bytes(loader.init_data())?;
        Ok(MediaSession { loader, source, last_fetched_sample: 0 })
    }

    /// Fetch next 1MB chunk and push to demuxer. Returns true if more data available.
    pub async fn buffer_more(&mut self) -> Result<bool, JsValue> {
        if self.loader.is_done() { return Ok(false); }
        let chunk = self.loader.fetch_chunk().await?;
        if !chunk.is_empty() {
            self.last_fetched_sample = self.source.push_chunk(&chunk, self.last_fetched_sample);
        }
        if self.loader.is_done() {
            self.source.finish();
        }
        Ok(!self.loader.is_done())
    }

    pub fn is_done(&self) -> bool { self.loader.is_done() }

    // ── Delegate to MediaSource ──

    pub fn header_ready(&self) -> bool { self.source.header_ready() }
    pub fn width(&self) -> u32 { self.source.width() }
    pub fn height(&self) -> u32 { self.source.height() }
    pub fn sample_count(&self) -> u32 { self.source.sample_count() }
    pub fn duration_ms(&self) -> f64 { self.source.duration_ms() }
    pub fn codec_description(&self) -> Vec<u8> { self.source.codec_description() }
    pub fn nal_length_size(&self) -> u8 { self.source.nal_length_size() }
    pub fn read_sample(&self, i: u32) -> Option<Sample> { self.source.read_sample(i) }
    pub fn find_keyframe_before(&self, us: f64) -> u32 { self.source.find_keyframe_before(us) }

    pub fn has_audio(&self) -> bool { self.source.has_audio() }
    pub fn audio_sample_rate(&self) -> u32 { self.source.audio_sample_rate() }
    pub fn audio_channel_count(&self) -> u16 { self.source.audio_channel_count() }
    pub fn audio_codec_config(&self) -> Vec<u8> { self.source.audio_codec_config() }
    pub fn audio_sample_count(&self) -> u32 { self.source.audio_sample_count() }
    pub fn read_audio_sample(&self, i: u32) -> Option<Sample> { self.source.read_audio_sample(i) }
    pub fn find_audio_sample_at(&self, us: f64) -> u32 { self.source.find_audio_sample_at(us) }

    pub fn has_subtitles(&self) -> bool { self.source.has_subtitles() }
    pub fn subtitle_count(&self) -> u32 { self.source.subtitle_count() }
    pub fn subtitle_event(&self, i: u32) -> Option<SubtitleEvent> { self.source.subtitle_event(i) }
    pub fn subtitle_track_count(&self) -> u32 { self.source.subtitle_track_count() }
    pub fn subtitle_track_info(&self, i: u32) -> Option<TrackInfo> { self.source.subtitle_track_info(i) }

    pub fn has_video_sample(&self, i: u32) -> bool { self.source.has_video_sample(i) }
    pub fn finish(&mut self) { self.source.finish(); }
}
