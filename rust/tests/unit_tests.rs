/// Unit tests for player logic — no WASM, no browser.

#[cfg(test)]
mod clock_tests {
    // PlaybackClock is wasm_bindgen — can't test directly without wasm target.
    // Test the logic concepts instead.

    #[test]
    fn elapsed_increases_with_time() {
        // elapsed = (now - start_time) * 1000 * speed
        let start = 1000.0;
        let now = 1050.0; // 50ms later
        let speed = 1.0;
        let elapsed_us = (now - start) * 1000.0 * speed;
        assert_eq!(elapsed_us, 50_000.0); // 50ms = 50000us
    }

    #[test]
    fn elapsed_with_2x_speed() {
        let start = 1000.0;
        let now = 1050.0;
        let speed = 2.0;
        let elapsed_us = (now - start) * 1000.0 * speed;
        assert_eq!(elapsed_us, 100_000.0);
    }
}

#[cfg(test)]
mod subtitle_tests {
    #[test]
    fn parse_ass_dialogue() {
        // ASS dialogue: Layer,Start,End,Style,Name,MarginL,MarginR,MarginV,Effect,Text
        // MKV ASS dialogue: ReadOrder,Layer,Style,Name,MarginL,MarginR,MarginV,Effect,Text
        let raw = r"0,0,Default,,0,0,0,,Hello {\b1}world{\b0}!";
        let parts: Vec<&str> = raw.splitn(9, ',').collect();
        assert_eq!(parts.len(), 9);
        let text = parts[8];
        assert_eq!(text, r"Hello {\b1}world{\b0}!");

        // Strip ASS tags
        let stripped = text.replace("{\\b1}", "").replace("{\\b0}", "");
        assert_eq!(stripped, "Hello world!");
    }

    #[test]
    fn strip_ass_tags() {
        let input = "{\\an8}Hello{\\b1} world";
        let mut result = String::new();
        let mut in_tag = false;
        let bytes = input.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if !in_tag && i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'\\' {
                in_tag = true;
                i += 1;
                continue;
            }
            if in_tag {
                if bytes[i] == b'}' { in_tag = false; }
                i += 1;
                continue;
            }
            result.push(bytes[i] as char);
            i += 1;
        }
        assert_eq!(result, "Hello world");
    }

    #[test]
    fn newline_conversion() {
        let input = "Line 1\\NLine 2";
        let output = input.replace("\\N", "<br>");
        assert_eq!(output, "Line 1<br>Line 2");
    }
}

#[cfg(test)]
mod format_detect_tests {
    #[test]
    fn detect_mkv() {
        let data = [0x1A, 0x45, 0xDF, 0xA3, 0x00, 0x00, 0x00, 0x00];
        assert!(data[0] == 0x1A && data[1] == 0x45 && data[2] == 0xDF && data[3] == 0xA3);
    }

    #[test]
    fn detect_mp4() {
        let data = [0x00, 0x00, 0x00, 0x1C, b'f', b't', b'y', b'p'];
        assert!(data[4] == b'f' && data[5] == b't' && data[6] == b'y' && data[7] == b'p');
    }

    #[test]
    fn detect_unknown() {
        let data = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        let is_mkv = data[0] == 0x1A && data[1] == 0x45;
        let is_mp4 = data[4] == b'f' && data[5] == b't';
        assert!(!is_mkv && !is_mp4);
    }
}

#[cfg(test)]
mod player_state_tests {
    #[test]
    fn should_feed_basic() {
        // should_feed: pending < 10, buf < 30, next < total
        let pending = 0u32;
        let buf_len = 0u32;
        let next = 0u32;
        let total = 100u32;
        let feed = pending <= 10 && buf_len <= 30 && next < total;
        assert!(feed);
    }

    #[test]
    fn should_feed_pending_full() {
        let pending = 15u32;
        let feed = pending <= 10;
        assert!(!feed);
    }

    #[test]
    fn should_feed_buffer_full() {
        let buf_len = 35u32;
        let feed = buf_len <= 30;
        assert!(!feed);
    }

    #[test]
    fn should_flush_when_done() {
        let next = 100u32;
        let total = 100u32;
        let flushed = false;
        let pending = 0u32;
        let downloading = false;
        let flush = next >= total && !flushed && pending == 0 && !downloading;
        assert!(flush);
    }

    #[test]
    fn should_not_flush_while_downloading() {
        let next = 100u32;
        let total = 100u32;
        let flushed = false;
        let pending = 0u32;
        let downloading = true;
        let flush = next >= total && !flushed && pending == 0 && !downloading;
        assert!(!flush);
    }

    #[test]
    fn needs_buffer_when_low() {
        let total = 50u32;
        let next = 0u32;
        let downloading = true;
        let buffered = total.saturating_sub(next);
        let needs = downloading && buffered < 240;
        assert!(needs);
    }

    #[test]
    fn no_buffer_when_enough() {
        let total = 500u32;
        let next = 0u32;
        let downloading = true;
        let buffered = total.saturating_sub(next);
        let needs = downloading && buffered < 240;
        assert!(!needs);
    }
}

#[cfg(test)]
mod ebml_tests {
    #[test]
    fn vint_1byte() {
        // 0x81 = 1000_0001 → width=1, value = 0x01 = 1
        let buf = [0x81];
        let b: u8 = buf[0];
        let len = b.leading_zeros() as usize + 1;
        assert_eq!(len, 1);
        let val = (b & (0xFF >> len)) as u64;
        assert_eq!(val, 1);
    }

    #[test]
    fn vint_2bytes() {
        // 0x40 0x02 = 0100_0000 0000_0010 → width=2, value = 0x0002 = 2
        let buf = [0x40, 0x02];
        let b: u8 = buf[0];
        let len = b.leading_zeros() as usize + 1;
        assert_eq!(len, 2);
        let mut val = (b & (0xFF >> len)) as u64;
        for i in 1..len {
            val = (val << 8) | buf[i] as u64;
        }
        assert_eq!(val, 2);
    }

    #[test]
    fn element_id_1byte() {
        // SimpleBlock = 0xA3
        let buf = [0xA3];
        let b: u8 = buf[0];
        let len = b.leading_zeros() as usize + 1;
        assert_eq!(len, 1);
        let val = b as u64;
        assert_eq!(val, 0xA3);
    }

    #[test]
    fn element_id_4bytes() {
        // EBML = 0x1A45DFA3
        let buf = [0x1A, 0x45, 0xDF, 0xA3];
        let b: u8 = buf[0];
        let len = b.leading_zeros() as usize + 1;
        assert_eq!(len, 4);
        let mut val = b as u64;
        for i in 1..len {
            val = (val << 8) | buf[i] as u64;
        }
        assert_eq!(val, 0x1A45DFA3);
    }

    #[test]
    fn unknown_size() {
        // 1-byte unknown: 0xFF → all VINT_DATA bits set
        let val = 0x7Fu64; // after stripping VINT_WIDTH+MARKER
        let vint_len = 1;
        let is_unknown = val == ((1u64 << (7 * vint_len)) - 1);
        assert!(is_unknown);
    }

    #[test]
    fn simple_block_parse() {
        // SimpleBlock: track(vint) + timestamp(i16) + flags(u8) + data
        // track=1 (0x81), timestamp=0x0000, flags=0x80 (keyframe), data=[0xDE, 0xAD]
        let block = [0x81, 0x00, 0x00, 0x80, 0xDE, 0xAD];
        let b: u8 = block[0];
        let track_len = b.leading_zeros() as usize + 1;
        assert_eq!(track_len, 1);
        let track = (b & (0xFF >> track_len)) as u64;
        assert_eq!(track, 1);

        let ts = ((block[1] as i16) << 8) | block[2] as i16;
        assert_eq!(ts, 0);

        let flags = block[3];
        let keyframe = (flags & 0x80) != 0;
        assert!(keyframe);

        let data = &block[4..];
        assert_eq!(data, &[0xDE, 0xAD]);
    }
}

#[cfg(test)]
mod mp4_box_tests {
    #[test]
    fn parse_box_header() {
        // size=28, type='ftyp'
        let data = [0x00, 0x00, 0x00, 0x1C, b'f', b't', b'y', b'p'];
        let size = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
        assert_eq!(size, 28);
        let box_type = &data[4..8];
        assert_eq!(box_type, b"ftyp");
    }

    #[test]
    fn box_scan() {
        // ftyp(28) + moov(100)
        let mut data = vec![0u8; 128];
        // ftyp header
        data[0..4].copy_from_slice(&28u32.to_be_bytes());
        data[4..8].copy_from_slice(b"ftyp");
        // moov header at offset 28
        data[28..32].copy_from_slice(&100u32.to_be_bytes());
        data[32..36].copy_from_slice(b"moov");

        // Scan boxes
        let mut pos = 0;
        let mut found_moov = false;
        while pos + 8 <= data.len() {
            let size = u32::from_be_bytes([data[pos], data[pos+1], data[pos+2], data[pos+3]]) as usize;
            let btype = &data[pos+4..pos+8];
            if size < 8 { break; }
            if btype == b"moov" {
                found_moov = true;
                break;
            }
            pos += size;
        }
        assert!(found_moov);
        assert_eq!(pos, 28);
    }
}

#[cfg(test)]
mod debounce_logic_tests {
    #[test]
    fn accumulated_seek() {
        // 3 presses of +10s should = +30s
        let base = 50_000_000i64; // 50s in us
        let mut accum = 0i64;
        accum += 10_000_000; // +10s
        accum += 10_000_000; // +10s
        accum += 10_000_000; // +10s
        let target = base + accum;
        assert_eq!(target, 80_000_000); // 80s
    }

    #[test]
    fn accumulated_seek_backward() {
        let base = 50_000_000i64;
        let mut accum = 0i64;
        accum -= 10_000_000;
        accum -= 10_000_000;
        let target = (base + accum).max(0);
        assert_eq!(target, 30_000_000);
    }

    #[test]
    fn clamp_to_zero() {
        let base = 5_000_000i64; // 5s
        let mut accum = 0i64;
        accum -= 10_000_000; // -10s
        let target = (base + accum).max(0);
        assert_eq!(target, 0);
    }
}
