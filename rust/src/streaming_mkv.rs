//! Streaming MKV demuxer — wraps the state machine parser for WASM.

use wasm_bindgen::prelude::*;

use crate::demuxer::Sample;
use crate::matroska::streaming::StreamingMkvParser;
use crate::mkv::SubtitleEvent;

struct FrameData {
    timestamp_us: f64,
    duration_us: f64,
    data: Vec<u8>,
    is_keyframe: bool,
}

#[wasm_bindgen]
pub struct StreamingMkvDemuxer {
    parser: StreamingMkvParser,

    video_track: u64,
    audio_tracks: Vec<u64>,
    subtitle_tracks: Vec<u64>,

    video_frames: Vec<FrameData>,
    audio_frames: Vec<FrameData>,
    subtitle_events: Vec<SubtitleEvent>,

    width: u32,
    height: u32,
    codec_private: Vec<u8>,
    audio_sample_rate: u32,
    audio_channels: u16,
    audio_codec_private: Vec<u8>,
    duration_ms: f64,

    header_parsed: bool,
}

#[wasm_bindgen]
impl StreamingMkvDemuxer {
    #[wasm_bindgen(constructor)]
    pub fn new() -> StreamingMkvDemuxer {
        console_error_panic_hook::set_once();
        StreamingMkvDemuxer {
            parser: StreamingMkvParser::new(),
            video_track: 0,
            audio_tracks: Vec::new(),
            subtitle_tracks: Vec::new(),
            video_frames: Vec::new(),
            audio_frames: Vec::new(),
            subtitle_events: Vec::new(),
            width: 0,
            height: 0,
            codec_private: Vec::new(),
            audio_sample_rate: 0,
            audio_channels: 0,
            audio_codec_private: Vec::new(),
            duration_ms: 0.0,
            header_parsed: false,
        }
    }

    /// Push a chunk of data. Returns true once header is parsed.
    pub fn push_data(&mut self, data: &[u8]) -> bool {
        self.parser.push(data);
        self.parser.parse();

        // Extract header info on first availability
        if !self.header_parsed && self.parser.header_done {
            self.extract_header();
            self.header_parsed = true;
        }

        // Collect any new frames
        self.collect_frames();

        self.header_parsed
    }

    pub fn finish(&mut self) {
        self.parser.parse();
        self.collect_frames();
    }

    fn extract_header(&mut self) {
        self.duration_ms = self.parser.duration * self.parser.timecode_scale as f64 / 1_000_000.0;

        for t in &self.parser.tracks {
            match t.track_type {
                1 if self.video_track == 0 => {
                    self.video_track = t.number;
                    self.width = t.pixel_width;
                    self.height = t.pixel_height;
                    self.codec_private = t.codec_private.clone();
                }
                2 => {
                    if self.audio_tracks.is_empty() {
                        self.audio_sample_rate = t.sample_rate as u32;
                        self.audio_channels = t.channels as u16;
                        self.audio_codec_private = t.codec_private.clone();
                    }
                    self.audio_tracks.push(t.number);
                }
                17 => {
                    self.subtitle_tracks.push(t.number);
                }
                _ => {}
            }
        }
    }

    fn collect_frames(&mut self) {
        for frame in self.parser.drain_frames() {
            let ts_us = frame.timestamp_ns as f64 / 1_000.0;
            let dur_us = frame.duration_ns.map(|d| d as f64 / 1_000.0).unwrap_or(0.0);

            if frame.track == self.video_track {
                self.video_frames.push(FrameData {
                    timestamp_us: ts_us, duration_us: dur_us,
                    data: frame.data, is_keyframe: frame.is_keyframe,
                });
            } else if self.audio_tracks.contains(&frame.track) {
                self.audio_frames.push(FrameData {
                    timestamp_us: ts_us, duration_us: dur_us,
                    data: frame.data, is_keyframe: true,
                });
            } else if self.subtitle_tracks.contains(&frame.track) {
                let text = String::from_utf8_lossy(&frame.data).to_string();
                self.subtitle_events.push(SubtitleEvent::new(ts_us, dur_us, text));
            }
        }
    }

    // ── Video ──

    pub fn header_ready(&self) -> bool { self.header_parsed }
    pub fn width(&self) -> u32 { self.width }
    pub fn height(&self) -> u32 { self.height }
    pub fn sample_count(&self) -> u32 { self.video_frames.len() as u32 }
    pub fn duration_ms(&self) -> f64 { self.duration_ms }
    pub fn codec_description(&self) -> Vec<u8> { self.codec_private.clone() }

    pub fn nal_length_size(&self) -> u8 {
        if self.codec_private.len() > 21 { (self.codec_private[21] & 0x03) + 1 } else { 4 }
    }

    pub fn read_sample(&self, index: u32) -> Option<Sample> {
        let f = self.video_frames.get(index as usize)?;
        Some(Sample::new(f.is_keyframe, f.timestamp_us, f.duration_us, f.data.clone()))
    }

    pub fn find_keyframe_before(&self, target_us: f64) -> u32 {
        let mut best = 0u32;
        for (i, f) in self.video_frames.iter().enumerate() {
            if !f.is_keyframe { continue; }
            if f.timestamp_us <= target_us { best = i as u32; } else { break; }
        }
        best
    }

    // ── Audio ──

    pub fn has_audio(&self) -> bool { !self.audio_tracks.is_empty() }
    pub fn audio_sample_rate(&self) -> u32 { self.audio_sample_rate }
    pub fn audio_channel_count(&self) -> u16 { self.audio_channels }
    pub fn audio_codec_config(&self) -> Vec<u8> { self.audio_codec_private.clone() }
    pub fn audio_sample_count(&self) -> u32 { self.audio_frames.len() as u32 }

    pub fn read_audio_sample(&self, index: u32) -> Option<Sample> {
        let f = self.audio_frames.get(index as usize)?;
        Some(Sample::new(true, f.timestamp_us, f.duration_us, f.data.clone()))
    }

    pub fn find_audio_sample_at(&self, target_us: f64) -> u32 {
        let mut best = 0u32;
        for (i, f) in self.audio_frames.iter().enumerate() {
            if f.timestamp_us <= target_us { best = i as u32; } else { break; }
        }
        best
    }

    // ── Subtitles ──

    pub fn has_subtitles(&self) -> bool { !self.subtitle_tracks.is_empty() || !self.subtitle_events.is_empty() }
    pub fn subtitle_track_count(&self) -> u32 { self.subtitle_tracks.len().max(if self.subtitle_events.is_empty() { 0 } else { 1 }) as u32 }
    pub fn subtitle_count(&self) -> u32 { self.subtitle_events.len() as u32 }

    pub fn subtitle_track_info(&self, index: u32) -> Option<crate::mkv::TrackInfo> {
        let track_id = self.subtitle_tracks.get(index as usize)?;
        let t = self.parser.tracks.iter().find(|t| t.number == *track_id)?;
        Some(crate::mkv::TrackInfo::from_parts(
            &t.name, &t.language, &t.codec_id,
        ))
    }

    pub fn subtitle_event(&self, index: u32) -> Option<SubtitleEvent> {
        let e = self.subtitle_events.get(index as usize)?;
        Some(SubtitleEvent::new(e.start_us(), e.duration_us(), e.text()))
    }
}
