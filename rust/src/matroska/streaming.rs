/// Streaming EBML/MKV reader — Read-only, no Seek required.
/// Inspired by ebml-iterable's pattern: internal buffer, forward-only parsing.
use std::io::Read;

use super::element_id::ElementId;

/// EBML variable-length integer
pub fn read_vint<R: Read>(r: &mut R) -> Option<(u64, usize)> {
    let mut first = [0u8; 1];
    r.read_exact(&mut first).ok()?;
    let b = first[0];
    if b == 0 { return None; }
    let len = b.leading_zeros() as usize + 1;
    let mut val = (b & (0xFF >> len)) as u64;
    for _ in 1..len {
        let mut byte = [0u8; 1];
        r.read_exact(&mut byte).ok()?;
        val = (val << 8) | byte[0] as u64;
    }
    Some((val, len))
}

/// Read EBML element ID (as raw u64)
pub fn read_element_id<R: Read>(r: &mut R) -> Option<(u64, usize)> {
    let mut first = [0u8; 1];
    r.read_exact(&mut first).ok()?;
    let b = first[0];
    if b == 0 { return None; }
    let len = b.leading_zeros() as usize + 1;
    let mut val = b as u64;
    for _ in 1..len {
        let mut byte = [0u8; 1];
        r.read_exact(&mut byte).ok()?;
        val = (val << 8) | byte[0] as u64;
    }
    Some((val, len))
}

/// Check if size is "unknown" (all VINT_DATA bits set)
fn is_unknown_size(val: u64, vint_len: usize) -> bool {
    val == ((1u64 << (7 * vint_len)) - 1)
}

/// Element header: ID + size
pub struct EbmlHeader {
    pub id: u64,
    pub size: Option<u64>, // None = unknown size (streaming)
    pub header_len: usize,
}

pub fn read_element_header<R: Read>(r: &mut R) -> Option<EbmlHeader> {
    let (id, id_len) = read_element_id(r)?;
    let (size_raw, size_len) = read_vint(r)?;
    let size = if is_unknown_size(size_raw, size_len) {
        None
    } else {
        Some(size_raw)
    };
    Some(EbmlHeader { id, size, header_len: id_len + size_len })
}

/// Skip N bytes from a Read source
pub fn skip<R: Read>(r: &mut R, n: u64) -> std::io::Result<()> {
    std::io::copy(&mut r.take(n), &mut std::io::sink())?;
    Ok(())
}

/// Read exact bytes
pub fn read_bytes<R: Read>(r: &mut R, n: usize) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf).ok()?;
    Some(buf)
}

/// Read unsigned int (big endian, variable length)
pub fn read_uint<R: Read>(r: &mut R, n: usize) -> Option<u64> {
    let bytes = read_bytes(r, n)?;
    let mut val = 0u64;
    for &b in &bytes {
        val = (val << 8) | b as u64;
    }
    Some(val)
}

/// Read float (4 or 8 bytes)
pub fn read_float<R: Read>(r: &mut R, n: usize) -> Option<f64> {
    match n {
        4 => {
            let bytes = read_bytes(r, 4)?;
            Some(f32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f64)
        }
        8 => {
            let bytes = read_bytes(r, 8)?;
            Some(f64::from_be_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3],
                bytes[4], bytes[5], bytes[6], bytes[7],
            ]))
        }
        _ => None,
    }
}

/// Read UTF-8 string
pub fn read_string<R: Read>(r: &mut R, n: usize) -> Option<String> {
    let bytes = read_bytes(r, n)?;
    String::from_utf8(bytes).ok()
}

// ── MKV element IDs ──

pub const EBML_ID: u64 = 0x1A45DFA3;
pub const SEGMENT_ID: u64 = 0x18538067;
pub const SEGMENT_INFO_ID: u64 = 0x1549A966;
pub const TRACKS_ID: u64 = 0x1654AE6B;
pub const TRACK_ENTRY_ID: u64 = 0xAE;
pub const TRACK_NUMBER_ID: u64 = 0xD7;
pub const TRACK_TYPE_ID: u64 = 0x83;
pub const CODEC_ID_ID: u64 = 0x86;
pub const CODEC_PRIVATE_ID: u64 = 0x63A2;
pub const LANGUAGE_ID: u64 = 0x22B59C;
pub const NAME_ID: u64 = 0x536E;
pub const VIDEO_ID: u64 = 0xE0;
pub const AUDIO_ID: u64 = 0xE1;
pub const PIXEL_WIDTH_ID: u64 = 0xB0;
pub const PIXEL_HEIGHT_ID: u64 = 0xBA;
pub const SAMPLING_FREQ_ID: u64 = 0xB5;
pub const CHANNELS_ID: u64 = 0x9F;
pub const TIMECODE_SCALE_ID: u64 = 0x2AD7B1;
pub const DURATION_ID: u64 = 0x4489;
pub const CLUSTER_ID: u64 = 0x1F43B675;
pub const CLUSTER_TIMESTAMP_ID: u64 = 0xE7;
pub const SIMPLE_BLOCK_ID: u64 = 0xA3;
pub const BLOCK_GROUP_ID: u64 = 0xA0;
pub const BLOCK_ID: u64 = 0xA1;
pub const BLOCK_DURATION_ID: u64 = 0x9B;
pub const CUES_ID: u64 = 0x1C53BB6B;

/// Master elements (containers) — we recurse into these
pub fn is_master(id: u64) -> bool {
    matches!(
        id,
        EBML_ID | SEGMENT_ID | SEGMENT_INFO_ID | TRACKS_ID | TRACK_ENTRY_ID
            | VIDEO_ID | AUDIO_ID | CLUSTER_ID | BLOCK_GROUP_ID | CUES_ID
    )
}

// ── Streaming MKV Parser ──

pub struct TrackInfo {
    pub number: u64,
    pub track_type: u64, // 1=video, 2=audio, 17=subtitle
    pub codec_id: String,
    pub codec_private: Vec<u8>,
    pub language: String,
    pub name: String,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub sample_rate: f64,
    pub channels: u64,
}

pub struct MkvHeader {
    pub timecode_scale: u64, // ns per tick, default 1_000_000
    pub duration: f64,       // in timecode_scale units
    pub tracks: Vec<TrackInfo>,
}

pub struct MkvFrame {
    pub track: u64,
    pub timestamp_ns: i64,
    pub data: Vec<u8>,
    pub is_keyframe: bool,
    pub duration_ns: Option<i64>,
}

/// Parse EBML header + SegmentInfo + Tracks from a Read source.
/// Stops at the first Cluster (ready for frame iteration).
pub fn parse_mkv_header<R: Read>(r: &mut R) -> Option<MkvHeader> {
    // Skip EBML header
    let hdr = read_element_header(r)?;
    if hdr.id != EBML_ID { return None; }
    skip(r, hdr.size?).ok()?;

    // Expect Segment (usually unknown size)
    let seg = read_element_header(r)?;
    if seg.id != SEGMENT_ID { return None; }

    let mut timecode_scale = 1_000_000u64;
    let mut duration = 0.0f64;
    let mut tracks = Vec::new();

    // Read Segment children until we hit a Cluster
    loop {
        let el = read_element_header(r);
        let el = match el {
            Some(e) => e,
            None => break,
        };

        if el.id == CLUSTER_ID {
            // We've reached clusters — header parsing done
            break;
        }

        let size = match el.size {
            Some(s) => s,
            None => break,
        };

        match el.id {
            SEGMENT_INFO_ID => {
                let data = read_bytes(r, size as usize)?;
                let mut cursor = &data[..];
                while !cursor.is_empty() {
                    let child = read_element_header(&mut cursor)?;
                    let csz = child.size? as usize;
                    match child.id {
                        TIMECODE_SCALE_ID => timecode_scale = read_uint(&mut cursor, csz)?,
                        DURATION_ID => duration = read_float(&mut cursor, csz)?,
                        _ => { skip(&mut cursor, csz as u64).ok()?; }
                    }
                }
            }
            TRACKS_ID => {
                let data = read_bytes(r, size as usize)?;
                let mut cursor = &data[..];
                while !cursor.is_empty() {
                    let entry = read_element_header(&mut cursor)?;
                    if entry.id != TRACK_ENTRY_ID {
                        skip(&mut cursor, entry.size? as u64).ok()?;
                        continue;
                    }
                    let entry_data = read_bytes(&mut cursor, entry.size? as usize)?;
                    tracks.push(parse_track_entry(&entry_data)?);
                }
            }
            _ => {
                // Skip unknown top-level elements (Cues, SeekHead, etc.)
                skip(r, size).ok()?;
            }
        }
    }

    Some(MkvHeader { timecode_scale, duration, tracks })
}

fn parse_track_entry(data: &[u8]) -> Option<TrackInfo> {
    let mut cursor = &data[..];
    let mut info = TrackInfo {
        number: 0, track_type: 0, codec_id: String::new(),
        codec_private: Vec::new(), language: "und".to_string(),
        name: String::new(), pixel_width: 0, pixel_height: 0,
        sample_rate: 0.0, channels: 0,
    };

    while !cursor.is_empty() {
        let el = read_element_header(&mut cursor)?;
        let sz = el.size? as usize;
        match el.id {
            TRACK_NUMBER_ID => info.number = read_uint(&mut cursor, sz)?,
            TRACK_TYPE_ID => info.track_type = read_uint(&mut cursor, sz)?,
            CODEC_ID_ID => info.codec_id = read_string(&mut cursor, sz)?,
            CODEC_PRIVATE_ID => info.codec_private = read_bytes(&mut cursor, sz)?,
            LANGUAGE_ID => info.language = read_string(&mut cursor, sz)?,
            NAME_ID => info.name = read_string(&mut cursor, sz)?,
            VIDEO_ID => {
                let vdata = read_bytes(&mut cursor, sz)?;
                let mut vc = &vdata[..];
                while !vc.is_empty() {
                    let vel = read_element_header(&mut vc)?;
                    let vsz = vel.size? as usize;
                    match vel.id {
                        PIXEL_WIDTH_ID => info.pixel_width = read_uint(&mut vc, vsz)? as u32,
                        PIXEL_HEIGHT_ID => info.pixel_height = read_uint(&mut vc, vsz)? as u32,
                        _ => { skip(&mut vc, vsz as u64).ok()?; }
                    }
                }
            }
            AUDIO_ID => {
                let adata = read_bytes(&mut cursor, sz)?;
                let mut ac = &adata[..];
                while !ac.is_empty() {
                    let ael = read_element_header(&mut ac)?;
                    let asz = ael.size? as usize;
                    match ael.id {
                        SAMPLING_FREQ_ID => info.sample_rate = read_float(&mut ac, asz)?,
                        CHANNELS_ID => info.channels = read_uint(&mut ac, asz)?,
                        _ => { skip(&mut ac, asz as u64).ok()?; }
                    }
                }
            }
            _ => { skip(&mut cursor, sz as u64).ok()?; }
        }
    }
    Some(info)
}

/// Parse a SimpleBlock header from raw block data.
/// Returns (track_number, relative_timestamp_ms, keyframe, frame_data)
pub fn parse_simple_block(data: &[u8]) -> Option<(u64, i16, bool, &[u8])> {
    let mut cursor = &data[..];
    let (track, track_len) = read_vint(&mut cursor)?;
    let consumed = data.len() - cursor.len();
    if data.len() < consumed + 3 { return None; }
    let ts = ((data[consumed] as i16) << 8) | data[consumed + 1] as i16;
    let flags = data[consumed + 2];
    let keyframe = (flags & 0x80) != 0;
    let frame_data = &data[consumed + 3..];
    Some((track, ts, keyframe, frame_data))
}

/// Iterator over MKV frames from a streaming Read source.
/// Call after parse_mkv_header (which consumes up to the first Cluster).
pub struct MkvFrameIter<R: Read> {
    reader: R,
    timecode_scale: u64,
    cluster_timestamp: u64,
    in_cluster: bool,
    pending_block: Option<(u64, i16, bool, Vec<u8>)>,
    pending_duration: Option<u64>,
    bytes_consumed: u64,
}

impl<R: Read> MkvFrameIter<R> {
    pub fn new(reader: R, timecode_scale: u64) -> Self {
        MkvFrameIter {
            reader,
            timecode_scale,
            cluster_timestamp: 0,
            in_cluster: true,
            pending_block: None,
            pending_duration: None,
            bytes_consumed: 0,
        }
    }

    pub fn set_cluster_timestamp(&mut self, ts: u64) {
        self.cluster_timestamp = ts;
    }

    pub fn cluster_timestamp(&self) -> u64 {
        self.cluster_timestamp
    }

    pub fn bytes_consumed(&self) -> u64 {
        self.bytes_consumed
    }

    pub fn next_frame(&mut self) -> Option<MkvFrame> {
        loop {
            let hdr = read_element_header(&mut self.reader)?;
            self.bytes_consumed += hdr.header_len as u64;
            let size = match hdr.size {
                Some(s) => s,
                None => {
                    // Unknown-size element (new Cluster)
                    if hdr.id == CLUSTER_ID {
                        self.in_cluster = true;
                        continue;
                    }
                    return None;
                }
            };

            match hdr.id {
                CLUSTER_ID => {
                    self.in_cluster = true;
                    // Cluster with known size — children follow
                    continue;
                }
                CLUSTER_TIMESTAMP_ID => {
                    self.cluster_timestamp = read_uint(&mut self.reader, size as usize)?;
                    self.bytes_consumed += size;
                    continue;
                }
                SIMPLE_BLOCK_ID => {
                    let data = read_bytes(&mut self.reader, size as usize)?;
                    self.bytes_consumed += size;
                    let (track, rel_ts, keyframe, frame_data) = parse_simple_block(&data)?;
                    let abs_ts = self.cluster_timestamp as i64 + rel_ts as i64;
                    let timestamp_ns = abs_ts * self.timecode_scale as i64;
                    return Some(MkvFrame {
                        track,
                        timestamp_ns,
                        data: frame_data.to_vec(),
                        is_keyframe: keyframe,
                        duration_ns: None,
                    });
                }
                BLOCK_GROUP_ID => {
                    let group_data = read_bytes(&mut self.reader, size as usize)?;
                    self.bytes_consumed += size;
                    let mut gc = &group_data[..];
                    let mut block_data = None;
                    let mut block_dur = None;
                    while !gc.is_empty() {
                        let gel = read_element_header(&mut gc)?;
                        let gsz = gel.size? as usize;
                        match gel.id {
                            BLOCK_ID => block_data = Some(read_bytes(&mut gc, gsz)?),
                            BLOCK_DURATION_ID => block_dur = Some(read_uint(&mut gc, gsz)?),
                            _ => { skip(&mut gc, gsz as u64).ok()?; }
                        }
                    }
                    if let Some(data) = block_data {
                        let (track, rel_ts, keyframe, frame_data) = parse_simple_block(&data)?;
                        let abs_ts = self.cluster_timestamp as i64 + rel_ts as i64;
                        let timestamp_ns = abs_ts * self.timecode_scale as i64;
                        let dur_ns = block_dur.map(|d| d as i64 * self.timecode_scale as i64);
                        return Some(MkvFrame {
                            track,
                            timestamp_ns,
                            data: frame_data.to_vec(),
                            is_keyframe: keyframe,
                            duration_ns: dur_ns,
                        });
                    }
                    continue;
                }
                _ => {
                    skip(&mut self.reader, size).ok()?;
                    self.bytes_consumed += size;
                    continue;
                }
            }
        }
    }
}
