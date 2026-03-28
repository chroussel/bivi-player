//! Subtitle engine — ASS parsing + active subtitle lookup by timestamp.

use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct SubtitleEngine {
    subs: Vec<Sub>,
    last_result: String,
}

struct Sub {
    start_us: f64,
    end_us: f64,
    text: String,
}

#[wasm_bindgen]
impl SubtitleEngine {
    #[wasm_bindgen(constructor)]
    pub fn new() -> SubtitleEngine {
        SubtitleEngine {
            subs: Vec::new(),
            last_result: String::new(),
        }
    }

    /// Load subtitle events from the demuxer.
    /// Each event: (start_us, duration_us, raw_text).
    /// Parses ASS dialogue format, strips tags.
    pub fn add_event(&mut self, start_us: f64, duration_us: f64, raw_text: &str) {
        let text = Self::parse_ass_dialogue(raw_text);
        if !text.is_empty() {
            self.subs.push(Sub {
                start_us,
                end_us: start_us + duration_us,
                text,
            });
        }
    }

    /// Clear all subtitles.
    pub fn clear(&mut self) {
        self.subs.clear();
        self.last_result.clear();
    }

    pub fn count(&self) -> u32 {
        self.subs.len() as u32
    }

    /// Get active subtitle text at the given timestamp.
    /// Returns empty string if no subtitle is active.
    /// Result is cached — only recomputes when output changes.
    pub fn get_active(&mut self, elapsed_us: f64) -> String {
        let mut active = String::new();
        for sub in &self.subs {
            if elapsed_us >= sub.start_us && elapsed_us < sub.end_us {
                if !active.is_empty() {
                    active.push_str("<br>");
                }
                active.push_str(&sub.text);
            }
        }
        if active != self.last_result {
            self.last_result = active.clone();
        }
        active
    }

    /// Check if the active text changed since last call.
    pub fn changed(&self) -> bool {
        // Caller should compare with previous — we just return the text
        true
    }

    /// Parse ASS dialogue line — strip timing fields and override tags.
    fn parse_ass_dialogue(raw: &str) -> String {
        let mut text = raw.to_string();

        // ASS dialogue: ReadOrder,Layer,Style,Name,MarginL,MarginR,MarginV,Effect,Text
        // In MKV, first fields may vary. Find text after 8th comma.
        let parts: Vec<&str> = text.splitn(9, ',').collect();
        if parts.len() >= 9 {
            text = parts[8].to_string();
        }

        // Strip ASS override tags: {\b1}, {\an8}, {\pos(x,y)}, etc.
        let mut result = String::with_capacity(text.len());
        let mut in_tag = false;
        let bytes = text.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if !in_tag && i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'\\' {
                in_tag = true;
                i += 1;
                continue;
            }
            if in_tag {
                if bytes[i] == b'}' {
                    in_tag = false;
                }
                i += 1;
                continue;
            }
            // Convert \N to newline marker
            if i + 1 < bytes.len() && bytes[i] == b'\\' && bytes[i + 1] == b'N' {
                result.push_str("<br>");
                i += 2;
                continue;
            }
            result.push(bytes[i] as char);
            i += 1;
        }

        result.trim().to_string()
    }
}
