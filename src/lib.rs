use bytes::Buf;
use wasm_bindgen::prelude::*;

// ── Helpers ──

const fn fourcc(s: &[u8; 4]) -> u32 {
    ((s[0] as u32) << 24) | ((s[1] as u32) << 16) | ((s[2] as u32) << 8) | (s[3] as u32)
}

const MOOV: u32 = fourcc(b"moov");
const TRAK: u32 = fourcc(b"trak");
const MDIA: u32 = fourcc(b"mdia");
const MINF: u32 = fourcc(b"minf");
const STBL: u32 = fourcc(b"stbl");
const STSD: u32 = fourcc(b"stsd");
const STTS: u32 = fourcc(b"stts");
const STSC: u32 = fourcc(b"stsc");
const STSZ: u32 = fourcc(b"stsz");
const STCO: u32 = fourcc(b"stco");
const CO64: u32 = fourcc(b"co64");
const STSS: u32 = fourcc(b"stss");
const CTTS: u32 = fourcc(b"ctts");
const MDHD: u32 = fourcc(b"mdhd");
const HDLR: u32 = fourcc(b"hdlr");
const HEV1: u32 = fourcc(b"hev1");
const HVC1: u32 = fourcc(b"hvc1");
const HVCC: u32 = fourcc(b"hvcC");
const VIDE: u32 = fourcc(b"vide");

struct BoxHeader {
    box_type: u32,
    size: u64,
    header_size: u64,
}

fn read_box_header(buf: &mut &[u8]) -> Option<BoxHeader> {
    if buf.remaining() < 8 {
        return None;
    }
    let size32 = buf.get_u32();
    let box_type = buf.get_u32();
    let (size, header_size) = if size32 == 1 {
        if buf.remaining() < 8 {
            return None;
        }
        (buf.get_u64(), 16u64)
    } else {
        (size32 as u64, 8u64)
    };
    Some(BoxHeader {
        box_type,
        size,
        header_size,
    })
}

/// Read version + flags from a FullBox header, returns (version, flags).
fn read_fullbox(buf: &mut &[u8]) -> Option<(u8, u32)> {
    if buf.remaining() < 4 {
        return None;
    }
    let version = buf.get_u8();
    // flags is 3 bytes big-endian
    let flags = ((buf.get_u8() as u32) << 16) | ((buf.get_u8() as u32) << 8) | buf.get_u8() as u32;
    Some((version, flags))
}

// ── Track structures ──

struct StscEntry {
    first_chunk: u32,
    samples_per_chunk: u32,
}

struct VideoTrack {
    timescale: u32,
    duration: u64,
    width: u16,
    height: u16,
    codec_fourcc: u32,
    hvcc_raw: Vec<u8>,

    sample_sizes: Vec<u32>,
    chunk_offsets: Vec<u64>,
    stsc_entries: Vec<StscEntry>,
    sample_durations: Vec<u32>,
    composition_offsets: Vec<i32>,
    sync_samples: Option<Vec<u32>>,
}

impl VideoTrack {
    fn sample_count(&self) -> usize {
        self.sample_sizes.len()
    }

    fn build_sample_offsets(&self) -> Vec<u64> {
        let count = self.sample_count();
        let mut offsets = vec![0u64; count];
        let total_chunks = self.chunk_offsets.len() as u32;
        let mut sample_idx = 0usize;

        for (i, entry) in self.stsc_entries.iter().enumerate() {
            let next_first = if i + 1 < self.stsc_entries.len() {
                self.stsc_entries[i + 1].first_chunk
            } else {
                total_chunks + 1
            };

            for chunk_num in entry.first_chunk..next_first {
                let chunk_offset = self.chunk_offsets[(chunk_num - 1) as usize];
                let mut offset = chunk_offset;
                for _ in 0..entry.samples_per_chunk {
                    if sample_idx >= count {
                        break;
                    }
                    offsets[sample_idx] = offset;
                    offset += self.sample_sizes[sample_idx] as u64;
                    sample_idx += 1;
                }
            }
        }
        offsets
    }

    fn build_dts(&self) -> Vec<u64> {
        let count = self.sample_count();
        let mut dts_values = Vec::with_capacity(count);
        let mut dts = 0u64;
        for (i, &dur) in self.sample_durations.iter().enumerate() {
            if i >= count {
                break;
            }
            dts_values.push(dts);
            dts += dur as u64;
        }
        if let Some(&last) = self.sample_durations.last() {
            while dts_values.len() < count {
                dts_values.push(dts);
                dts += last as u64;
            }
        }
        dts_values
    }

    fn is_sync(&self, sample_index_0based: usize) -> bool {
        match &self.sync_samples {
            None => true,
            Some(syncs) => syncs.contains(&((sample_index_0based + 1) as u32)),
        }
    }
}

// ── Box scanning ──

/// Scan a region of the file for top-level boxes. Returns (type, content_start, box_end) as byte offsets.
fn find_boxes(data: &[u8], start: usize, end: usize) -> Vec<(u32, usize, usize)> {
    let mut boxes = Vec::new();
    let mut pos = start;

    while pos + 8 <= end {
        let mut buf: &[u8] = &data[pos..end];
        if let Some(hdr) = read_box_header(&mut buf) {
            let box_end = pos + hdr.size as usize;
            if hdr.size < 8 || box_end > end {
                break;
            }
            let content_start = pos + hdr.header_size as usize;
            boxes.push((hdr.box_type, content_start, box_end));
            pos = box_end;
        } else {
            break;
        }
    }
    boxes
}

fn find_box(data: &[u8], start: usize, end: usize, box_type: u32) -> Option<(usize, usize)> {
    find_boxes(data, start, end)
        .into_iter()
        .find(|(t, _, _)| *t == box_type)
        .map(|(_, s, e)| (s, e))
}

// ── Box parsers ──
// Each parser receives the box *content* as a slice (after the box header).

fn parse_mdhd(content: &[u8]) -> Option<(u32, u64)> {
    let mut buf = content;
    let (version, _) = read_fullbox(&mut buf)?;
    if version == 1 {
        if buf.remaining() < 20 {
            return None;
        }
        buf.advance(16); // creation_time(8) + modification_time(8)
        let timescale = buf.get_u32();
        let duration = buf.get_u64();
        Some((timescale, duration))
    } else {
        if buf.remaining() < 12 {
            return None;
        }
        buf.advance(8); // creation_time(4) + modification_time(4)
        let timescale = buf.get_u32();
        let duration = buf.get_u32() as u64;
        Some((timescale, duration))
    }
}

fn parse_hdlr(content: &[u8]) -> Option<u32> {
    let mut buf = content;
    read_fullbox(&mut buf)?;
    if buf.remaining() < 8 {
        return None;
    }
    buf.advance(4); // pre_defined
    Some(buf.get_u32()) // handler_type
}

fn parse_stsd_hevc(content: &[u8], file_data: &[u8], stsd_file_start: usize) -> Option<(u32, u16, u16, Vec<u8>)> {
    let mut buf = content;
    read_fullbox(&mut buf)?;
    if buf.remaining() < 4 {
        return None;
    }
    let entry_count = buf.get_u32();
    if entry_count == 0 {
        return None;
    }

    // Read the sample entry box header
    let before_entry = buf.remaining();
    let entry_hdr = read_box_header(&mut buf)?;
    let codec_fourcc = entry_hdr.box_type;
    if codec_fourcc != HEV1 && codec_fourcc != HVC1 {
        return None;
    }

    // VisualSampleEntry: 78 bytes of fixed fields after box header
    if buf.remaining() < 78 {
        return None;
    }
    buf.advance(6 + 2); // reserved(6) + data_reference_index(2)
    buf.advance(2 + 2); // pre_defined + reserved
    buf.advance(12); // pre_defined(3 * u32)
    let width = buf.get_u16();
    let height = buf.get_u16();
    buf.advance(4 + 4 + 4); // horizresolution + vertresolution + reserved
    buf.advance(2); // frame_count
    buf.advance(32); // compressorname
    buf.advance(2 + 2); // depth + pre_defined

    // Scan sub-boxes for hvcC within the sample entry
    // Calculate file-level positions for the remaining data
    let consumed = content.len() - before_entry + entry_hdr.header_size as usize + 78;
    let sub_start = stsd_file_start + consumed;
    let entry_end = stsd_file_start + (content.len() - before_entry) + entry_hdr.size as usize;

    if let Some((hvcc_start, hvcc_end)) = find_box(file_data, sub_start, entry_end, HVCC) {
        let hvcc_raw = file_data[hvcc_start..hvcc_end].to_vec();
        Some((codec_fourcc, width, height, hvcc_raw))
    } else {
        None
    }
}

fn parse_stts(content: &[u8]) -> Option<Vec<u32>> {
    let mut buf = content;
    read_fullbox(&mut buf)?;
    if buf.remaining() < 4 {
        return None;
    }
    let entry_count = buf.get_u32() as usize;
    let mut durations = Vec::new();
    for _ in 0..entry_count {
        if buf.remaining() < 8 {
            return None;
        }
        let count = buf.get_u32();
        let delta = buf.get_u32();
        for _ in 0..count {
            durations.push(delta);
        }
    }
    Some(durations)
}

fn parse_stsc(content: &[u8]) -> Option<Vec<StscEntry>> {
    let mut buf = content;
    read_fullbox(&mut buf)?;
    if buf.remaining() < 4 {
        return None;
    }
    let entry_count = buf.get_u32() as usize;
    let mut entries = Vec::with_capacity(entry_count);
    for _ in 0..entry_count {
        if buf.remaining() < 12 {
            return None;
        }
        let first_chunk = buf.get_u32();
        let samples_per_chunk = buf.get_u32();
        buf.advance(4); // sample_description_index
        entries.push(StscEntry {
            first_chunk,
            samples_per_chunk,
        });
    }
    Some(entries)
}

fn parse_stsz(content: &[u8]) -> Option<Vec<u32>> {
    let mut buf = content;
    read_fullbox(&mut buf)?;
    if buf.remaining() < 8 {
        return None;
    }
    let default_size = buf.get_u32();
    let count = buf.get_u32() as usize;
    let mut sizes = Vec::with_capacity(count);
    if default_size != 0 {
        sizes.resize(count, default_size);
    } else {
        for _ in 0..count {
            if buf.remaining() < 4 {
                return None;
            }
            sizes.push(buf.get_u32());
        }
    }
    Some(sizes)
}

fn parse_stco(content: &[u8]) -> Option<Vec<u64>> {
    let mut buf = content;
    read_fullbox(&mut buf)?;
    if buf.remaining() < 4 {
        return None;
    }
    let count = buf.get_u32() as usize;
    let mut offsets = Vec::with_capacity(count);
    for _ in 0..count {
        if buf.remaining() < 4 {
            return None;
        }
        offsets.push(buf.get_u32() as u64);
    }
    Some(offsets)
}

fn parse_co64(content: &[u8]) -> Option<Vec<u64>> {
    let mut buf = content;
    read_fullbox(&mut buf)?;
    if buf.remaining() < 4 {
        return None;
    }
    let count = buf.get_u32() as usize;
    let mut offsets = Vec::with_capacity(count);
    for _ in 0..count {
        if buf.remaining() < 8 {
            return None;
        }
        offsets.push(buf.get_u64());
    }
    Some(offsets)
}

fn parse_stss(content: &[u8]) -> Option<Vec<u32>> {
    let mut buf = content;
    read_fullbox(&mut buf)?;
    if buf.remaining() < 4 {
        return None;
    }
    let count = buf.get_u32() as usize;
    let mut samples = Vec::with_capacity(count);
    for _ in 0..count {
        if buf.remaining() < 4 {
            return None;
        }
        samples.push(buf.get_u32());
    }
    Some(samples)
}

fn parse_ctts(content: &[u8]) -> Option<Vec<i32>> {
    let mut buf = content;
    let (version, _) = read_fullbox(&mut buf)?;
    if buf.remaining() < 4 {
        return None;
    }
    let entry_count = buf.get_u32() as usize;
    let mut offsets = Vec::new();
    for _ in 0..entry_count {
        if buf.remaining() < 8 {
            return None;
        }
        let count = buf.get_u32();
        let offset = if version == 0 {
            buf.get_u32() as i32
        } else {
            buf.get_i32()
        };
        for _ in 0..count {
            offsets.push(offset);
        }
    }
    Some(offsets)
}

// ── Track / file parsing ──

fn parse_trak(data: &[u8], trak_start: usize, trak_end: usize) -> Option<VideoTrack> {
    let (mdia_s, mdia_e) = find_box(data, trak_start, trak_end, MDIA)?;
    let mdia_boxes = find_boxes(data, mdia_s, mdia_e);

    let &(_, hdlr_s, hdlr_e) = mdia_boxes.iter().find(|(t, _, _)| *t == HDLR)?;
    if parse_hdlr(&data[hdlr_s..hdlr_e])? != VIDE {
        return None;
    }

    let &(_, mdhd_s, mdhd_e) = mdia_boxes.iter().find(|(t, _, _)| *t == MDHD)?;
    let (timescale, duration) = parse_mdhd(&data[mdhd_s..mdhd_e])?;

    let (minf_s, minf_e) = find_box(data, mdia_s, mdia_e, MINF)?;
    let (stbl_s, stbl_e) = find_box(data, minf_s, minf_e, STBL)?;
    let stbl_boxes = find_boxes(data, stbl_s, stbl_e);

    let get = |bt: u32| -> Option<(usize, usize)> {
        stbl_boxes
            .iter()
            .find(|(t, _, _)| *t == bt)
            .map(|(_, s, e)| (*s, *e))
    };

    let (stsd_s, stsd_e) = get(STSD)?;
    let (codec_fourcc, width, height, hvcc_raw) =
        parse_stsd_hevc(&data[stsd_s..stsd_e], data, stsd_s)?;

    let (stts_s, stts_e) = get(STTS)?;
    let sample_durations = parse_stts(&data[stts_s..stts_e])?;

    let (stsc_s, stsc_e) = get(STSC)?;
    let stsc_entries = parse_stsc(&data[stsc_s..stsc_e])?;

    let (stsz_s, stsz_e) = get(STSZ)?;
    let sample_sizes = parse_stsz(&data[stsz_s..stsz_e])?;

    let chunk_offsets = if let Some((s, e)) = get(STCO) {
        parse_stco(&data[s..e])?
    } else if let Some((s, e)) = get(CO64) {
        parse_co64(&data[s..e])?
    } else {
        return None;
    };

    let sync_samples = get(STSS).and_then(|(s, e)| parse_stss(&data[s..e]));

    let composition_offsets = get(CTTS)
        .and_then(|(s, e)| parse_ctts(&data[s..e]))
        .unwrap_or_default();

    Some(VideoTrack {
        timescale,
        duration,
        width,
        height,
        codec_fourcc,
        hvcc_raw,
        sample_sizes,
        chunk_offsets,
        stsc_entries,
        sample_durations,
        composition_offsets,
        sync_samples,
    })
}

fn parse_mp4(data: &[u8]) -> Result<VideoTrack, String> {
    let (moov_s, moov_e) = find_box(data, 0, data.len(), MOOV).ok_or("no moov box found")?;

    for (box_type, start, end) in find_boxes(data, moov_s, moov_e) {
        if box_type == TRAK {
            if let Some(track) = parse_trak(data, start, end) {
                return Ok(track);
            }
        }
    }
    Err("no HEVC video track found".into())
}

// ── Codec string ──

fn build_codec_string(hvcc: &[u8], codec_fourcc: u32) -> String {
    if hvcc.len() < 13 {
        return String::from("hev1.1.6.L93.B0");
    }

    let prefix = if codec_fourcc == HVC1 {
        "hvc1"
    } else {
        "hev1"
    };

    let mut buf = &hvcc[1..];
    let byte1 = buf.get_u8();
    let profile_space = (byte1 >> 6) & 0x03;
    let tier_flag = (byte1 >> 5) & 0x01;
    let profile_idc = byte1 & 0x1F;

    let compat = buf.get_u32();
    let mut constraint_bytes = [0u8; 6];
    constraint_bytes.copy_from_slice(&buf[..6]);
    buf.advance(6);
    let level_idc = buf.get_u8();

    let profile_space_str = match profile_space {
        1 => "A",
        2 => "B",
        3 => "C",
        _ => "",
    };

    let tier_str = if tier_flag == 1 { "H" } else { "L" };

    // Constraint string: hex bytes, trailing zeros removed
    let last_nonzero = constraint_bytes
        .iter()
        .rposition(|&b| b != 0)
        .unwrap_or(0);
    let constraint_str: String = constraint_bytes[..=last_nonzero]
        .iter()
        .map(|b| format!("{:X}", b))
        .collect::<Vec<_>>()
        .join(".");

    let constraint_suffix = if constraint_str.is_empty() {
        String::new()
    } else {
        format!(".{}", constraint_str)
    };

    // Reverse bits of compat flags for the codec string
    let compat_rev = compat.reverse_bits();

    format!(
        "{}.{}{}.{:X}.{}{}{}",
        prefix, profile_space_str, profile_idc, compat_rev, tier_str, level_idc, constraint_suffix
    )
}

// ── WASM API ──

#[wasm_bindgen]
pub struct Sample {
    is_sync: bool,
    timestamp_us: f64,
    duration_us: f64,
    data: Vec<u8>,
}

#[wasm_bindgen]
impl Sample {
    #[wasm_bindgen(getter)]
    pub fn is_sync(&self) -> bool {
        self.is_sync
    }
    #[wasm_bindgen(getter)]
    pub fn timestamp_us(&self) -> f64 {
        self.timestamp_us
    }
    #[wasm_bindgen(getter)]
    pub fn duration_us(&self) -> f64 {
        self.duration_us
    }
    #[wasm_bindgen(getter)]
    pub fn data(&self) -> Vec<u8> {
        self.data.clone()
    }
}

#[wasm_bindgen]
pub struct Demuxer {
    data: Vec<u8>,
    track: VideoTrack,
    sample_offsets: Vec<u64>,
    dts_values: Vec<u64>,
    pts_offset: f64, // subtracted from all PTS to normalize to 0
}

#[wasm_bindgen]
impl Demuxer {
    #[wasm_bindgen(constructor)]
    pub fn new(data: Vec<u8>) -> Result<Demuxer, JsValue> {
        console_error_panic_hook::set_once();
        let track =
            parse_mp4(&data).map_err(|e| JsValue::from_str(&format!("MP4 parse error: {}", e)))?;
        let sample_offsets = track.build_sample_offsets();
        let dts_values = track.build_dts();

        // Find minimum PTS to normalize (handles negative DTS from edit lists)
        let count = track.sample_count();
        let mut min_pts = f64::MAX;
        for i in 0..count {
            let dts = dts_values[i] as f64;
            let cts = if i < track.composition_offsets.len() {
                track.composition_offsets[i] as f64
            } else {
                0.0
            };
            let pts = dts + cts;
            if pts < min_pts {
                min_pts = pts;
            }
        }
        if min_pts == f64::MAX {
            min_pts = 0.0;
        }

        Ok(Demuxer {
            data,
            track,
            sample_offsets,
            dts_values,
            pts_offset: min_pts,
        })
    }

    pub fn width(&self) -> u32 {
        self.track.width as u32
    }

    pub fn height(&self) -> u32 {
        self.track.height as u32
    }

    pub fn sample_count(&self) -> u32 {
        self.track.sample_count() as u32
    }

    pub fn duration_ms(&self) -> f64 {
        if self.track.timescale == 0 {
            return 0.0;
        }
        (self.track.duration as f64 / self.track.timescale as f64) * 1000.0
    }

    pub fn codec_string(&self) -> String {
        build_codec_string(&self.track.hvcc_raw, self.track.codec_fourcc)
    }

    pub fn codec_description(&self) -> Vec<u8> {
        self.track.hvcc_raw.clone()
    }

    /// NAL length size in bytes (typically 4), from hvcC length_size_minus_one + 1.
    pub fn nal_length_size(&self) -> u8 {
        if self.track.hvcc_raw.len() > 21 {
            (self.track.hvcc_raw[21] & 0x03) + 1
        } else {
            4
        }
    }

    pub fn read_sample(&self, index: u32) -> Option<Sample> {
        let i = index as usize;
        if i >= self.track.sample_count() {
            return None;
        }

        let offset = self.sample_offsets[i] as usize;
        let size = self.track.sample_sizes[i] as usize;
        if offset + size > self.data.len() {
            return None;
        }

        let sample_data = self.data[offset..offset + size].to_vec();

        let timescale = self.track.timescale as f64;
        let dts = self.dts_values[i] as f64;
        let cts_offset = if i < self.track.composition_offsets.len() {
            self.track.composition_offsets[i] as f64
        } else {
            0.0
        };
        let pts = dts + cts_offset - self.pts_offset;
        let timestamp_us = (pts / timescale) * 1_000_000.0;

        let duration = if i < self.track.sample_durations.len() {
            self.track.sample_durations[i]
        } else {
            self.track.sample_durations.last().copied().unwrap_or(1)
        };
        let duration_us = (duration as f64 / timescale) * 1_000_000.0;

        Some(Sample {
            is_sync: self.track.is_sync(i),
            timestamp_us,
            duration_us,
            data: sample_data,
        })
    }
}
