use crate::matroska::{Frame, MatroskaFile, TrackEntry, TrackType};
use std::io::Cursor;
use wasm_bindgen::prelude::*;

use crate::demuxer::Sample;

#[wasm_bindgen]
pub struct MkvDemuxer {
    data: Vec<u8>,
    // Track metadata
    video_track: u64,
    audio_track: Option<u64>,
    subtitle_track: Option<u64>,
    video_codec_private: Vec<u8>,
    audio_codec_private: Vec<u8>,
    subtitle_header: String,
    width: u32,
    height: u32,
    sample_rate: u32,
    channel_count: u16,
    // Pre-read all frames into sorted lists
    video_frames: Vec<FrameData>,
    audio_frames: Vec<FrameData>,
    subtitle_events: Vec<SubtitleEvent>,
    duration_ms: f64,
}

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

#[wasm_bindgen]
impl SubtitleEvent {
    #[wasm_bindgen(getter)]
    pub fn start_us(&self) -> f64 { self.start_us }
    #[wasm_bindgen(getter)]
    pub fn duration_us(&self) -> f64 { self.duration_us }
    #[wasm_bindgen(getter)]
    pub fn text(&self) -> String { self.text.clone() }
}

fn find_track(tracks: &[TrackEntry], track_type: TrackType) -> Option<&TrackEntry> {
    tracks.iter().find(|t| t.track_type() == track_type)
}

#[wasm_bindgen]
impl MkvDemuxer {
    #[wasm_bindgen(constructor)]
    pub fn new(data: Vec<u8>) -> Result<MkvDemuxer, JsValue> {
        console_error_panic_hook::set_once();

        let cursor = Cursor::new(&data);
        let mut mkv = MatroskaFile::open(cursor)
            .map_err(|e| JsValue::from_str(&format!("MKV parse error: {}", e)))?;

        let tracks = mkv.tracks().to_vec();

        // Find video track
        let video_entry = find_track(&tracks, TrackType::Video)
            .ok_or_else(|| JsValue::from_str("No video track"))?;
        let video_track = video_entry.track_number().get();
        let video_codec_private = video_entry.codec_private().unwrap_or(&[]).to_vec();
        let (width, height) = video_entry.video()
            .map(|v| (v.pixel_width().get() as u32, v.pixel_height().get() as u32))
            .unwrap_or((0, 0));

        // Find audio track
        let audio_entry = find_track(&tracks, TrackType::Audio);
        let audio_track = audio_entry.map(|t| t.track_number().get());
        let audio_codec_private = audio_entry
            .and_then(|t| t.codec_private())
            .unwrap_or(&[]).to_vec();
        let (sample_rate, channel_count) = audio_entry
            .and_then(|t| t.audio())
            .map(|a| (a.sampling_frequency() as u32, a.channels().get() as u16))
            .unwrap_or((0, 0));

        // Find subtitle track
        let subtitle_entry = find_track(&tracks, TrackType::Subtitle);
        let subtitle_track = subtitle_entry.map(|t| t.track_number().get());
        let subtitle_header = subtitle_entry
            .and_then(|t| t.codec_private())
            .map(|b| String::from_utf8_lossy(b).to_string())
            .unwrap_or_default();

        let timescale_ns = mkv.info().timestamp_scale().get();

        // Duration: raw value is in TimecodeScale units → multiply to get ns → convert to ms
        let duration_ms = mkv.info().duration()
            .map(|d| d * timescale_ns as f64 / 1_000_000.0)
            .unwrap_or(0.0);

        // Read all frames
        let mut video_frames = Vec::new();
        let mut audio_frames = Vec::new();
        let mut subtitle_events = Vec::new();

        let mut frame = Frame::default();
        while mkv.next_frame(&mut frame)
            .map_err(|e| JsValue::from_str(&format!("MKV frame error: {}", e)))?
        {
            let ts_us = (frame.timestamp as f64 * timescale_ns as f64) / 1_000.0;
            // Duration in microseconds (if available, from block duration)
            let dur_us = frame.duration
                .map(|d| (d as f64 * timescale_ns as f64) / 1_000.0)
                .unwrap_or(0.0);

            if frame.track == video_track {
                video_frames.push(FrameData {
                    timestamp_us: ts_us,
                    duration_us: dur_us,
                    data: std::mem::take(&mut frame.data),
                    is_keyframe: frame.is_keyframe.unwrap_or(false),
                });
            } else if Some(frame.track) == audio_track {
                audio_frames.push(FrameData {
                    timestamp_us: ts_us,
                    duration_us: dur_us,
                    data: std::mem::take(&mut frame.data),
                    is_keyframe: true,
                });
            } else if Some(frame.track) == subtitle_track {
                // ASS/SSA subtitle data in MKV is the dialogue line (after the header fields)
                let text = String::from_utf8_lossy(&frame.data).to_string();
                subtitle_events.push(SubtitleEvent {
                    start_us: ts_us,
                    duration_us: dur_us,
                    text,
                });
                frame.data.clear();
            } else {
                frame.data.clear();
            }
        }

        Ok(MkvDemuxer {
            data,
            video_track,
            audio_track,
            subtitle_track,
            video_codec_private,
            audio_codec_private,
            subtitle_header,
            width,
            height,
            sample_rate,
            channel_count,
            video_frames,
            audio_frames,
            subtitle_events,
            duration_ms,
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
        Some(Sample::new(
            f.is_keyframe,
            f.timestamp_us,
            f.duration_us,
            f.data.clone(),
        ))
    }

    pub fn find_keyframe_before(&self, target_us: f64) -> u32 {
        let mut best = 0u32;
        for (i, f) in self.video_frames.iter().enumerate() {
            if !f.is_keyframe { continue; }
            if f.timestamp_us <= target_us { best = i as u32; } else { break; }
        }
        best
    }

    pub fn find_audio_sample_at(&self, target_us: f64) -> u32 {
        let mut best = 0u32;
        for (i, f) in self.audio_frames.iter().enumerate() {
            if f.timestamp_us <= target_us { best = i as u32; } else { break; }
        }
        best
    }

    // ── Audio ──

    pub fn has_audio(&self) -> bool { self.audio_track.is_some() }
    pub fn audio_sample_rate(&self) -> u32 { self.sample_rate }
    pub fn audio_channel_count(&self) -> u16 { self.channel_count }
    pub fn audio_codec_config(&self) -> Vec<u8> { self.audio_codec_private.clone() }
    pub fn audio_sample_count(&self) -> u32 { self.audio_frames.len() as u32 }

    pub fn read_audio_sample(&self, index: u32) -> Option<Sample> {
        let f = self.audio_frames.get(index as usize)?;
        Some(Sample::new(true, f.timestamp_us, f.duration_us, f.data.clone()))
    }

    // ── Subtitles ──

    pub fn has_subtitles(&self) -> bool { self.subtitle_track.is_some() }
    pub fn subtitle_header(&self) -> String { self.subtitle_header.clone() }
    pub fn subtitle_count(&self) -> u32 { self.subtitle_events.len() as u32 }

    pub fn subtitle_event(&self, index: u32) -> Option<SubtitleEvent> {
        let e = self.subtitle_events.get(index as usize)?;
        Some(SubtitleEvent {
            start_us: e.start_us,
            duration_us: e.duration_us,
            text: e.text.clone(),
        })
    }
}
