//! Streaming demuxer — parses MP4 moov box without needing the full file.
//! Media data (mdat) is fetched on demand via JS Range requests.

use wasm_bindgen::prelude::*;

use crate::demuxer::{self, Sample};

#[wasm_bindgen]
pub struct StreamingDemuxer {
    // Parsed from moov
    video: demuxer::VideoTrack,
    v_sample_offsets: Vec<u64>,
    v_dts_values: Vec<u64>,
    v_pts_offset: f64,
    audio: Option<demuxer::AudioTrack>,
    a_sample_offsets: Vec<u64>,
    a_dts_values: Vec<u64>,
    // Buffered sample data: sparse — only samples we've fetched
    v_sample_cache: Vec<Option<Vec<u8>>>,
    a_sample_cache: Vec<Option<Vec<u8>>>,
}

#[wasm_bindgen]
impl StreamingDemuxer {
    /// Create from moov box data (not the full file).
    #[wasm_bindgen(constructor)]
    pub fn new(moov_data: Vec<u8>) -> Result<StreamingDemuxer, JsValue> {
        console_error_panic_hook::set_once();
        let tracks = demuxer::parse_mp4_moov(&moov_data)
            .map_err(|e| JsValue::from_str(&format!("moov parse error: {}", e)))?;

        let v_sample_offsets = tracks.video.build_sample_offsets();
        let v_dts_values = tracks.video.build_dts();
        let v_pts_offset = demuxer::compute_pts_offset_for(&tracks.video, &v_dts_values);

        let v_count = tracks.video.sample_count();
        let (a_sample_offsets, a_dts_values, a_count) = if let Some(ref a) = tracks.audio {
            let off = a.build_sample_offsets();
            let dts = a.build_dts();
            let c = a.sample_count();
            (off, dts, c)
        } else {
            (Vec::new(), Vec::new(), 0)
        };

        Ok(StreamingDemuxer {
            video: tracks.video,
            v_sample_offsets, v_dts_values, v_pts_offset,
            audio: tracks.audio,
            a_sample_offsets, a_dts_values,
            v_sample_cache: vec![None; v_count],
            a_sample_cache: vec![None; a_count],
        })
    }

    // ── Video ──

    pub fn width(&self) -> u32 { self.video.width as u32 }
    pub fn height(&self) -> u32 { self.video.height as u32 }
    pub fn sample_count(&self) -> u32 { self.video.sample_count() as u32 }

    pub fn duration_ms(&self) -> f64 {
        if self.video.timescale == 0 { return 0.0; }
        (self.video.duration as f64 / self.video.timescale as f64) * 1000.0
    }

    pub fn codec_description(&self) -> Vec<u8> { self.video.hvcc_raw.clone() }

    pub fn nal_length_size(&self) -> u8 {
        if self.video.hvcc_raw.len() > 21 { (self.video.hvcc_raw[21] & 0x03) + 1 } else { 4 }
    }

    /// Get byte range [offset, offset+size) for a video sample in the file.
    pub fn video_sample_offset(&self, index: u32) -> f64 {
        self.v_sample_offsets.get(index as usize).copied().unwrap_or(0) as f64
    }

    pub fn video_sample_size(&self, index: u32) -> u32 {
        self.video.sample_sizes.get(index as usize).copied().unwrap_or(0)
    }

    /// Provide fetched sample data from JS.
    pub fn set_video_sample_data(&mut self, index: u32, data: Vec<u8>) {
        if let Some(slot) = self.v_sample_cache.get_mut(index as usize) {
            *slot = Some(data);
        }
    }

    pub fn has_video_sample(&self, index: u32) -> bool {
        self.v_sample_cache.get(index as usize).is_some_and(|s| s.is_some())
    }

    pub fn read_sample(&self, index: u32) -> Option<Sample> {
        let i = index as usize;
        let data = self.v_sample_cache.get(i)?.as_ref()?.clone();
        let timescale = self.video.timescale as f64;
        let dts = self.v_dts_values[i] as f64;
        let cts_offset = if i < self.video.composition_offsets.len() {
            self.video.composition_offsets[i] as f64
        } else { 0.0 };
        let pts = dts + cts_offset - self.v_pts_offset;
        let timestamp_us = (pts / timescale) * 1_000_000.0;
        let duration = if i < self.video.sample_durations.len() {
            self.video.sample_durations[i]
        } else {
            self.video.sample_durations.last().copied().unwrap_or(1)
        };
        let duration_us = (duration as f64 / timescale) * 1_000_000.0;
        Some(Sample::new(self.video.is_sync(i), timestamp_us, duration_us, data))
    }

    pub fn find_keyframe_before(&self, target_us: f64) -> u32 {
        let mut best = 0u32;
        for i in 0..self.video.sample_count() {
            if !self.video.is_sync(i) { continue; }
            let timescale = self.video.timescale as f64;
            let dts = self.v_dts_values[i] as f64;
            let cts = if i < self.video.composition_offsets.len() {
                self.video.composition_offsets[i] as f64
            } else { 0.0 };
            let pts = dts + cts - self.v_pts_offset;
            let ts_us = (pts / timescale) * 1_000_000.0;
            if ts_us <= target_us { best = i as u32; } else { break; }
        }
        best
    }

    /// Get the byte range needed to buffer N seconds of video starting from sample `start`.
    /// Returns [file_offset_start, file_offset_end, sample_end_index].
    pub fn video_buffer_range(&self, start: u32, seconds: f64) -> Vec<f64> {
        let timescale = self.video.timescale as f64;
        let count = self.video.sample_count();
        if start as usize >= count { return vec![0.0, 0.0, start as f64]; }

        let start_dts = self.v_dts_values[start as usize] as f64;
        let limit_ticks = start_dts + seconds * timescale;

        let mut min_off = u64::MAX;
        let mut max_end = 0u64;
        let mut end_idx = start;

        for i in (start as usize)..count {
            if self.v_dts_values[i] as f64 > limit_ticks { break; }
            let off = self.v_sample_offsets[i];
            let size = self.video.sample_sizes[i] as u64;
            min_off = min_off.min(off);
            max_end = max_end.max(off + size);
            end_idx = i as u32 + 1;
        }

        vec![min_off as f64, max_end as f64, end_idx as f64]
    }

    // ── Audio ──

    pub fn has_audio(&self) -> bool { self.audio.is_some() }

    pub fn audio_sample_rate(&self) -> u32 {
        self.audio.as_ref().map_or(0, |a| a.sample_rate)
    }

    pub fn audio_channel_count(&self) -> u16 {
        self.audio.as_ref().map_or(0, |a| a.channel_count)
    }

    pub fn audio_codec_config(&self) -> Vec<u8> {
        self.audio.as_ref().map_or_else(Vec::new, |a| a.codec_config.clone())
    }

    pub fn audio_sample_count(&self) -> u32 {
        self.audio.as_ref().map_or(0, |a| a.sample_count() as u32)
    }

    pub fn audio_sample_offset(&self, index: u32) -> f64 {
        self.a_sample_offsets.get(index as usize).copied().unwrap_or(0) as f64
    }

    pub fn audio_sample_size(&self, index: u32) -> u32 {
        self.audio.as_ref()
            .and_then(|a| a.sample_sizes.get(index as usize).copied())
            .unwrap_or(0)
    }

    pub fn set_audio_sample_data(&mut self, index: u32, data: Vec<u8>) {
        if let Some(slot) = self.a_sample_cache.get_mut(index as usize) {
            *slot = Some(data);
        }
    }

    pub fn has_audio_sample(&self, index: u32) -> bool {
        self.a_sample_cache.get(index as usize).is_some_and(|s| s.is_some())
    }

    pub fn read_audio_sample(&self, index: u32) -> Option<Sample> {
        let a = self.audio.as_ref()?;
        let i = index as usize;
        let data = self.a_sample_cache.get(i)?.as_ref()?.clone();
        let timescale = a.timescale as f64;
        let dts = self.a_dts_values[i] as f64;
        let timestamp_us = (dts / timescale) * 1_000_000.0;
        let duration = if i < a.sample_durations.len() {
            a.sample_durations[i]
        } else {
            a.sample_durations.last().copied().unwrap_or(1)
        };
        let duration_us = (duration as f64 / timescale) * 1_000_000.0;
        Some(Sample::new(true, timestamp_us, duration_us, data))
    }

    pub fn find_audio_sample_at(&self, target_us: f64) -> u32 {
        let a = match &self.audio { Some(a) => a, None => return 0 };
        let timescale = a.timescale as f64;
        let mut best = 0u32;
        for i in 0..a.sample_count() {
            let dts = self.a_dts_values[i] as f64;
            let ts_us = (dts / timescale) * 1_000_000.0;
            if ts_us <= target_us { best = i as u32; } else { break; }
        }
        best
    }

    /// Evict cached sample data outside [keep_start, keep_end) to free memory.
    pub fn evict_samples(&mut self, keep_video_start: u32, keep_video_end: u32,
                         keep_audio_start: u32, keep_audio_end: u32) {
        for i in 0..self.v_sample_cache.len() {
            if (i as u32) < keep_video_start || (i as u32) >= keep_video_end {
                self.v_sample_cache[i] = None;
            }
        }
        for i in 0..self.a_sample_cache.len() {
            if (i as u32) < keep_audio_start || (i as u32) >= keep_audio_end {
                self.a_sample_cache[i] = None;
            }
        }
    }
}
