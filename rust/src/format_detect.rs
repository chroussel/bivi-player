//! Container format detection from first bytes.

use wasm_bindgen::prelude::*;

#[wasm_bindgen]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ContainerFormat {
    Mp4,
    Mkv,
    Unknown,
}

/// Detect container format from the first bytes of a file.
#[wasm_bindgen]
pub fn detect_format(data: &[u8]) -> ContainerFormat {
    if data.len() < 8 {
        return ContainerFormat::Unknown;
    }
    // EBML magic: 0x1A 0x45 0xDF 0xA3
    if data[0] == 0x1A && data[1] == 0x45 && data[2] == 0xDF && data[3] == 0xA3 {
        return ContainerFormat::Mkv;
    }
    // MP4: 'ftyp' at offset 4
    if data[4] == b'f' && data[5] == b't' && data[6] == b'y' && data[7] == b'p' {
        return ContainerFormat::Mp4;
    }
    ContainerFormat::Unknown
}

/// Detect from URL extension.
#[wasm_bindgen]
pub fn detect_format_from_url(url: &str) -> ContainerFormat {
    let lower = url.to_lowercase();
    if lower.ends_with(".mkv") || lower.contains(".mkv?") {
        ContainerFormat::Mkv
    } else if lower.ends_with(".mp4") || lower.ends_with(".m4v") || lower.contains(".mp4?") {
        ContainerFormat::Mp4
    } else {
        ContainerFormat::Unknown
    }
}
