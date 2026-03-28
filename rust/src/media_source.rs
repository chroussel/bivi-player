//! MediaSource — unified demuxer factory.
//! Takes initial bytes, detects format, creates the right streaming demuxer.

use wasm_bindgen::prelude::*;

use crate::demuxer::Sample;
use crate::format_detect::{detect_format, ContainerFormat};
use crate::mkv::{SubtitleEvent, TrackInfo};
use crate::streaming::StreamingDemuxer;
use crate::streaming_mkv::StreamingMkvDemuxer;

/// Probe result from first bytes of a file.
#[wasm_bindgen]
pub struct ProbeResult {
    format: ContainerFormat,
    moov_offset: Option<u64>,   // MP4: where moov starts (None if in first chunk)
    needs_moov: bool,            // MP4: true if moov not yet found
}

#[wasm_bindgen]
impl ProbeResult {
    pub fn format(&self) -> ContainerFormat { self.format }
    pub fn is_mkv(&self) -> bool { self.format == ContainerFormat::Mkv }
    pub fn is_mp4(&self) -> bool { self.format == ContainerFormat::Mp4 }
    pub fn needs_moov(&self) -> bool { self.needs_moov }
    pub fn moov_offset(&self) -> f64 { self.moov_offset.unwrap_or(0) as f64 }
}

/// Probe first bytes to detect format and find key structures.
#[wasm_bindgen]
pub fn probe(data: &[u8]) -> ProbeResult {
    let format = detect_format(data);
    match format {
        ContainerFormat::Mkv => ProbeResult {
            format, moov_offset: None, needs_moov: false,
        },
        ContainerFormat::Mp4 => {
            // Scan for moov box
            let mut pos = 0usize;
            let mut moov_offset = None;
            while pos + 8 <= data.len() {
                let size = u32::from_be_bytes([data[pos], data[pos+1], data[pos+2], data[pos+3]]) as u64;
                let box_type = &data[pos+4..pos+8];
                if box_type == b"moov" {
                    moov_offset = Some(pos as u64);
                    break;
                }
                if size < 8 { break; }
                pos += size as usize;
            }
            // If moov not found in data, compute offset from box sizes
            let needs_moov = moov_offset.is_none();
            let moov_off = if needs_moov { Some(pos as u64) } else { moov_offset };
            ProbeResult { format, moov_offset: moov_off, needs_moov }
        }
        _ => ProbeResult { format, moov_offset: None, needs_moov: false },
    }
}

pub(crate) enum Inner {
    Mp4(StreamingDemuxer),
    Mkv(StreamingMkvDemuxer),
    None,
}

#[wasm_bindgen]
pub struct MediaSource {
    pub(crate) inner: Inner,
}

#[wasm_bindgen]
impl MediaSource {
    #[wasm_bindgen(constructor)]
    pub fn new() -> MediaSource {
        console_error_panic_hook::set_once();
        MediaSource { inner: Inner::None }
    }

    /// Initialize from moov box data (MP4 streaming).
    pub fn init_mp4(&mut self, moov_data: Vec<u8>) -> Result<(), JsValue> {
        let demuxer = StreamingDemuxer::new(moov_data)?;
        self.inner = Inner::Mp4(demuxer);
        Ok(())
    }

    /// Initialize for MKV streaming (push data incrementally).
    pub fn init_mkv(&mut self) {
        self.inner = Inner::Mkv(StreamingMkvDemuxer::new());
    }

    /// Auto-detect format from first bytes and initialize.
    /// For MP4: `data` should be the moov box content.
    /// For MKV: creates streaming parser, push more data via push_data().
    pub fn init_from_bytes(&mut self, data: Vec<u8>) -> Result<bool, JsValue> {
        let fmt = detect_format(&data);
        match fmt {
            ContainerFormat::Mkv => {
                let mut mkv = StreamingMkvDemuxer::new();
                mkv.push_data(&data);
                self.inner = Inner::Mkv(mkv);
                Ok(true) // is MKV
            }
            _ => {
                // Assume MP4 — data is moov box
                let demuxer = StreamingDemuxer::new(data)?;
                self.inner = Inner::Mp4(demuxer);
                Ok(false) // not MKV
            }
        }
    }

    pub fn is_mkv(&self) -> bool {
        matches!(self.inner, Inner::Mkv(_))
    }

    // ── Push data (MKV streaming) ──

    pub fn push_data(&mut self, data: &[u8]) -> bool {
        match &mut self.inner {
            Inner::Mkv(mkv) => mkv.push_data(data),
            _ => false,
        }
    }

    pub fn header_ready(&self) -> bool {
        match &self.inner {
            Inner::Mkv(mkv) => mkv.header_ready(),
            Inner::Mp4(_) => true,
            Inner::None => false,
        }
    }

    pub fn finish(&mut self) {
        if let Inner::Mkv(mkv) = &mut self.inner {
            mkv.finish();
        }
    }

    // ── Video ──

    pub fn width(&self) -> u32 {
        match &self.inner { Inner::Mp4(d) => d.width(), Inner::Mkv(d) => d.width(), _ => 0 }
    }
    pub fn height(&self) -> u32 {
        match &self.inner { Inner::Mp4(d) => d.height(), Inner::Mkv(d) => d.height(), _ => 0 }
    }
    pub fn sample_count(&self) -> u32 {
        match &self.inner { Inner::Mp4(d) => d.sample_count(), Inner::Mkv(d) => d.sample_count(), _ => 0 }
    }
    pub fn duration_ms(&self) -> f64 {
        match &self.inner { Inner::Mp4(d) => d.duration_ms(), Inner::Mkv(d) => d.duration_ms(), _ => 0.0 }
    }
    pub fn codec_description(&self) -> Vec<u8> {
        match &self.inner { Inner::Mp4(d) => d.codec_description(), Inner::Mkv(d) => d.codec_description(), _ => Vec::new() }
    }
    pub fn nal_length_size(&self) -> u8 {
        match &self.inner { Inner::Mp4(d) => d.nal_length_size(), Inner::Mkv(d) => d.nal_length_size(), _ => 4 }
    }
    pub fn read_sample(&self, index: u32) -> Option<Sample> {
        match &self.inner { Inner::Mp4(d) => d.read_sample(index), Inner::Mkv(d) => d.read_sample(index), _ => None }
    }
    pub fn find_keyframe_before(&self, target_us: f64) -> u32 {
        match &self.inner { Inner::Mp4(d) => d.find_keyframe_before(target_us), Inner::Mkv(d) => d.find_keyframe_before(target_us), _ => 0 }
    }

    // ── Audio ──

    pub fn has_audio(&self) -> bool {
        match &self.inner { Inner::Mp4(d) => d.has_audio(), Inner::Mkv(d) => d.has_audio(), _ => false }
    }
    pub fn audio_sample_rate(&self) -> u32 {
        match &self.inner { Inner::Mp4(d) => d.audio_sample_rate(), Inner::Mkv(d) => d.audio_sample_rate(), _ => 0 }
    }
    pub fn audio_channel_count(&self) -> u16 {
        match &self.inner { Inner::Mp4(d) => d.audio_channel_count(), Inner::Mkv(d) => d.audio_channel_count(), _ => 0 }
    }
    pub fn audio_codec_config(&self) -> Vec<u8> {
        match &self.inner { Inner::Mp4(d) => d.audio_codec_config(), Inner::Mkv(d) => d.audio_codec_config(), _ => Vec::new() }
    }
    pub fn audio_sample_count(&self) -> u32 {
        match &self.inner { Inner::Mp4(d) => d.audio_sample_count(), Inner::Mkv(d) => d.audio_sample_count(), _ => 0 }
    }
    pub fn read_audio_sample(&self, index: u32) -> Option<Sample> {
        match &self.inner { Inner::Mp4(d) => d.read_audio_sample(index), Inner::Mkv(d) => d.read_audio_sample(index), _ => None }
    }
    pub fn find_audio_sample_at(&self, target_us: f64) -> u32 {
        match &self.inner { Inner::Mp4(d) => d.find_audio_sample_at(target_us), Inner::Mkv(d) => d.find_audio_sample_at(target_us), _ => 0 }
    }

    // ── Subtitles ──

    pub fn has_subtitles(&self) -> bool {
        match &self.inner { Inner::Mkv(d) => d.has_subtitles(), _ => false }
    }
    pub fn subtitle_count(&self) -> u32 {
        match &self.inner { Inner::Mkv(d) => d.subtitle_count(), _ => 0 }
    }
    pub fn subtitle_event(&self, index: u32) -> Option<SubtitleEvent> {
        match &self.inner { Inner::Mkv(d) => d.subtitle_event(index), _ => None }
    }
    pub fn subtitle_track_count(&self) -> u32 {
        match &self.inner { Inner::Mkv(d) => d.subtitle_track_count(), _ => 0 }
    }
    pub fn subtitle_track_info(&self, index: u32) -> Option<TrackInfo> {
        match &self.inner { Inner::Mkv(d) => d.subtitle_track_info(index), _ => None }
    }

    // ── MP4 streaming (sample cache) ──

    // ── Multi-track ──

    pub fn audio_track_count(&self) -> u32 {
        match &self.inner { Inner::Mkv(d) => d.audio_track_count(), _ => if self.has_audio() { 1 } else { 0 } }
    }
    pub fn audio_track_info(&self, i: u32) -> Option<TrackInfo> {
        match &self.inner { Inner::Mkv(d) => d.audio_track_info(i), _ => None }
    }
    pub fn set_audio_track(&mut self, i: u32) {
        if let Inner::Mkv(d) = &mut self.inner { d.set_audio_track(i); }
    }
    pub fn set_subtitle_track(&mut self, i: u32) {
        if let Inner::Mkv(d) = &mut self.inner { d.set_subtitle_track(i); }
    }

    pub fn has_video_sample(&self, i: u32) -> bool {
        match &self.inner { Inner::Mp4(d) => d.has_video_sample(i), _ => true }
    }
    pub fn has_audio_sample(&self, i: u32) -> bool {
        match &self.inner { Inner::Mp4(d) => d.has_audio_sample(i), _ => true }
    }
    pub fn video_sample_offset(&self, i: u32) -> f64 {
        match &self.inner { Inner::Mp4(d) => d.video_sample_offset(i), _ => 0.0 }
    }
    pub fn video_sample_size(&self, i: u32) -> u32 {
        match &self.inner { Inner::Mp4(d) => d.video_sample_size(i), _ => 0 }
    }
    pub fn audio_sample_offset(&self, i: u32) -> f64 {
        match &self.inner { Inner::Mp4(d) => d.audio_sample_offset(i), _ => 0.0 }
    }
    pub fn audio_sample_size(&self, i: u32) -> u32 {
        match &self.inner { Inner::Mp4(d) => d.audio_sample_size(i), _ => 0 }
    }
    pub fn set_video_sample_data(&mut self, i: u32, data: Vec<u8>) {
        if let Inner::Mp4(d) = &mut self.inner { d.set_video_sample_data(i, data); }
    }
    pub fn set_audio_sample_data(&mut self, i: u32, data: Vec<u8>) {
        if let Inner::Mp4(d) = &mut self.inner { d.set_audio_sample_data(i, data); }
    }
    pub fn video_buffer_range(&self, start: u32, seconds: f64) -> Vec<f64> {
        match &self.inner { Inner::Mp4(d) => d.video_buffer_range(start, seconds), _ => vec![0.0, 0.0, start as f64] }
    }
    pub fn evict_samples(&mut self, vs: u32, ve: u32, as_: u32, ae: u32) {
        if let Inner::Mp4(d) = &mut self.inner { d.evict_samples(vs, ve, as_, ae); }
    }

    /// Push a fetched data chunk — handles both MKV (progressive parse) and
    /// MP4 (distribute bytes to sample cache based on offsets).
    /// `from_sample` is used for MP4 to know where to start distributing.
    /// Returns the next sample index to fetch from.
    /// Push a fetched data chunk.
    /// `file_offset`: the actual byte position in the file where `data` starts.
    /// `from_sample`: which video sample index to start distributing from.
    pub fn push_chunk(&mut self, data: &[u8], file_offset: u64, from_sample: u32) -> u32 {
        match &mut self.inner {
            Inner::Mkv(mkv) => {
                mkv.push_data(data);
                mkv.sample_count()
            }
            Inner::Mp4(mp4) => {
                let start_off = file_offset;
                let end_off = start_off + data.len() as u64;
                let v_count = mp4.sample_count();
                let mut last = from_sample;

                for i in from_sample..v_count {
                    let off = mp4.video_sample_offset(i) as u64;
                    let sz = mp4.video_sample_size(i) as u64;
                    if off + sz > end_off { break; }
                    if off < start_off || sz == 0 { continue; }
                    if mp4.has_video_sample(i) { continue; }
                    let lo = (off - start_off) as usize;
                    mp4.set_video_sample_data(i, data[lo..lo + sz as usize].to_vec());
                    last = i + 1;
                }

                // Audio
                if mp4.has_audio() {
                    let a_count = mp4.audio_sample_count();
                    for i in 0..a_count {
                        let off = mp4.audio_sample_offset(i) as u64;
                        if off >= end_off { break; }
                        if off < start_off { continue; }
                        let sz = mp4.audio_sample_size(i) as u64;
                        if sz == 0 || off + sz > end_off { continue; }
                        if mp4.has_audio_sample(i) { continue; }
                        let lo = (off - start_off) as usize;
                        mp4.set_audio_sample_data(i, data[lo..lo + sz as usize].to_vec());
                    }
                }

                last
            }
            Inner::None => from_sample,
        }
    }
}
