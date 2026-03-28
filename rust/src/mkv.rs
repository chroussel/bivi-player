use crate::matroska::{Frame, MatroskaFile, TrackEntry, TrackType};
use std::io::Cursor;
use wasm_bindgen::prelude::*;

use crate::demuxer::Sample;

struct FrameData {
    timestamp_us: f64,
    duration_us: f64,
    data: Vec<u8>,
    is_keyframe: bool,
}

#[wasm_bindgen]
pub struct SubtitleEvent {
    start_us: f64,
    duration_us: f64,
    text: String,
}

impl SubtitleEvent {
    pub fn new(start_us: f64, duration_us: f64, text: String) -> Self {
        SubtitleEvent { start_us, duration_us, text }
    }
}

#[wasm_bindgen]
impl SubtitleEvent {
    #[wasm_bindgen(getter)]
    pub fn start_us(&self) -> f64 { self.start_us }
    #[wasm_bindgen(getter)]
    pub fn duration_us(&self) -> f64 { self.duration_us }
    #[wasm_bindgen(getter)]
    pub fn text(&self) -> String { self.text.clone() }
}

/// Metadata for a selectable track
#[wasm_bindgen]
pub struct TrackInfo {
    name: String,
    language: String,
    codec: String,
}

impl TrackInfo {
    pub fn from_parts(name: &str, language: &str, codec: &str) -> Self {
        TrackInfo { name: name.to_string(), language: language.to_string(), codec: codec.to_string() }
    }
}

#[wasm_bindgen]
impl TrackInfo {
    #[wasm_bindgen(getter)]
    pub fn name(&self) -> String { self.name.clone() }
    #[wasm_bindgen(getter)]
    pub fn language(&self) -> String { self.language.clone() }
    #[wasm_bindgen(getter)]
    pub fn codec(&self) -> String { self.codec.clone() }
}

struct AudioTrackData {
    track_id: u64,
    sample_rate: u32,
    channel_count: u16,
    codec_config: Vec<u8>,
    name: String,
    language: String,
    codec_id: String,
    frames: Vec<FrameData>,
}

struct SubtitleTrackData {
    track_id: u64,
    header: String,
    name: String,
    language: String,
    codec_id: String,
    events: Vec<SubtitleEvent>,
}

#[wasm_bindgen]
pub struct MkvDemuxer {
    video_codec_private: Vec<u8>,
    width: u32,
    height: u32,
    video_frames: Vec<FrameData>,
    duration_ms: f64,
    // Multiple audio tracks
    audio_tracks: Vec<AudioTrackData>,
    selected_audio: usize,
    // Multiple subtitle tracks
    subtitle_tracks: Vec<SubtitleTrackData>,
    selected_subtitle: usize,
}

fn track_info_from(t: &TrackEntry) -> (String, String, String) {
    (
        t.name().unwrap_or("").to_string(),
        t.language().unwrap_or("und").to_string(),
        t.codec_id().to_string(),
    )
}

#[wasm_bindgen]
impl MkvDemuxer {
    #[wasm_bindgen(constructor)]
    pub fn new(data: Vec<u8>) -> Result<MkvDemuxer, JsValue> {
        #[cfg(target_arch = "wasm32")] console_error_panic_hook::set_once();

        let cursor = Cursor::new(&data);
        let mut mkv = MatroskaFile::open(cursor)
            .map_err(|e| JsValue::from_str(&format!("MKV parse error: {}", e)))?;

        let tracks = mkv.tracks().to_vec();

        // Video track (first one)
        let video_entry = tracks.iter().find(|t| t.track_type() == TrackType::Video)
            .ok_or_else(|| JsValue::from_str("No video track"))?;
        let video_track = video_entry.track_number().get();
        let video_codec_private = video_entry.codec_private().unwrap_or(&[]).to_vec();
        let (width, height) = video_entry.video()
            .map(|v| (v.pixel_width().get() as u32, v.pixel_height().get() as u32))
            .unwrap_or((0, 0));

        // All audio tracks
        let mut audio_tracks: Vec<AudioTrackData> = Vec::new();
        for t in tracks.iter().filter(|t| t.track_type() == TrackType::Audio) {
            let (name, language, codec_id) = track_info_from(t);
            let (sr, ch) = t.audio()
                .map(|a| (a.sampling_frequency() as u32, a.channels().get() as u16))
                .unwrap_or((0, 0));
            audio_tracks.push(AudioTrackData {
                track_id: t.track_number().get(),
                sample_rate: sr,
                channel_count: ch,
                codec_config: t.codec_private().unwrap_or(&[]).to_vec(),
                name, language, codec_id,
                frames: Vec::new(),
            });
        }

        // All subtitle tracks
        let mut subtitle_tracks: Vec<SubtitleTrackData> = Vec::new();
        for t in tracks.iter().filter(|t| t.track_type() == TrackType::Subtitle) {
            let (name, language, codec_id) = track_info_from(t);
            let header = t.codec_private()
                .map(|b| String::from_utf8_lossy(b).to_string())
                .unwrap_or_default();
            subtitle_tracks.push(SubtitleTrackData {
                track_id: t.track_number().get(),
                header, name, language, codec_id,
                events: Vec::new(),
            });
        }

        // Build track ID lookup maps
        let audio_ids: Vec<u64> = audio_tracks.iter().map(|a| a.track_id).collect();
        let sub_ids: Vec<u64> = subtitle_tracks.iter().map(|s| s.track_id).collect();

        // Duration
        let timescale_ns = mkv.info().timestamp_scale().get();
        let duration_ms = mkv.info().duration()
            .map(|d| d * timescale_ns as f64 / 1_000_000.0)
            .unwrap_or(0.0);

        // Read all frames
        let mut video_frames = Vec::new();
        let mut frame = Frame::default();
        while mkv.next_frame(&mut frame)
            .map_err(|e| JsValue::from_str(&format!("MKV frame error: {}", e)))?
        {
            let ts_us = (frame.timestamp as f64 * timescale_ns as f64) / 1_000.0;
            let dur_us = frame.duration
                .map(|d| (d as f64 * timescale_ns as f64) / 1_000.0)
                .unwrap_or(0.0);

            if frame.track == video_track {
                video_frames.push(FrameData {
                    timestamp_us: ts_us, duration_us: dur_us,
                    data: std::mem::take(&mut frame.data),
                    is_keyframe: frame.is_keyframe.unwrap_or(false),
                });
            } else if let Some(idx) = audio_ids.iter().position(|&id| id == frame.track) {
                audio_tracks[idx].frames.push(FrameData {
                    timestamp_us: ts_us, duration_us: dur_us,
                    data: std::mem::take(&mut frame.data),
                    is_keyframe: true,
                });
            } else if let Some(idx) = sub_ids.iter().position(|&id| id == frame.track) {
                let text = String::from_utf8_lossy(&frame.data).to_string();
                subtitle_tracks[idx].events.push(SubtitleEvent {
                    start_us: ts_us, duration_us: dur_us, text,
                });
                frame.data.clear();
            } else {
                frame.data.clear();
            }
        }

        Ok(MkvDemuxer {
            video_codec_private, width, height,
            video_frames, duration_ms,
            audio_tracks, selected_audio: 0,
            subtitle_tracks, selected_subtitle: 0,
        })
    }

    // ── Video ──

    pub fn width(&self) -> u32 { self.width }
    pub fn height(&self) -> u32 { self.height }
    pub fn sample_count(&self) -> u32 { self.video_frames.len() as u32 }
    pub fn duration_ms(&self) -> f64 { self.duration_ms }
    pub fn codec_description(&self) -> Vec<u8> { self.video_codec_private.clone() }

    pub fn nal_length_size(&self) -> u8 {
        if self.video_codec_private.len() > 21 {
            (self.video_codec_private[21] & 0x03) + 1
        } else { 4 }
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

    // ── Audio (multi-track) ──

    pub fn has_audio(&self) -> bool { !self.audio_tracks.is_empty() }
    pub fn audio_track_count(&self) -> u32 { self.audio_tracks.len() as u32 }

    pub fn audio_track_info(&self, index: u32) -> Option<TrackInfo> {
        let t = self.audio_tracks.get(index as usize)?;
        Some(TrackInfo { name: t.name.clone(), language: t.language.clone(), codec: t.codec_id.clone() })
    }

    pub fn set_audio_track(&mut self, index: u32) { self.selected_audio = index as usize; }

    pub fn audio_sample_rate(&self) -> u32 {
        self.audio_tracks.get(self.selected_audio).map_or(0, |a| a.sample_rate)
    }

    pub fn audio_channel_count(&self) -> u16 {
        self.audio_tracks.get(self.selected_audio).map_or(0, |a| a.channel_count)
    }

    pub fn audio_codec_config(&self) -> Vec<u8> {
        self.audio_tracks.get(self.selected_audio)
            .map_or_else(Vec::new, |a| a.codec_config.clone())
    }

    pub fn audio_sample_count(&self) -> u32 {
        self.audio_tracks.get(self.selected_audio).map_or(0, |a| a.frames.len() as u32)
    }

    pub fn read_audio_sample(&self, index: u32) -> Option<Sample> {
        let a = self.audio_tracks.get(self.selected_audio)?;
        let f = a.frames.get(index as usize)?;
        Some(Sample::new(true, f.timestamp_us, f.duration_us, f.data.clone()))
    }

    pub fn find_audio_sample_at(&self, target_us: f64) -> u32 {
        let a = match self.audio_tracks.get(self.selected_audio) { Some(a) => a, None => return 0 };
        let mut best = 0u32;
        for (i, f) in a.frames.iter().enumerate() {
            if f.timestamp_us <= target_us { best = i as u32; } else { break; }
        }
        best
    }

    // ── Subtitles (multi-track) ──

    pub fn has_subtitles(&self) -> bool { !self.subtitle_tracks.is_empty() }
    pub fn subtitle_track_count(&self) -> u32 { self.subtitle_tracks.len() as u32 }

    pub fn subtitle_track_info(&self, index: u32) -> Option<TrackInfo> {
        let t = self.subtitle_tracks.get(index as usize)?;
        Some(TrackInfo { name: t.name.clone(), language: t.language.clone(), codec: t.codec_id.clone() })
    }

    pub fn set_subtitle_track(&mut self, index: u32) { self.selected_subtitle = index as usize; }

    pub fn subtitle_header(&self) -> String {
        self.subtitle_tracks.get(self.selected_subtitle)
            .map_or_else(String::new, |s| s.header.clone())
    }

    pub fn subtitle_count(&self) -> u32 {
        self.subtitle_tracks.get(self.selected_subtitle).map_or(0, |s| s.events.len() as u32)
    }

    pub fn subtitle_event(&self, index: u32) -> Option<SubtitleEvent> {
        let s = self.subtitle_tracks.get(self.selected_subtitle)?;
        let e = s.events.get(index as usize)?;
        Some(SubtitleEvent { start_us: e.start_us, duration_us: e.duration_us, text: e.text.clone() })
    }
}
