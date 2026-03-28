//! MediaSession — owns StreamLoader + MediaSource.
//! Single Rust object that handles streaming, demuxing, and buffering.

use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use crate::demuxer::Sample;
use crate::media_source::MediaSource;
use crate::mkv::{SubtitleEvent, TrackInfo};
use crate::player_state::PlayerState;
use crate::stream_loader::StreamLoader;

#[wasm_bindgen]
pub struct MediaSession {
    loader: StreamLoader,
    source: MediaSource,
    state: PlayerState,
    last_fetched_sample: u32,
}

#[wasm_bindgen]
impl MediaSession {
    /// Create a session: probes URL, detects format, creates demuxer.
    #[wasm_bindgen(constructor)]
    pub async fn new(url: String) -> Result<MediaSession, JsValue> {
        #[cfg(target_arch = "wasm32")] console_error_panic_hook::set_once();
        let loader = StreamLoader::new(url).await?;
        let mut source = MediaSource::new();
        source.init_from_bytes(loader.init_data())?;
        let mut state = PlayerState::new();
        state.set_still_downloading(true);
        Ok(MediaSession { loader, source, state, last_fetched_sample: 0 })
    }

    /// Get parameters for the next fetch. Returns (offset, size) or None if done.
    pub fn next_fetch_range(&self) -> Option<Vec<f64>> {
        if self.loader.is_done() { return None; }
        let chunk_size = 1024 * 1024u64;
        match &self.source.inner {
            crate::media_source::Inner::Mp4(mp4) => {
                let sample_off = mp4.video_sample_offset(self.last_fetched_sample) as u64;
                let start = sample_off.saturating_sub(64 * 1024);
                let end = (start + chunk_size).min(self.loader.file_size() as u64);
                Some(vec![start as f64, end as f64])
            }
            _ => {
                let start = self.loader.current_offset();
                let end = (start + chunk_size).min(self.loader.file_size() as u64);
                Some(vec![start as f64, end as f64])
            }
        }
    }

    /// Push fetched data into the demuxer. Call after fetching the range from next_fetch_range.
    pub fn push_fetched(&mut self, data: &[u8], file_offset: f64) -> bool {
        if !data.is_empty() {
            self.last_fetched_sample = self.source.push_chunk(data, file_offset as u64, self.last_fetched_sample);
        }
        self.loader.advance_offset(data.len() as u64);
        self.state.set_total_video_samples(self.source.sample_count());
        self.state.set_total_audio_samples(self.source.audio_sample_count());
        if self.loader.is_done() {
            self.source.finish();
            self.state.set_still_downloading(false);
            return false;
        }
        true
    }

    pub fn is_done(&self) -> bool { self.loader.is_done() }

    // ── State delegation ──

    pub fn next_video_sample(&self) -> u32 { self.state.next_video_sample() }
    pub fn set_next_video_sample(&mut self, n: u32) { self.state.set_next_video_sample(n); }
    pub fn advance_video_sample(&mut self) { self.state.advance_video_sample(); }
    pub fn next_audio_sample(&self) -> u32 { self.state.next_audio_sample() }
    pub fn set_next_audio_sample(&mut self, n: u32) { self.state.set_next_audio_sample(n); }
    pub fn advance_audio_sample(&mut self) { self.state.advance_audio_sample(); }
    pub fn total_video_samples(&self) -> u32 { self.state.total_video_samples() }
    pub fn total_audio_samples(&self) -> u32 { self.state.total_audio_samples() }
    pub fn pending_decodes(&self) -> u32 { self.state.pending_decodes() }
    pub fn add_pending(&mut self, n: u32) { self.state.add_pending(n); }
    pub fn sub_pending(&mut self, n: u32) { self.state.sub_pending(n); }
    pub fn clear_pending(&mut self) { self.state.clear_pending(); }
    pub fn flushed(&self) -> bool { self.state.flushed() }
    pub fn set_flushed(&mut self, v: bool) { self.state.set_flushed(v); }
    pub fn still_downloading(&self) -> bool { self.state.still_downloading() }
    pub fn should_feed(&self, buf_len: u32) -> bool { self.state.should_feed(buf_len) }
    pub fn should_flush(&self) -> bool { self.state.should_flush() }
    /// Should the player fetch more data?
    pub fn needs_buffer(&self) -> bool {
        if !self.state.still_downloading() { return false; }
        let next = self.state.next_video_sample();
        let total = self.state.total_video_samples();
        let buffered = total.saturating_sub(next);
        // MKV: check frame count
        if buffered < 240 { return true; }
        // MP4: check if sample 10s ahead is cached
        let look = (next + 240).min(total.saturating_sub(1));
        !self.source.has_video_sample(look)
    }

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

    // ── Multi-track ──

    pub fn audio_track_count(&self) -> u32 { self.source.audio_track_count() }
    pub fn audio_track_info(&self, i: u32) -> Option<TrackInfo> { self.source.audio_track_info(i) }
    pub fn set_audio_track(&mut self, i: u32) { self.source.set_audio_track(i); }
    pub fn set_subtitle_track(&mut self, i: u32) { self.source.set_subtitle_track(i); }
}
