use bytes::Buf;
use wasm_bindgen::prelude::*;

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
const SOUN: u32 = fourcc(b"soun");
const MP4A: u32 = fourcc(b"mp4a");
const ESDS: u32 = fourcc(b"esds");

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

fn read_fullbox(buf: &mut &[u8]) -> Option<(u8, u32)> {
    if buf.remaining() < 4 {
        return None;
    }
    let version = buf.get_u8();
    let flags =
        ((buf.get_u8() as u32) << 16) | ((buf.get_u8() as u32) << 8) | buf.get_u8() as u32;
    Some((version, flags))
}

struct StscEntry {
    first_chunk: u32,
    samples_per_chunk: u32,
}

pub(crate) struct VideoTrack {
    pub timescale: u32,
    pub duration: u64,
    pub width: u16,
    pub height: u16,
    pub codec_fourcc: u32,
    pub hvcc_raw: Vec<u8>,
    pub sample_sizes: Vec<u32>,
    pub chunk_offsets: Vec<u64>,
    stsc_entries: Vec<StscEntry>,
    pub sample_durations: Vec<u32>,
    pub composition_offsets: Vec<i32>,
    sync_samples: Option<Vec<u32>>,
}

impl VideoTrack {
    pub fn sample_count(&self) -> usize {
        self.sample_sizes.len()
    }

    pub fn build_sample_offsets(&self) -> Vec<u64> {
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

    pub fn build_dts(&self) -> Vec<u64> {
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

    pub fn is_sync(&self, sample_index_0based: usize) -> bool {
        match &self.sync_samples {
            None => true,
            Some(syncs) => syncs.contains(&((sample_index_0based + 1) as u32)),
        }
    }
}

pub(crate) struct AudioTrack {
    pub timescale: u32,
    pub duration: u64,
    pub sample_rate: u32,
    pub channel_count: u16,
    pub codec_config: Vec<u8>, // AudioSpecificConfig from esds
    pub sample_sizes: Vec<u32>,
    pub chunk_offsets: Vec<u64>,
    stsc_entries: Vec<StscEntry>,
    pub sample_durations: Vec<u32>,
}

impl AudioTrack {
    pub fn sample_count(&self) -> usize { self.sample_sizes.len() }

    pub fn build_sample_offsets(&self) -> Vec<u64> {
        let count = self.sample_count();
        let mut offsets = vec![0u64; count];
        let total_chunks = self.chunk_offsets.len() as u32;
        let mut sample_idx = 0usize;
        for (i, entry) in self.stsc_entries.iter().enumerate() {
            let next_first = if i + 1 < self.stsc_entries.len() {
                self.stsc_entries[i + 1].first_chunk
            } else { total_chunks + 1 };
            for chunk_num in entry.first_chunk..next_first {
                let chunk_offset = self.chunk_offsets[(chunk_num - 1) as usize];
                let mut offset = chunk_offset;
                for _ in 0..entry.samples_per_chunk {
                    if sample_idx >= count { break; }
                    offsets[sample_idx] = offset;
                    offset += self.sample_sizes[sample_idx] as u64;
                    sample_idx += 1;
                }
            }
        }
        offsets
    }

    pub fn build_dts(&self) -> Vec<u64> {
        let count = self.sample_count();
        let mut dts_values = Vec::with_capacity(count);
        let mut dts = 0u64;
        for (i, &dur) in self.sample_durations.iter().enumerate() {
            if i >= count { break; }
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
}

// ── Box scanning ──

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

fn parse_mdhd(content: &[u8]) -> Option<(u32, u64)> {
    let mut buf = content;
    let (version, _) = read_fullbox(&mut buf)?;
    if version == 1 {
        if buf.remaining() < 20 {
            return None;
        }
        buf.advance(16);
        let timescale = buf.get_u32();
        let duration = buf.get_u64();
        Some((timescale, duration))
    } else {
        if buf.remaining() < 12 {
            return None;
        }
        buf.advance(8);
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
    buf.advance(4);
    Some(buf.get_u32())
}

fn parse_stsd_hevc(
    content: &[u8],
    file_data: &[u8],
    stsd_file_start: usize,
) -> Option<(u32, u16, u16, Vec<u8>)> {
    let mut buf = content;
    read_fullbox(&mut buf)?;
    if buf.remaining() < 4 {
        return None;
    }
    let entry_count = buf.get_u32();
    if entry_count == 0 {
        return None;
    }
    let before_entry = buf.remaining();
    let entry_hdr = read_box_header(&mut buf)?;
    let codec_fourcc = entry_hdr.box_type;
    if codec_fourcc != HEV1 && codec_fourcc != HVC1 {
        return None;
    }
    if buf.remaining() < 78 {
        return None;
    }
    buf.advance(6 + 2);
    buf.advance(2 + 2);
    buf.advance(12);
    let width = buf.get_u16();
    let height = buf.get_u16();
    buf.advance(4 + 4 + 4);
    buf.advance(2);
    buf.advance(32);
    buf.advance(2 + 2);
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

/// Parse audio sample description — returns (sample_rate, channel_count, codec_config)
fn parse_stsd_aac(
    content: &[u8],
    file_data: &[u8],
    stsd_file_start: usize,
) -> Option<(u32, u16, Vec<u8>)> {
    let mut buf = content;
    read_fullbox(&mut buf)?;
    if buf.remaining() < 4 { return None; }
    let entry_count = buf.get_u32();
    if entry_count == 0 { return None; }
    let before_entry = buf.remaining();
    let entry_hdr = read_box_header(&mut buf)?;
    if entry_hdr.box_type != MP4A { return None; }
    // AudioSampleEntry: 6 reserved + 2 data_ref_index + 8 reserved + 2 channels + 2 sample_size + 2 pre_defined + 2 reserved + 4 sample_rate
    if buf.remaining() < 28 { return None; }
    buf.advance(6 + 2); // reserved + data_reference_index
    buf.advance(8); // reserved
    let channel_count = buf.get_u16();
    buf.advance(2 + 2 + 2); // sample_size + pre_defined + reserved
    let sample_rate = buf.get_u32() >> 16; // fixed-point 16.16

    // Find esds sub-box
    let consumed = content.len() - before_entry + entry_hdr.header_size as usize + 28;
    let sub_start = stsd_file_start + consumed;
    let entry_end = stsd_file_start + (content.len() - before_entry) + entry_hdr.size as usize;

    if let Some((esds_start, esds_end)) = find_box(file_data, sub_start, entry_end, ESDS) {
        let esds_data = &file_data[esds_start..esds_end];
        // Extract AudioSpecificConfig from esds
        let asc = extract_audio_specific_config(esds_data);
        Some((sample_rate, channel_count, asc))
    } else {
        None
    }
}

/// Extract AudioSpecificConfig from esds box content
fn extract_audio_specific_config(esds: &[u8]) -> Vec<u8> {
    // esds is a FullBox: version(1) + flags(3) + ES_Descriptor
    if esds.len() < 4 { return Vec::new(); }
    let mut pos = 4usize; // skip version + flags

    // Walk through ES_Descriptor tags to find DecoderSpecificInfo (tag 0x05)
    // Tags: 0x03 = ES_Descriptor, 0x04 = DecoderConfigDescriptor, 0x05 = DecoderSpecificInfo
    fn read_descr_len(data: &[u8], pos: &mut usize) -> usize {
        let mut len = 0usize;
        for _ in 0..4 {
            if *pos >= data.len() { return len; }
            let b = data[*pos];
            *pos += 1;
            len = (len << 7) | (b & 0x7F) as usize;
            if b & 0x80 == 0 { break; }
        }
        len
    }

    while pos < esds.len() {
        let tag = esds[pos];
        pos += 1;
        let len = read_descr_len(esds, &mut pos);
        if tag == 0x05 {
            // DecoderSpecificInfo — this IS the AudioSpecificConfig
            let end = (pos + len).min(esds.len());
            return esds[pos..end].to_vec();
        }
        if tag == 0x03 {
            // ES_Descriptor: skip ES_ID(2) + flags(1)
            if pos + 3 <= esds.len() { pos += 3; }
            continue; // next tag inside
        }
        if tag == 0x04 {
            // DecoderConfigDescriptor: skip objectTypeIndication(1) + streamType(1) + bufferSizeDB(3) + maxBitrate(4) + avgBitrate(4)
            if pos + 13 <= esds.len() { pos += 13; }
            continue; // next tag (should be 0x05)
        }
        // Unknown tag — skip
        pos += len;
    }
    Vec::new()
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
        buf.advance(4);
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

fn parse_audio_trak(data: &[u8], trak_start: usize, trak_end: usize) -> Option<AudioTrack> {
    let (mdia_s, mdia_e) = find_box(data, trak_start, trak_end, MDIA)?;
    let mdia_boxes = find_boxes(data, mdia_s, mdia_e);
    let &(_, hdlr_s, hdlr_e) = mdia_boxes.iter().find(|(t, _, _)| *t == HDLR)?;
    if parse_hdlr(&data[hdlr_s..hdlr_e])? != SOUN { return None; }
    let &(_, mdhd_s, mdhd_e) = mdia_boxes.iter().find(|(t, _, _)| *t == MDHD)?;
    let (timescale, duration) = parse_mdhd(&data[mdhd_s..mdhd_e])?;
    let (minf_s, minf_e) = find_box(data, mdia_s, mdia_e, MINF)?;
    let (stbl_s, stbl_e) = find_box(data, minf_s, minf_e, STBL)?;
    let stbl_boxes = find_boxes(data, stbl_s, stbl_e);
    let get = |bt: u32| -> Option<(usize, usize)> {
        stbl_boxes.iter().find(|(t, _, _)| *t == bt).map(|(_, s, e)| (*s, *e))
    };
    let (stsd_s, stsd_e) = get(STSD)?;
    let (sample_rate, channel_count, codec_config) =
        parse_stsd_aac(&data[stsd_s..stsd_e], data, stsd_s)?;
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
    } else { return None; };
    Some(AudioTrack {
        timescale, duration, sample_rate, channel_count, codec_config,
        sample_sizes, chunk_offsets, stsc_entries, sample_durations,
    })
}

pub struct Mp4Tracks {
    pub video: VideoTrack,
    pub audio: Option<AudioTrack>,
}

/// Parse from moov box data directly (for streaming — moov fetched separately).
pub fn parse_mp4_moov(moov_data: &[u8]) -> Result<Mp4Tracks, String> {
    // moov_data IS the moov content — scan for trak boxes inside it
    let mut video = None;
    let mut audio = None;
    for (box_type, start, end) in find_boxes(moov_data, 0, moov_data.len()) {
        if box_type == TRAK {
            if video.is_none() {
                if let Some(t) = parse_trak(moov_data, start, end) { video = Some(t); continue; }
            }
            if audio.is_none() {
                if let Some(t) = parse_audio_trak(moov_data, start, end) { audio = Some(t); }
            }
        }
    }
    let video = video.ok_or("no HEVC video track in moov")?;
    Ok(Mp4Tracks { video, audio })
}

pub fn compute_pts_offset_for(track: &VideoTrack, dts_values: &[u64]) -> f64 {
    let count = track.sample_count();
    let mut min_pts = f64::MAX;
    for i in 0..count {
        let dts = dts_values[i] as f64;
        let cts = if i < track.composition_offsets.len() {
            track.composition_offsets[i] as f64
        } else { 0.0 };
        if dts + cts < min_pts { min_pts = dts + cts; }
    }
    if min_pts == f64::MAX { 0.0 } else { min_pts }
}

fn parse_mp4(data: &[u8]) -> Result<Mp4Tracks, String> {
    let (moov_s, moov_e) = find_box(data, 0, data.len(), MOOV).ok_or("no moov box found")?;
    let mut video = None;
    let mut audio = None;
    for (box_type, start, end) in find_boxes(data, moov_s, moov_e) {
        if box_type == TRAK {
            if video.is_none() {
                if let Some(t) = parse_trak(data, start, end) { video = Some(t); continue; }
            }
            if audio.is_none() {
                if let Some(t) = parse_audio_trak(data, start, end) { audio = Some(t); }
            }
        }
    }
    let video = video.ok_or("no HEVC video track found")?;
    Ok(Mp4Tracks { video, audio })
}

fn build_codec_string(hvcc: &[u8], codec_fourcc: u32) -> String {
    if hvcc.len() < 13 {
        return String::from("hev1.1.6.L93.B0");
    }
    let prefix = if codec_fourcc == HVC1 { "hvc1" } else { "hev1" };
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
    let last_nonzero = constraint_bytes.iter().rposition(|&b| b != 0).unwrap_or(0);
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
    let compat_rev = compat.reverse_bits();
    format!(
        "{}.{}{}.{:X}.{}{}{}",
        prefix, profile_space_str, profile_idc, compat_rev, tier_str, level_idc, constraint_suffix
    )
}

// ── WASM exports ──

#[wasm_bindgen]
pub struct Sample {
    is_sync: bool,
    timestamp_us: f64,
    duration_us: f64,
    data: Vec<u8>,
}

impl Sample {
    pub fn new(is_sync: bool, timestamp_us: f64, duration_us: f64, data: Vec<u8>) -> Self {
        Sample { is_sync, timestamp_us, duration_us, data }
    }
}

#[wasm_bindgen]
impl Sample {
    #[wasm_bindgen(getter)]
    pub fn is_sync(&self) -> bool { self.is_sync }
    #[wasm_bindgen(getter)]
    pub fn timestamp_us(&self) -> f64 { self.timestamp_us }
    #[wasm_bindgen(getter)]
    pub fn duration_us(&self) -> f64 { self.duration_us }
    #[wasm_bindgen(getter)]
    pub fn data(&self) -> Vec<u8> { self.data.clone() }
}

#[wasm_bindgen]
pub struct Demuxer {
    data: Vec<u8>,
    // Video
    video: VideoTrack,
    v_sample_offsets: Vec<u64>,
    v_dts_values: Vec<u64>,
    v_pts_offset: f64,
    // Audio
    audio: Option<AudioTrack>,
    a_sample_offsets: Vec<u64>,
    a_dts_values: Vec<u64>,
}

fn compute_pts_offset(track: &VideoTrack) -> f64 {
    let count = track.sample_count();
    let dts_values = track.build_dts();
    let mut min_pts = f64::MAX;
    for i in 0..count {
        let dts = dts_values[i] as f64;
        let cts = if i < track.composition_offsets.len() {
            track.composition_offsets[i] as f64
        } else { 0.0 };
        if dts + cts < min_pts { min_pts = dts + cts; }
    }
    if min_pts == f64::MAX { 0.0 } else { min_pts }
}

#[wasm_bindgen]
impl Demuxer {
    #[wasm_bindgen(constructor)]
    pub fn new(data: Vec<u8>) -> Result<Demuxer, JsValue> {
        console_error_panic_hook::set_once();
        let tracks =
            parse_mp4(&data).map_err(|e| JsValue::from_str(&format!("MP4 parse error: {}", e)))?;

        let v_sample_offsets = tracks.video.build_sample_offsets();
        let v_dts_values = tracks.video.build_dts();
        let v_pts_offset = compute_pts_offset(&tracks.video);

        let (a_sample_offsets, a_dts_values) = if let Some(ref a) = tracks.audio {
            (a.build_sample_offsets(), a.build_dts())
        } else {
            (Vec::new(), Vec::new())
        };

        Ok(Demuxer {
            data,
            video: tracks.video,
            v_sample_offsets, v_dts_values, v_pts_offset,
            audio: tracks.audio,
            a_sample_offsets, a_dts_values,
        })
    }

    // ── Video API ──

    pub fn width(&self) -> u32 { self.video.width as u32 }
    pub fn height(&self) -> u32 { self.video.height as u32 }
    pub fn sample_count(&self) -> u32 { self.video.sample_count() as u32 }

    pub fn duration_ms(&self) -> f64 {
        if self.video.timescale == 0 { return 0.0; }
        (self.video.duration as f64 / self.video.timescale as f64) * 1000.0
    }

    pub fn codec_string(&self) -> String {
        build_codec_string(&self.video.hvcc_raw, self.video.codec_fourcc)
    }

    pub fn codec_description(&self) -> Vec<u8> { self.video.hvcc_raw.clone() }

    pub fn nal_length_size(&self) -> u8 {
        if self.video.hvcc_raw.len() > 21 { (self.video.hvcc_raw[21] & 0x03) + 1 } else { 4 }
    }

    pub fn read_sample(&self, index: u32) -> Option<Sample> {
        let i = index as usize;
        if i >= self.video.sample_count() { return None; }
        let offset = self.v_sample_offsets[i] as usize;
        let size = self.video.sample_sizes[i] as usize;
        if offset + size > self.data.len() { return None; }
        let sample_data = self.data[offset..offset + size].to_vec();
        let timescale = self.video.timescale as f64;
        let dts = self.v_dts_values[i] as f64;
        let cts_offset = if i < self.video.composition_offsets.len() {
            self.video.composition_offsets[i] as f64
        } else { 0.0 };
        let pts = dts + cts_offset - self.v_pts_offset;
        let timestamp_us = (pts / timescale) * 1_000_000.0;
        let duration = if i < self.video.sample_durations.len() {
            self.video.sample_durations[i]
        } else {
            self.video.sample_durations.last().copied().unwrap_or(1)
        };
        let duration_us = (duration as f64 / timescale) * 1_000_000.0;
        Some(Sample { is_sync: self.video.is_sync(i), timestamp_us, duration_us, data: sample_data })
    }

    /// Find the video sample index of the keyframe at or before `target_us`.
    pub fn find_keyframe_before(&self, target_us: f64) -> u32 {
        let mut best = 0u32;
        for i in 0..self.video.sample_count() {
            if !self.video.is_sync(i) { continue; }
            let timescale = self.video.timescale as f64;
            let dts = self.v_dts_values[i] as f64;
            let cts = if i < self.video.composition_offsets.len() {
                self.video.composition_offsets[i] as f64
            } else { 0.0 };
            let pts = dts + cts - self.v_pts_offset;
            let ts_us = (pts / timescale) * 1_000_000.0;
            if ts_us <= target_us { best = i as u32; } else { break; }
        }
        best
    }

    /// Find the audio sample index at or before `target_us`.
    pub fn find_audio_sample_at(&self, target_us: f64) -> u32 {
        let a = match &self.audio { Some(a) => a, None => return 0 };
        let timescale = a.timescale as f64;
        let mut best = 0u32;
        for i in 0..a.sample_count() {
            let dts = self.a_dts_values[i] as f64;
            let ts_us = (dts / timescale) * 1_000_000.0;
            if ts_us <= target_us { best = i as u32; } else { break; }
        }
        best
    }

    // ── Audio API ──

    pub fn has_audio(&self) -> bool { self.audio.is_some() }

    pub fn audio_sample_rate(&self) -> u32 {
        self.audio.as_ref().map_or(0, |a| a.sample_rate)
    }

    pub fn audio_channel_count(&self) -> u16 {
        self.audio.as_ref().map_or(0, |a| a.channel_count)
    }

    /// AudioSpecificConfig from esds — needed for WebCodecs AudioDecoder config
    pub fn audio_codec_config(&self) -> Vec<u8> {
        self.audio.as_ref().map_or_else(Vec::new, |a| a.codec_config.clone())
    }

    pub fn audio_sample_count(&self) -> u32 {
        self.audio.as_ref().map_or(0, |a| a.sample_count() as u32)
    }

    pub fn read_audio_sample(&self, index: u32) -> Option<Sample> {
        let a = self.audio.as_ref()?;
        let i = index as usize;
        if i >= a.sample_count() { return None; }
        let offset = self.a_sample_offsets[i] as usize;
        let size = a.sample_sizes[i] as usize;
        if offset + size > self.data.len() { return None; }
        let sample_data = self.data[offset..offset + size].to_vec();
        let timescale = a.timescale as f64;
        let dts = self.a_dts_values[i] as f64;
        let timestamp_us = (dts / timescale) * 1_000_000.0;
        let duration = if i < a.sample_durations.len() {
            a.sample_durations[i]
        } else {
            a.sample_durations.last().copied().unwrap_or(1)
        };
        let duration_us = (duration as f64 / timescale) * 1_000_000.0;
        Some(Sample { is_sync: true, timestamp_us, duration_us, data: sample_data })
    }
}
