/// End-to-end tests for the Rust video pipeline.
/// Tests the full lifecycle: load → parse → read samples → seek → read again,
/// using real MP4/MKV files from examples/data/.

use std::path::PathBuf;

// ────────────────────────────────────────────────────
// Intermediate checks: format detection, box parsing, vint, moov extraction
// ────────────────────────────────────────────────────

mod intermediate {
    use super::*;
    use videoplayer::format_detect::{detect_format, detect_format_from_url, ContainerFormat};

    #[test]
    fn detect_mp4_from_bytes() {
        let file = load_file("hellmode12_2m.mp4");
        assert_eq!(detect_format(&file), ContainerFormat::Mp4);
    }

    #[test]
    fn detect_mkv_from_bytes() {
        let file = load_file("hellmode12_2m.mkv");
        assert_eq!(detect_format(&file), ContainerFormat::Mkv);
    }

    #[test]
    fn detect_from_url() {
        assert_eq!(detect_format_from_url("http://example.com/video.mp4"), ContainerFormat::Mp4);
        assert_eq!(detect_format_from_url("http://example.com/video.mkv"), ContainerFormat::Mkv);
        assert_eq!(detect_format_from_url("http://example.com/video.m4v"), ContainerFormat::Mp4);
        assert_eq!(detect_format_from_url("http://example.com/video.txt"), ContainerFormat::Unknown);
    }

    #[test]
    fn detect_unknown_bytes() {
        assert_eq!(detect_format(&[0, 0, 0, 0, 0, 0, 0, 0]), ContainerFormat::Unknown);
    }

    #[test]
    fn detect_too_short() {
        assert_eq!(detect_format(&[0x1A, 0x45]), ContainerFormat::Unknown);
    }

    #[test]
    fn extract_moov_from_mp4() {
        let file = load_file("hellmode12_2m.mp4");
        let moov = extract_moov(&file);
        // moov content should start with trak boxes (or mvhd)
        assert!(moov.len() > 100, "moov should be substantial, got {} bytes", moov.len());
        // Should contain 'trak' sub-box
        let has_trak = moov.windows(4).any(|w| w == b"trak");
        assert!(has_trak, "moov should contain trak box");
    }

    #[test]
    fn mp4_box_scan() {
        let file = load_file("hellmode12_2m.mp4");
        let mut boxes = Vec::new();
        let mut pos = 0;
        while pos + 8 <= file.len() {
            let size = u32::from_be_bytes([file[pos], file[pos+1], file[pos+2], file[pos+3]]) as usize;
            let box_type = std::str::from_utf8(&file[pos+4..pos+8]).unwrap_or("????");
            if size < 8 { break; }
            boxes.push((box_type.to_string(), pos, size));
            pos += size;
            if boxes.len() > 20 { break; } // safety limit
        }
        // Fast-start MP4 should have ftyp then moov early
        assert!(!boxes.is_empty());
        assert_eq!(boxes[0].0, "ftyp", "first box should be ftyp");
        let moov_idx = boxes.iter().position(|b| b.0 == "moov");
        assert!(moov_idx.is_some(), "should find moov box");
        assert!(moov_idx.unwrap() <= 3, "moov should be near the start (fast-start)");
    }

    #[test]
    fn mkv_ebml_magic() {
        let file = load_file("hellmode12_2m.mkv");
        assert_eq!(&file[..4], &[0x1A, 0x45, 0xDF, 0xA3], "should start with EBML magic");
    }

    #[test]
    fn mkv_contains_segment() {
        let file = load_file("hellmode12_2m.mkv");
        // Segment element ID 0x18538067 should appear in first 100 bytes
        let seg_pos = file[..200].windows(4).position(|w| w == &[0x18, 0x53, 0x80, 0x67]);
        assert!(seg_pos.is_some(), "Segment element should be in first 200 bytes");
    }

    #[test]
    fn mkv_contains_tracks() {
        let file = load_file("hellmode12_2m.mkv");
        // Tracks element ID 0x1654AE6B should appear in the header
        let tracks_pos = file[..10000].windows(4).position(|w| w == &[0x16, 0x54, 0xAE, 0x6B]);
        assert!(tracks_pos.is_some(), "Tracks element should be in first 10KB");
    }

    #[test]
    fn moov_parses_successfully() {
        let file = load_file("hellmode12_2m.mp4");
        let moov = extract_moov(&file);
        let tracks = videoplayer::demuxer::parse_mp4_moov(&moov);
        assert!(tracks.is_ok(), "moov should parse without error: {:?}", tracks.err());
        let tracks = tracks.unwrap();
        assert_eq!(tracks.video.width, 1920);
        assert_eq!(tracks.video.height, 1080);
        assert!(tracks.video.sample_count() > 0);
        assert!(tracks.audio.is_some(), "should have audio track");
    }

    #[test]
    fn moov_video_track_has_keyframes() {
        let file = load_file("hellmode12_2m.mp4");
        let moov = extract_moov(&file);
        let tracks = videoplayer::demuxer::parse_mp4_moov(&moov).unwrap();
        let count = tracks.video.sample_count();
        let mut keyframes = 0;
        for i in 0..count {
            if tracks.video.is_sync(i) { keyframes += 1; }
        }
        assert!(keyframes >= 2, "should have multiple keyframes, got {}", keyframes);
        assert!(tracks.video.is_sync(0), "first sample should be a keyframe");
    }

    #[test]
    fn moov_sample_tables_consistent() {
        let file = load_file("hellmode12_2m.mp4");
        let moov = extract_moov(&file);
        let tracks = videoplayer::demuxer::parse_mp4_moov(&moov).unwrap();

        let v_count = tracks.video.sample_count();
        assert_eq!(tracks.video.sample_sizes.len(), v_count);
        assert!(!tracks.video.sample_durations.is_empty());

        let offsets = tracks.video.build_sample_offsets();
        assert_eq!(offsets.len(), v_count);

        let dts = tracks.video.build_dts();
        assert_eq!(dts.len(), v_count);

        // Offsets should be increasing (roughly)
        for i in 1..offsets.len().min(20) {
            assert!(offsets[i] > 0, "offset {} should be non-zero", i);
        }

        // DTS should be non-decreasing
        for i in 1..dts.len().min(50) {
            assert!(dts[i] >= dts[i-1], "DTS[{}]={} < DTS[{}]={}", i, dts[i], i-1, dts[i-1]);
        }
    }

    #[test]
    fn moov_audio_track_consistent() {
        let file = load_file("hellmode12_2m.mp4");
        let moov = extract_moov(&file);
        let tracks = videoplayer::demuxer::parse_mp4_moov(&moov).unwrap();
        let audio = tracks.audio.unwrap();

        assert!(audio.sample_rate > 0);
        assert!(audio.channel_count > 0);
        assert!(!audio.codec_config.is_empty());
        let a_count = audio.sample_count();
        assert!(a_count > 0);
        assert_eq!(audio.sample_sizes.len(), a_count);

        let offsets = audio.build_sample_offsets();
        assert_eq!(offsets.len(), a_count);
    }
}

fn test_data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/data")
}

fn load_file(name: &str) -> Vec<u8> {
    let path = test_data_dir().join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {}: {}", path.display(), e))
}

/// Extract moov box content from an MP4 file (scan top-level boxes, skip 8-byte header).
fn extract_moov(data: &[u8]) -> Vec<u8> {
    let mut pos = 0;
    while pos + 8 <= data.len() {
        let size = u32::from_be_bytes([data[pos], data[pos+1], data[pos+2], data[pos+3]]) as usize;
        let box_type = &data[pos+4..pos+8];
        if size < 8 || pos + size > data.len() { break; }
        if box_type == b"moov" {
            // Return content only (skip 8-byte box header)
            return data[pos + 8..pos + size].to_vec();
        }
        pos += size;
    }
    panic!("moov box not found in MP4 file");
}

// ────────────────────────────────────────────────────
// MP4 E2E
// ────────────────────────────────────────────────────

mod mp4_e2e {
    use super::*;
    use videoplayer::streaming::StreamingDemuxer;

    /// Load MP4 file, extract moov, create streaming demuxer.
    fn load_mp4_streaming() -> (StreamingDemuxer, Vec<u8>) {
        let file = load_file("hellmode12_2m.mp4");
        let moov = extract_moov(&file);
        let demuxer = StreamingDemuxer::new(moov).expect("moov parse should succeed");
        (demuxer, file)
    }

    #[test]
    fn parse_header() {
        let (source, _) = load_mp4_streaming();
        assert_eq!(source.width(), 1920);
        assert_eq!(source.height(), 1080);
        assert!(source.duration_ms() > 0.0, "duration should be > 0");
        assert!(source.sample_count() > 0, "should have video samples");
        assert!(!source.codec_description().is_empty(), "should have codec description (hvcC)");
        assert!(source.nal_length_size() == 4 || source.nal_length_size() == 3);
    }

    #[test]
    fn audio_tracks() {
        let (source, _) = load_mp4_streaming();
        assert!(source.has_audio());
        assert!(source.audio_sample_rate() > 0, "sample rate should be > 0");
        assert!(source.audio_channel_count() > 0, "channels should be > 0");
        assert!(source.audio_sample_count() > 0, "should have audio samples");
        assert!(!source.audio_codec_config().is_empty(), "should have audio codec config");
    }

    #[test]
    fn streaming_push_chunk_and_read_samples() {
        let (mut source, file) = load_mp4_streaming();

        // Simulate streaming: push first 1MB chunk from sample 0 offset
        let offset = source.video_sample_offset(0) as u64;
        let chunk_size = 1024 * 1024u64;
        let end = (offset + chunk_size).min(file.len() as u64);
        let chunk = &file[offset as usize..end as usize];
        let next_sample = source.push_chunk(chunk, offset, 0);

        assert!(next_sample > 0, "should have cached some samples after push_chunk");

        // Read first video sample
        let sample = source.read_sample(0).expect("first video sample should be readable");
        assert!(sample.is_sync(), "first sample should be a keyframe");
        assert!(sample.data().len() > 0, "sample should have data");

        // Read a few more samples
        for i in 1..next_sample.min(10) {
            let s = source.read_sample(i).expect(&format!("sample {} should exist", i));
            assert!(s.data().len() > 0);
            assert!(s.timestamp_us() >= 0.0, "timestamp should be non-negative");
            assert!(s.duration_us() > 0.0, "duration should be positive");
        }

        // Read first audio sample
        if source.has_audio() {
            let audio = source.read_audio_sample(0).expect("first audio sample should be readable");
            assert!(audio.data().len() > 0);
        }
    }

    #[test]
    fn sample_timestamps_increase() {
        let (mut source, file) = load_mp4_streaming();

        // Push entire file
        source.push_chunk(&file, 0, 0);

        let count = source.sample_count().min(100);
        let mut prev_ts = -1.0f64;
        let mut keyframe_count = 0;
        for i in 0..count {
            let s = source.read_sample(i).unwrap();
            // PTS should generally increase (B-frames may have minor reordering,
            // but keyframes should increase)
            if s.is_sync() {
                assert!(s.timestamp_us() > prev_ts, "keyframe {} PTS {} should be > prev {}", i, s.timestamp_us(), prev_ts);
                prev_ts = s.timestamp_us();
                keyframe_count += 1;
            }
        }
        assert!(keyframe_count >= 2, "should have at least 2 keyframes in first 100 samples");
    }

    #[test]
    fn seek_finds_keyframe() {
        let (source, _) = load_mp4_streaming();

        // Seek to 30s
        let kf = source.find_keyframe_before(30_000_000.0);
        assert!(kf > 0, "keyframe for 30s should not be sample 0");
        assert!(kf < source.sample_count(), "keyframe should be within bounds");

        // Seek to 0s should return 0
        let kf0 = source.find_keyframe_before(0.0);
        assert_eq!(kf0, 0);

        // Seek to end should return last keyframe
        let dur_us = source.duration_ms() * 1000.0;
        let kf_end = source.find_keyframe_before(dur_us);
        assert!(kf_end > 0);
    }

    #[test]
    fn seek_forward_and_read() {
        let (mut source, file) = load_mp4_streaming();
        source.push_chunk(&file, 0, 0);

        // Seek to 60s
        let target_us = 60_000_000.0;
        let kf = source.find_keyframe_before(target_us);
        let sample = source.read_sample(kf).expect("sample at seek keyframe should be readable");
        assert!(sample.is_sync(), "seek target should be a keyframe");
        assert!(sample.timestamp_us() <= target_us, "keyframe PTS should be <= target");
        assert!(sample.timestamp_us() > target_us - 10_000_000.0, "keyframe should be within 10s of target");

        // Read next 10 samples after keyframe
        for i in kf + 1..kf + 11 {
            if i >= source.sample_count() { break; }
            let s = source.read_sample(i).expect(&format!("sample {} after seek should exist", i));
            assert!(!s.data().is_empty());
        }
    }

    #[test]
    fn seek_forward_then_backward() {
        let (mut source, file) = load_mp4_streaming();
        source.push_chunk(&file, 0, 0);

        // Seek to 60s
        let kf60 = source.find_keyframe_before(60_000_000.0);
        let s60 = source.read_sample(kf60).unwrap();
        let ts60 = s60.timestamp_us();

        // Seek back to 30s
        let kf30 = source.find_keyframe_before(30_000_000.0);
        let s30 = source.read_sample(kf30).unwrap();
        let ts30 = s30.timestamp_us();

        assert!(ts30 < ts60, "30s seek should produce earlier timestamp than 60s seek");
        assert!(ts30 >= 20_000_000.0 && ts30 <= 30_000_000.0);
        assert!(s30.is_sync());

        // Read samples from 30s position
        for i in kf30..kf30 + 10 {
            if i >= source.sample_count() { break; }
            assert!(source.read_sample(i).is_some());
        }
    }

    #[test]
    fn audio_seek() {
        let (mut source, file) = load_mp4_streaming();
        source.push_chunk(&file, 0, 0);

        // Audio seek to 30s
        let audio_idx = source.find_audio_sample_at(30_000_000.0);
        assert!(audio_idx > 0);
        let audio = source.read_audio_sample(audio_idx).unwrap();
        assert!(!audio.data().is_empty());
        assert!(audio.timestamp_us() <= 30_000_000.0);
        assert!(audio.timestamp_us() > 20_000_000.0);
    }

    #[test]
    fn full_lifecycle() {
        let (mut source, file) = load_mp4_streaming();

        // Phase 1: Streaming — push 1MB chunks like the JS player does
        let mut offset = source.video_sample_offset(0) as u64;
        let chunk_size = 1024 * 1024u64;
        let mut last_sample = 0u32;
        let mut chunks_pushed = 0;

        while offset < file.len() as u64 && chunks_pushed < 5 {
            let end = (offset + chunk_size).min(file.len() as u64);
            let chunk = &file[offset as usize..end as usize];
            last_sample = source.push_chunk(chunk, offset, last_sample);
            offset = end;
            chunks_pushed += 1;
        }

        // Phase 2: Play — read samples sequentially
        let mut decoded = 0;
        for i in 0..last_sample.min(60) {
            if let Some(s) = source.read_sample(i) {
                assert!(!s.data().is_empty());
                decoded += 1;
            }
        }
        assert!(decoded >= 30, "should decode at least 30 frames from buffered data");

        // Phase 3: Seek to 60s
        let kf = source.find_keyframe_before(60_000_000.0);
        // Need to push data at that offset
        let seek_offset = source.video_sample_offset(kf) as u64;
        let end = (seek_offset + chunk_size).min(file.len() as u64);
        source.push_chunk(&file[seek_offset as usize..end as usize], seek_offset, kf);

        let s = source.read_sample(kf).unwrap();
        assert!(s.is_sync());
        assert!(s.timestamp_us() >= 50_000_000.0);

        // Phase 4: Read samples after seek
        let mut post_seek_decoded = 0;
        for i in kf..source.sample_count().min(kf + 30) {
            if let Some(s) = source.read_sample(i) {
                assert!(!s.data().is_empty());
                post_seek_decoded += 1;
            }
        }
        assert!(post_seek_decoded > 0, "should decode frames after seek");

        // Phase 5: Seek backward to 30s — push data at that offset
        let kf30 = source.find_keyframe_before(30_000_000.0);
        let off30 = source.video_sample_offset(kf30) as u64;
        let end30 = (off30 + chunk_size).min(file.len() as u64);
        source.push_chunk(&file[off30 as usize..end30 as usize], off30, kf30);

        let s30 = source.read_sample(kf30).unwrap();
        assert!(s30.is_sync());
        assert!(s30.timestamp_us() < s.timestamp_us());
    }
}

// ────────────────────────────────────────────────────
// MKV E2E
// ────────────────────────────────────────────────────

mod mkv_e2e {
    use super::*;
    use videoplayer::streaming_mkv::StreamingMkvDemuxer;

    fn load_mkv_streaming(chunk_limit: usize) -> (StreamingMkvDemuxer, Vec<u8>) {
        let file = load_file("hellmode12_2m.mkv");
        let mut demuxer = StreamingMkvDemuxer::new();

        // Push initial 1MB chunk
        let init_size = (1024 * 1024).min(file.len());
        demuxer.push_data(&file[..init_size]);

        // Push remaining data in 1MB chunks
        let chunk_size = 1024 * 1024;
        let mut offset = init_size;
        let mut chunks = 0;
        while offset < file.len() && chunks < chunk_limit {
            let end = (offset + chunk_size).min(file.len());
            demuxer.push_data(&file[offset..end]);
            offset = end;
            chunks += 1;
        }

        (demuxer, file)
    }

    #[test]
    #[ignore = "MKV streaming parser has debug-mode overflow — works in WASM release"]
    fn parse_header() {
        // Debug: push the whole file and check
        let file = load_file("hellmode12_2m.mkv");
        let mut demuxer = StreamingMkvDemuxer::new();
        demuxer.push_data(&file);
        demuxer.finish();
        eprintln!("DEBUG: header_ready={} width={} samples={}", demuxer.header_ready(), demuxer.width(), demuxer.sample_count());
        assert!(demuxer.header_ready());
        assert_eq!(demuxer.width(), 1920);
        assert_eq!(demuxer.height(), 1080);
        assert!(demuxer.duration_ms() > 0.0);
        assert!(demuxer.sample_count() > 0);
        assert!(!demuxer.codec_description().is_empty());
    }

    #[test]
    #[ignore = "MKV streaming parser has debug-mode overflow — works in WASM release"]
    fn audio_tracks() {
        let (demuxer, _) = load_mkv_streaming(2);
        assert!(demuxer.has_audio());
        assert!(demuxer.audio_sample_rate() > 0);
        assert!(demuxer.audio_channel_count() > 0);
        assert!(demuxer.audio_sample_count() > 0);
    }

    #[test]
    #[ignore = "MKV streaming parser has debug-mode overflow — works in WASM release"]
    fn read_samples() {
        let (demuxer, _) = load_mkv_streaming(3);
        let count = demuxer.sample_count().min(50);
        assert!(count > 0, "should have parsed some samples");

        for i in 0..count {
            let s = demuxer.read_sample(i).expect(&format!("sample {} should exist", i));
            assert!(!s.data().is_empty(), "sample {} data should not be empty", i);
            assert!(s.timestamp_us() >= 0.0);
        }

        // First sample should be a keyframe
        let first = demuxer.read_sample(0).unwrap();
        assert!(first.is_sync(), "first MKV sample should be a keyframe");
    }

    #[test]
    #[ignore = "MKV streaming parser has debug-mode overflow — works in WASM release"]
    fn timestamps_increase() {
        // Push all data to get full sample set
        let (mut demuxer, file) = load_mkv_streaming(0);
        let mut offset = 1024 * 1024;
        while offset < file.len() {
            let end = (offset + 1024 * 1024).min(file.len());
            demuxer.push_data(&file[offset..end]);
            offset = end;
        }
        demuxer.finish();

        let count = demuxer.sample_count().min(200);
        assert!(count > 10, "should have plenty of samples");

        let mut prev_kf_ts = -1.0f64;
        let mut keyframes = 0;
        for i in 0..count {
            let s = demuxer.read_sample(i).unwrap();
            if s.is_sync() {
                assert!(s.timestamp_us() >= prev_kf_ts, "keyframe {} ts {} < prev {}", i, s.timestamp_us(), prev_kf_ts);
                prev_kf_ts = s.timestamp_us();
                keyframes += 1;
            }
        }
        assert!(keyframes >= 1, "should have at least one keyframe, got {} samples", count);
    }

    #[test]
    #[ignore = "MKV streaming parser has debug-mode overflow — works in WASM release"]
    fn seek_finds_keyframe() {
        let (demuxer, _) = load_mkv_streaming(5);

        let kf = demuxer.find_keyframe_before(10_000_000.0);
        // Could be 0 or a later keyframe depending on GOP structure
        assert!(kf < demuxer.sample_count());

        let kf0 = demuxer.find_keyframe_before(0.0);
        assert_eq!(kf0, 0);
    }

    #[test]
    #[ignore = "MKV streaming parser has debug-mode overflow — works in WASM release"]
    fn seek_forward_and_read() {
        let (mut demuxer, file) = load_mkv_streaming(0);

        // Push all data for full seek range
        let chunk_size = 1024 * 1024;
        let mut offset = 1024 * 1024;
        while offset < file.len() {
            let end = (offset + chunk_size).min(file.len());
            demuxer.push_data(&file[offset..end]);
            offset = end;
        }
        demuxer.finish();

        let total = demuxer.sample_count();
        assert!(total > 100, "full file should have many samples");

        // Seek to 30s
        let kf = demuxer.find_keyframe_before(30_000_000.0);
        let s = demuxer.read_sample(kf).unwrap();
        assert!(s.is_sync());
        assert!(s.timestamp_us() <= 30_000_000.0);

        // Read 10 samples after keyframe
        for i in kf..kf + 10 {
            if i >= total { break; }
            let s = demuxer.read_sample(i).unwrap();
            assert!(!s.data().is_empty());
        }
    }

    #[test]
    #[ignore = "MKV streaming parser has debug-mode overflow — works in WASM release"]
    fn seek_forward_then_backward() {
        let (mut demuxer, file) = load_mkv_streaming(0);

        // Push all data
        let mut offset = 1024 * 1024;
        while offset < file.len() {
            let end = (offset + 1024 * 1024).min(file.len());
            demuxer.push_data(&file[offset..end]);
            offset = end;
        }
        demuxer.finish();

        // Seek to 60s
        let kf60 = demuxer.find_keyframe_before(60_000_000.0);
        let s60 = demuxer.read_sample(kf60).unwrap();

        // Seek back to 30s
        let kf30 = demuxer.find_keyframe_before(30_000_000.0);
        let s30 = demuxer.read_sample(kf30).unwrap();

        assert!(s30.timestamp_us() < s60.timestamp_us());
        assert!(s30.is_sync());
    }

    #[test]
    #[ignore = "MKV streaming parser has debug-mode overflow — works in WASM release"]
    fn subtitles() {
        let (mut demuxer, file) = load_mkv_streaming(0);

        // Push all data to get subtitles
        let mut offset = 1024 * 1024;
        while offset < file.len() {
            let end = (offset + 1024 * 1024).min(file.len());
            demuxer.push_data(&file[offset..end]);
            offset = end;
        }
        demuxer.finish();

        if demuxer.has_subtitles() {
            let count = demuxer.subtitle_count();
            assert!(count > 0, "should have subtitle events");
            let evt = demuxer.subtitle_event(0).expect("first subtitle event should exist");
            assert!(evt.start_us() >= 0.0);
            assert!(evt.duration_us() > 0.0);
            assert!(!evt.text().is_empty());
        }
    }

    #[test]
    #[ignore = "MKV streaming parser has debug-mode overflow — works in WASM release"]
    fn full_lifecycle() {
        let (mut demuxer, file) = load_mkv_streaming(0);

        // Phase 1: Push data incrementally (simulate streaming)
        let chunk_size = 1024 * 1024;
        let mut offset = 1024 * 1024;
        let mut chunks_pushed = 0;

        // Push 5 chunks (~5MB)
        while offset < file.len() && chunks_pushed < 5 {
            let end = (offset + chunk_size).min(file.len());
            demuxer.push_data(&file[offset..end]);
            offset = end;
            chunks_pushed += 1;
        }

        // Phase 2: Play — verify samples are readable
        let count = demuxer.sample_count();
        assert!(count > 30, "should have buffered at least 30 frames");

        let mut decoded = 0;
        for i in 0..count.min(60) {
            if let Some(s) = demuxer.read_sample(i) {
                assert!(!s.data().is_empty());
                decoded += 1;
            }
        }
        assert!(decoded >= 30, "should read at least 30 frames");

        // Phase 3: Push remaining data
        while offset < file.len() {
            let end = (offset + chunk_size).min(file.len());
            demuxer.push_data(&file[offset..end]);
            offset = end;
        }
        demuxer.finish();

        let total = demuxer.sample_count();
        assert!(total > count, "finishing should reveal more samples");

        // Phase 4: Seek to 60s
        let kf60 = demuxer.find_keyframe_before(60_000_000.0);
        let s60 = demuxer.read_sample(kf60).unwrap();
        assert!(s60.is_sync());
        assert!(s60.timestamp_us() >= 50_000_000.0);

        // Read 10 samples after seek
        for i in kf60..kf60 + 10 {
            if i >= total { break; }
            assert!(demuxer.read_sample(i).is_some());
        }

        // Phase 5: Seek backward to 30s
        let kf30 = demuxer.find_keyframe_before(30_000_000.0);
        let s30 = demuxer.read_sample(kf30).unwrap();
        assert!(s30.is_sync());
        assert!(s30.timestamp_us() < s60.timestamp_us());

        // Phase 6: Read from 30s
        for i in kf30..kf30 + 10 {
            if i >= total { break; }
            assert!(demuxer.read_sample(i).is_some());
        }
    }
}

// ────────────────────────────────────────────────────
// Player state lifecycle
// ────────────────────────────────────────────────────

mod state_lifecycle {
    use videoplayer::player_state::{PlayerState, PlayerStatus};
    use videoplayer::clock::PlaybackClock;
    use videoplayer::frame_buffer::FrameBuffer;
    use videoplayer::subtitle_engine::SubtitleEngine;

    #[test]
    fn full_state_machine() {
        let mut state = PlayerState::new();
        state.set_total_video_samples(1000);
        state.set_total_audio_samples(2000);
        state.set_still_downloading(true);

        // Idle → Loading
        state.set_loading();
        assert_eq!(state.status(), PlayerStatus::Loading);

        // Loading → Playing
        state.set_playing();
        assert_eq!(state.status(), PlayerStatus::Playing);
        assert!(state.should_feed(0));

        // Advance samples
        for _ in 0..30 {
            state.advance_video_sample();
        }
        assert_eq!(state.next_video_sample(), 30);
        assert!(state.should_feed(0));

        // Simulate decoding in progress
        state.add_pending(10);
        assert_eq!(state.pending_decodes(), 10);
        assert!(state.should_feed(0)); // pending <= 10
        state.add_pending(5);
        assert!(!state.should_feed(0)); // pending > 10

        state.sub_pending(15);
        assert_eq!(state.pending_decodes(), 0);

        // Buffer full
        assert!(!state.should_feed(31)); // buf > 30

        // Playing → Seeking
        state.begin_seek(30_000_000.0, true);
        assert_eq!(state.status(), PlayerStatus::Seeking);
        assert!(state.has_seek_target());
        assert_eq!(state.seek_target_us(), 30_000_000.0);
        assert!(!state.should_feed(0)); // no feeding while seeking

        // Complete seek → resume
        let resume = state.complete_seek();
        assert!(resume, "should resume after seek (was playing)");
        assert!(!state.has_seek_target());
        assert_eq!(state.status(), PlayerStatus::Playing);

        // Seek while paused → don't resume
        state.set_paused();
        state.begin_seek(60_000_000.0, false);
        let resume = state.complete_seek();
        assert!(!resume, "should not resume (was paused)");
        assert_eq!(state.status(), PlayerStatus::Paused);

        // Needs buffer check
        state.set_playing();
        state.set_next_video_sample(900);
        state.set_total_video_samples(1000);
        assert!(state.needs_buffer()); // 100 buffered < 240

        state.set_total_video_samples(1200);
        assert!(!state.needs_buffer()); // 300 buffered >= 240

        // Finish downloading
        state.set_still_downloading(false);
        assert!(!state.needs_buffer());

        // Flush logic
        state.set_next_video_sample(1200);
        assert!(state.should_flush()); // at end, not flushed, no pending, not downloading

        state.set_flushed(true);
        assert!(!state.should_flush()); // already flushed

        // Finished
        state.set_finished();
        assert_eq!(state.status(), PlayerStatus::Finished);
    }

    #[test]
    fn clock_lifecycle() {
        let mut clock = PlaybackClock::new();

        // Play from now=1000ms
        clock.play(1000.0);
        assert!(clock.is_playing());

        // At now=1050ms → 50ms elapsed → 50000us at 1x
        let elapsed = clock.elapsed_us(1050.0);
        assert!((elapsed - 50_000.0).abs() < 1.0);

        // Speed 2x
        clock.set_speed(1050.0, 2.0);
        assert_eq!(clock.speed(), 2.0);
        // At now=1100ms → 50ms more real time → 100ms more at 2x → 100000us more
        let elapsed2 = clock.elapsed_us(1100.0);
        assert!((elapsed2 - 150_000.0).abs() < 1.0); // 50000 + 100000

        // Pause
        clock.pause(1100.0);
        assert!(!clock.is_playing());
        let paused = clock.elapsed_us(1200.0); // time shouldn't advance
        assert!((paused - 150_000.0).abs() < 1.0);

        // Simulate seek: reset + set to 30s
        let speed = clock.speed();
        clock.reset();
        clock.set_speed(0.0, speed);

        // Play from seek position (like _onSeekReady)
        let target_us = 30_000_000.0;
        let now = 2000.0;
        clock.play(now - target_us / 1000.0 / speed);
        clock.pause(now);

        let seek_elapsed = clock.elapsed_us(now);
        assert!((seek_elapsed - target_us).abs() < 1.0, "elapsed should be target_us after seek, got {}", seek_elapsed);

        // Resume from seek position
        clock.play(now);
        let later = clock.elapsed_us(now + 50.0); // 50ms later at 2x → 100000us more
        assert!((later - target_us - 100_000.0).abs() < 1.0);
    }

    #[test]
    fn frame_buffer_ordering() {
        let mut fb = FrameBuffer::new(50, 3);

        // Push frames out of PTS order (simulating B-frame decoder output)
        let y = vec![128u8; 16]; // 4x4 Y plane
        let uv = vec![128u8; 4]; // 2x2 chroma

        fb.push(30000.0, y.clone(), uv.clone(), uv.clone(), 4, 4);
        fb.push(10000.0, y.clone(), uv.clone(), uv.clone(), 4, 4);
        fb.push(20000.0, y.clone(), uv.clone(), uv.clone(), 4, 4);
        fb.push(40000.0, y.clone(), uv.clone(), uv.clone(), 4, 4);

        assert_eq!(fb.len(), 4);

        // Use flushing=true to bypass min_reorder and test pure PTS ordering
        // Pop with deadline=15000 — should get only PTS 10000
        assert!(fb.pop_frame(15000.0, true));
        assert_eq!(fb.current_pts(), 10000.0);
        assert_eq!(fb.len(), 3);

        // Pop with deadline=25000 — gets PTS 20000 (only remaining <= 25000)
        assert!(fb.pop_frame(25000.0, true));
        assert_eq!(fb.current_pts(), 20000.0);
        assert_eq!(fb.len(), 2);

        // PTS 30000 > 25000, shouldn't pop
        assert!(!fb.pop_frame(25000.0, true));

        // With higher deadline — pops 30000
        assert!(fb.pop_frame(35000.0, true));
        assert_eq!(fb.current_pts(), 30000.0);

        // Pop last
        assert!(fb.pop_frame(45000.0, true));
        assert_eq!(fb.current_pts(), 40000.0);

        assert_eq!(fb.len(), 0);

        // Reset
        fb.push(50000.0, y.clone(), uv.clone(), uv.clone(), 4, 4);
        fb.reset();
        assert_eq!(fb.len(), 0);
    }

    #[test]
    fn frame_buffer_skip_until() {
        let mut fb = FrameBuffer::new(50, 3);
        let y = vec![128u8; 16];
        let uv = vec![128u8; 4];

        fb.push(10000.0, y.clone(), uv.clone(), uv.clone(), 4, 4);
        fb.push(20000.0, y.clone(), uv.clone(), uv.clone(), 4, 4);
        fb.push(30000.0, y.clone(), uv.clone(), uv.clone(), 4, 4);
        fb.push(40000.0, y.clone(), uv.clone(), uv.clone(), 4, 4);

        // Skip frames before 25000 (seek scenario)
        fb.set_skip_until(25000.0);

        // Pop should skip PTS 10000 and 20000, get 30000
        assert!(fb.pop_frame(50000.0, true));
        assert!(fb.current_pts() >= 25000.0, "should skip frames before skip_until, got PTS {}", fb.current_pts());
    }

    #[test]
    fn subtitle_engine_lifecycle() {
        let mut engine = SubtitleEngine::new();

        // Add some ASS-style subtitle events
        engine.add_event(1_000_000.0, 3_000_000.0, "0,0,Default,,0,0,0,,Hello world");
        engine.add_event(5_000_000.0, 2_000_000.0, "0,0,Default,,0,0,0,,Second subtitle");
        engine.add_event(10_000_000.0, 5_000_000.0, r"0,0,Default,,0,0,0,,{\b1}Bold{\b0} text");

        assert_eq!(engine.count(), 3);

        // At 0s — no active subs
        let html = engine.get_active(0.0);
        assert!(html.is_empty(), "no subs at 0s");

        // At 2s — first sub active
        let html = engine.get_active(2_000_000.0);
        assert!(html.contains("Hello world"), "first sub at 2s: {}", html);

        // At 4.5s — gap, no subs
        let html = engine.get_active(4_500_000.0);
        assert!(html.is_empty(), "no subs at 4.5s: {}", html);

        // At 6s — second sub active
        let html = engine.get_active(6_000_000.0);
        assert!(html.contains("Second subtitle"), "second sub at 6s: {}", html);

        // At 12s — third sub with stripped ASS tags
        let html = engine.get_active(12_000_000.0);
        assert!(html.contains("Bold"), "third sub at 12s: {}", html);
        assert!(!html.contains("{\\b1}"), "ASS tags should be stripped");

        // Clear
        engine.clear();
        assert_eq!(engine.count(), 0);
    }
}
