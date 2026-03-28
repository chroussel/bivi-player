//! Player state machine — tracks playing/paused/seeking/buffering states.

use wasm_bindgen::prelude::*;

#[wasm_bindgen]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlayerStatus {
    Idle,
    Loading,
    Buffering,
    Playing,
    Paused,
    Seeking,
    Finished,
    Error,
}

#[wasm_bindgen]
pub struct PlayerState {
    status: PlayerStatus,
    was_playing_before_seek: bool,
    seek_target_us: Option<f64>,
    next_video_sample: u32,
    next_audio_sample: u32,
    total_video_samples: u32,
    total_audio_samples: u32,
    pending_decodes: u32,
    flushed: bool,
    still_downloading: bool,
    nal_length_size: u8,
    duration_ms: f64,
}

#[wasm_bindgen]
impl PlayerState {
    #[wasm_bindgen(constructor)]
    pub fn new() -> PlayerState {
        PlayerState {
            status: PlayerStatus::Idle,
            was_playing_before_seek: false,
            seek_target_us: None,
            next_video_sample: 0,
            next_audio_sample: 0,
            total_video_samples: 0,
            total_audio_samples: 0,
            pending_decodes: 0,
            flushed: false,
            still_downloading: false,
            nal_length_size: 4,
            duration_ms: 0.0,
        }
    }

    // ── Status ──

    pub fn status(&self) -> PlayerStatus { self.status }
    pub fn is_playing(&self) -> bool { self.status == PlayerStatus::Playing }
    pub fn is_seeking(&self) -> bool { self.status == PlayerStatus::Seeking }

    pub fn set_loading(&mut self) { self.status = PlayerStatus::Loading; }
    pub fn set_playing(&mut self) { self.status = PlayerStatus::Playing; }
    pub fn set_paused(&mut self) { self.status = PlayerStatus::Paused; }
    pub fn set_finished(&mut self) { self.status = PlayerStatus::Finished; }
    pub fn set_error(&mut self) { self.status = PlayerStatus::Error; }

    // ── Media info ──

    pub fn duration_ms(&self) -> f64 { self.duration_ms }
    pub fn set_duration_ms(&mut self, ms: f64) { self.duration_ms = ms; }
    pub fn nal_length_size(&self) -> u8 { self.nal_length_size }
    pub fn set_nal_length_size(&mut self, n: u8) { self.nal_length_size = n; }

    // ── Sample tracking ──

    pub fn next_video_sample(&self) -> u32 { self.next_video_sample }
    pub fn set_next_video_sample(&mut self, n: u32) { self.next_video_sample = n; }
    pub fn advance_video_sample(&mut self) { self.next_video_sample += 1; }

    pub fn next_audio_sample(&self) -> u32 { self.next_audio_sample }
    pub fn set_next_audio_sample(&mut self, n: u32) { self.next_audio_sample = n; }
    pub fn advance_audio_sample(&mut self) { self.next_audio_sample += 1; }

    pub fn total_video_samples(&self) -> u32 { self.total_video_samples }
    pub fn set_total_video_samples(&mut self, n: u32) { self.total_video_samples = n; }
    pub fn total_audio_samples(&self) -> u32 { self.total_audio_samples }
    pub fn set_total_audio_samples(&mut self, n: u32) { self.total_audio_samples = n; }

    pub fn pending_decodes(&self) -> u32 { self.pending_decodes }
    pub fn add_pending(&mut self, n: u32) { self.pending_decodes += n; }
    pub fn sub_pending(&mut self, n: u32) { self.pending_decodes = self.pending_decodes.saturating_sub(n); }
    pub fn clear_pending(&mut self) { self.pending_decodes = 0; }

    pub fn flushed(&self) -> bool { self.flushed }
    pub fn set_flushed(&mut self, v: bool) { self.flushed = v; }

    pub fn still_downloading(&self) -> bool { self.still_downloading }
    pub fn set_still_downloading(&mut self, v: bool) { self.still_downloading = v; }

    // ── Seek ──

    pub fn begin_seek(&mut self, target_us: f64, was_playing: bool) {
        self.was_playing_before_seek = was_playing;
        self.seek_target_us = Some(target_us);
        self.status = PlayerStatus::Seeking;
        self.flushed = false;
        self.pending_decodes = 0;
    }

    pub fn seek_target_us(&self) -> f64 {
        self.seek_target_us.unwrap_or(0.0)
    }

    pub fn has_seek_target(&self) -> bool {
        self.seek_target_us.is_some()
    }

    pub fn complete_seek(&mut self) -> bool {
        let resume = self.was_playing_before_seek;
        self.seek_target_us = None;
        self.status = if resume { PlayerStatus::Playing } else { PlayerStatus::Paused };
        resume
    }

    // ── Buffer decisions ──

    /// Should the player feed more samples to the decoder?
    pub fn should_feed(&self, frame_buffer_len: u32) -> bool {
        if self.seek_target_us.is_some() { return false; }
        if self.pending_decodes > 10 { return false; }
        if frame_buffer_len > 30 { return false; }
        if self.next_video_sample >= self.total_video_samples {
            return false;
        }
        true
    }

    /// Should the player flush the decoder?
    pub fn should_flush(&self) -> bool {
        self.next_video_sample >= self.total_video_samples
            && !self.flushed
            && self.pending_decodes == 0
            && !self.still_downloading
    }

    /// Should the player fetch more data from the network?
    pub fn needs_buffer(&self) -> bool {
        if !self.still_downloading { return false; }
        let buffered = self.total_video_samples.saturating_sub(self.next_video_sample);
        buffered < 240 // ~10s at 24fps
    }

    /// Reset for new video.
    pub fn reset(&mut self) {
        self.next_video_sample = 0;
        self.next_audio_sample = 0;
        self.total_video_samples = 0;
        self.total_audio_samples = 0;
        self.pending_decodes = 0;
        self.flushed = false;
        self.seek_target_us = None;
        self.status = PlayerStatus::Idle;
    }
}
