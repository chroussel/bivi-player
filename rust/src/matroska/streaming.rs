/// Streaming MKV parser — state machine that consumes bytes incrementally.
/// push() data in, pull frames out. No Seek, no re-parsing.

// ── EBML primitives ──

fn read_vint(buf: &[u8], pos: usize) -> Option<(u64, usize)> {
    if pos >= buf.len() { return None; }
    let b = buf[pos];
    if b == 0 { return None; }
    let len = b.leading_zeros() as usize + 1;
    if pos + len > buf.len() { return None; }
    let mut val = (b & (0xFFu8.wrapping_shr(len as u32))) as u64;
    for i in 1..len {
        val = (val << 8) | buf[pos + i] as u64;
    }
    Some((val, len))
}

fn read_element_id(buf: &[u8], pos: usize) -> Option<(u64, usize)> {
    if pos >= buf.len() { return None; }
    let b = buf[pos];
    if b == 0 { return None; }
    let len = b.leading_zeros() as usize + 1;
    if pos + len > buf.len() { return None; }
    let mut val = b as u64;
    for i in 1..len {
        val = (val << 8) | buf[pos + i] as u64;
    }
    Some((val, len))
}

fn is_unknown_size(val: u64, vint_len: usize) -> bool {
    val == ((1u64 << (7 * vint_len)) - 1)
}

/// Try to read element header (ID + size) at position. Returns (id, size_or_none, total_header_bytes).
fn try_read_header(buf: &[u8], pos: usize) -> Option<(u64, Option<u64>, usize)> {
    let (id, id_len) = read_element_id(buf, pos)?;
    let (size_raw, size_len) = read_vint(buf, pos + id_len)?;
    let size = if is_unknown_size(size_raw, size_len) { None } else { Some(size_raw) };
    Some((id, size, id_len + size_len))
}

fn read_uint(buf: &[u8], pos: usize, n: usize) -> Option<u64> {
    if pos + n > buf.len() { return None; }
    let mut val = 0u64;
    for i in 0..n {
        val = (val << 8) | buf[pos + i] as u64;
    }
    Some(val)
}

fn read_float(buf: &[u8], pos: usize, n: usize) -> Option<f64> {
    if pos + n > buf.len() { return None; }
    match n {
        4 => Some(f32::from_be_bytes([buf[pos], buf[pos+1], buf[pos+2], buf[pos+3]]) as f64),
        8 => Some(f64::from_be_bytes([
            buf[pos], buf[pos+1], buf[pos+2], buf[pos+3],
            buf[pos+4], buf[pos+5], buf[pos+6], buf[pos+7],
        ])),
        _ => None,
    }
}

fn read_string(buf: &[u8], pos: usize, n: usize) -> Option<String> {
    if pos + n > buf.len() { return None; }
    String::from_utf8(buf[pos..pos+n].to_vec()).ok()
}

// ── MKV element IDs ──

const EBML_ID: u64 = 0x1A45DFA3;
const SEGMENT_ID: u64 = 0x18538067;
const SEGMENT_INFO_ID: u64 = 0x1549A966;
const TRACKS_ID: u64 = 0x1654AE6B;
const TRACK_ENTRY_ID: u64 = 0xAE;
const TRACK_NUMBER_ID: u64 = 0xD7;
const TRACK_TYPE_ID: u64 = 0x83;
const CODEC_ID_ID: u64 = 0x86;
const CODEC_PRIVATE_ID: u64 = 0x63A2;
const LANGUAGE_ID: u64 = 0x22B59C;
const NAME_ID: u64 = 0x536E;
const VIDEO_ID: u64 = 0xE0;
const AUDIO_ID: u64 = 0xE1;
const PIXEL_WIDTH_ID: u64 = 0xB0;
const PIXEL_HEIGHT_ID: u64 = 0xBA;
const SAMPLING_FREQ_ID: u64 = 0xB5;
const CHANNELS_ID: u64 = 0x9F;
const TIMECODE_SCALE_ID: u64 = 0x2AD7B1;
const DURATION_ID: u64 = 0x4489;
const CLUSTER_ID: u64 = 0x1F43B675;
const CLUSTER_TIMESTAMP_ID: u64 = 0xE7;
const SIMPLE_BLOCK_ID: u64 = 0xA3;
const BLOCK_GROUP_ID: u64 = 0xA0;
const BLOCK_ID: u64 = 0xA1;
const BLOCK_DURATION_ID: u64 = 0x9B;

// ── Data types ──

pub struct TrackInfo {
    pub number: u64,
    pub track_type: u64,
    pub codec_id: String,
    pub codec_private: Vec<u8>,
    pub language: String,
    pub name: String,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub sample_rate: f64,
    pub channels: u64,
}

pub struct MkvFrame {
    pub track: u64,
    pub timestamp_ns: i64,
    pub data: Vec<u8>,
    pub is_keyframe: bool,
    pub duration_ns: Option<i64>,
}

// ── State machine ──

#[derive(Debug)]
enum State {
    /// Read EBML header element, skip it
    EbmlHeader,
    /// Read Segment header (usually unknown size)
    SegmentHeader,
    /// Read next top-level element inside Segment
    TopLevel,
    /// Skip N bytes of a top-level element we don't care about
    Skip { remaining: usize },
    /// Inside a Cluster, reading children
    ClusterChildren,
    /// Done (error or EOF)
    Done,
}

pub struct StreamingMkvParser {
    buf: Vec<u8>,
    pos: usize,       // current read position in buf
    committed: usize,  // bytes we've fully processed (can be discarded)
    state: State,

    // Parsed header data
    pub timecode_scale: u64,
    pub duration: f64,
    pub tracks: Vec<TrackInfo>,
    pub header_done: bool,

    // Cluster state
    cluster_timestamp: u64,

    // Output queue
    frames: Vec<MkvFrame>,
}

impl StreamingMkvParser {
    pub fn new() -> Self {
        StreamingMkvParser {
            buf: Vec::new(),
            pos: 0,
            committed: 0,
            state: State::EbmlHeader,
            timecode_scale: 1_000_000,
            duration: 0.0,
            tracks: Vec::new(),
            header_done: false,
            cluster_timestamp: 0,
            frames: Vec::new(),
        }
    }

    /// Push data into the parser buffer.
    pub fn push(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    /// Drain parsed frames.
    pub fn drain_frames(&mut self) -> Vec<MkvFrame> {
        std::mem::take(&mut self.frames)
    }

    /// Run the state machine — processes as much data as possible.
    /// Returns true if header is parsed (tracks available).
    pub fn parse(&mut self) -> bool {
        loop {
            match self.state {
                State::EbmlHeader => {
                    if !self.step_ebml_header() { return self.header_done; }
                }
                State::SegmentHeader => {
                    if !self.step_segment_header() { return self.header_done; }
                }
                State::TopLevel => {
                    if !self.step_top_level() { return self.header_done; }
                }
                State::Skip { remaining } => {
                    let avail = self.buf.len() - self.pos;
                    if avail >= remaining {
                        self.pos += remaining;
                        self.commit();
                        self.state = State::TopLevel;
                    } else {
                        self.pos += avail;
                        self.state = State::Skip { remaining: remaining - avail };
                        self.commit();
                        return self.header_done;
                    }
                }
                State::ClusterChildren => {
                    if !self.step_cluster_children() { return self.header_done; }
                }
                State::Done => return self.header_done,
            }
        }
    }

    fn step_ebml_header(&mut self) -> bool {
        let Some((id, size, hdr_len)) = try_read_header(&self.buf, self.pos) else { return false };
        if id != EBML_ID { self.state = State::Done; return false; }
        let size = match size { Some(s) => s as usize, None => { self.state = State::Done; return false; } };
        if self.pos + hdr_len + size > self.buf.len() { return false; }
        self.pos += hdr_len + size;
        self.commit();
        self.state = State::SegmentHeader;
        true
    }

    fn step_segment_header(&mut self) -> bool {
        let Some((id, _size, hdr_len)) = try_read_header(&self.buf, self.pos) else { return false };
        if id != SEGMENT_ID { self.state = State::Done; return false; }
        // Segment is usually unknown size — we just enter it
        self.pos += hdr_len;
        self.commit();
        self.state = State::TopLevel;
        true
    }

    fn step_top_level(&mut self) -> bool {
        let Some((id, size, hdr_len)) = try_read_header(&self.buf, self.pos) else { return false };

        if id == CLUSTER_ID {
            self.pos += hdr_len;
            self.commit();
            self.header_done = true;
            self.state = State::ClusterChildren;
            return true;
        }

        let size = match size { Some(s) => s as usize, None => { self.state = State::Done; return false; } };

        match id {
            SEGMENT_INFO_ID => {
                if self.pos + hdr_len + size > self.buf.len() { return false; }
                self.parse_segment_info(self.pos + hdr_len, size);
                self.pos += hdr_len + size;
                self.commit();
                true
            }
            TRACKS_ID => {
                if self.pos + hdr_len + size > self.buf.len() { return false; }
                self.parse_tracks(self.pos + hdr_len, size);
                self.pos += hdr_len + size;
                self.commit();
                true
            }
            _ => {
                // Skip this element
                self.pos += hdr_len;
                self.state = State::Skip { remaining: size };
                true
            }
        }
    }

    fn step_cluster_children(&mut self) -> bool {
        let Some((id, size, hdr_len)) = try_read_header(&self.buf, self.pos) else { return false };

        // New cluster?
        if id == CLUSTER_ID {
            self.pos += hdr_len;
            self.commit();
            return true; // stay in ClusterChildren
        }

        let size = match size {
            Some(s) => s as usize,
            None => { self.pos += hdr_len; self.commit(); return true; }
        };

        if self.pos + hdr_len + size > self.buf.len() { return false; }

        match id {
            CLUSTER_TIMESTAMP_ID => {
                self.cluster_timestamp = read_uint(&self.buf, self.pos + hdr_len, size).unwrap_or(0);
            }
            SIMPLE_BLOCK_ID => {
                let data_start = self.pos + hdr_len;
                if let Some(frame) = self.parse_simple_block(data_start, size) {
                    self.frames.push(frame);
                }
            }
            BLOCK_GROUP_ID => {
                let data_start = self.pos + hdr_len;
                if let Some(frame) = self.parse_block_group(data_start, size) {
                    self.frames.push(frame);
                }
            }
            _ => {} // skip unknown cluster children
        }

        self.pos += hdr_len + size;
        self.commit();
        true
    }

    fn parse_simple_block(&self, pos: usize, size: usize) -> Option<MkvFrame> {
        if size < 4 { return None; }
        let (track, vint_len) = read_vint(&self.buf, pos)?;
        let ts_pos = pos + vint_len;
        if ts_pos + 3 > pos + size { return None; }
        let rel_ts = ((self.buf[ts_pos] as i16) << 8) | self.buf[ts_pos + 1] as i16;
        let flags = self.buf[ts_pos + 2];
        let keyframe = (flags & 0x80) != 0;
        let data_start = ts_pos + 3;
        let data_end = pos + size;
        if data_start > data_end { return None; }
        let abs_ts = self.cluster_timestamp as i64 + rel_ts as i64;
        Some(MkvFrame {
            track,
            timestamp_ns: abs_ts * self.timecode_scale as i64,
            data: self.buf[data_start..data_end].to_vec(),
            is_keyframe: keyframe,
            duration_ns: None,
        })
    }

    fn parse_block_group(&self, pos: usize, size: usize) -> Option<MkvFrame> {
        let end = pos + size;
        let mut cursor = pos;
        let mut block_frame: Option<MkvFrame> = None;
        let mut block_duration: Option<u64> = None;

        while cursor < end {
            let (id, el_size, hdr_len) = try_read_header(&self.buf, cursor)?;
            let el_size = el_size? as usize;
            let data_pos = cursor + hdr_len;
            if data_pos + el_size > end { break; }

            match id {
                BLOCK_ID => {
                    block_frame = self.parse_simple_block(data_pos, el_size);
                }
                BLOCK_DURATION_ID => {
                    block_duration = read_uint(&self.buf, data_pos, el_size);
                }
                _ => {}
            }
            cursor = data_pos + el_size;
        }

        if let Some(ref mut frame) = block_frame {
            frame.duration_ns = block_duration.map(|d| d as i64 * self.timecode_scale as i64);
        }
        block_frame
    }

    fn parse_segment_info(&mut self, pos: usize, size: usize) {
        let end = pos + size;
        let mut cursor = pos;
        while cursor < end {
            let Some((id, el_size, hdr_len)) = try_read_header(&self.buf, cursor) else { break };
            let Some(el_size) = el_size else { break };
            let el_size = el_size as usize;
            let data_pos = cursor + hdr_len;
            if data_pos + el_size > end { break; }

            match id {
                TIMECODE_SCALE_ID => {
                    self.timecode_scale = read_uint(&self.buf, data_pos, el_size).unwrap_or(1_000_000);
                }
                DURATION_ID => {
                    self.duration = read_float(&self.buf, data_pos, el_size).unwrap_or(0.0);
                }
                _ => {}
            }
            cursor = data_pos + el_size;
        }
    }

    fn parse_tracks(&mut self, pos: usize, size: usize) {
        let end = pos + size;
        let mut cursor = pos;
        while cursor < end {
            let Some((id, el_size, hdr_len)) = try_read_header(&self.buf, cursor) else { break };
            let Some(el_size) = el_size else { break };
            let el_size = el_size as usize;
            let data_pos = cursor + hdr_len;
            if data_pos + el_size > end { break; }

            if id == TRACK_ENTRY_ID {
                if let Some(track) = self.parse_track_entry(data_pos, el_size) {
                    self.tracks.push(track);
                }
            }
            cursor = data_pos + el_size;
        }
    }

    fn parse_track_entry(&self, pos: usize, size: usize) -> Option<TrackInfo> {
        let end = pos + size;
        let mut cursor = pos;
        let mut info = TrackInfo {
            number: 0, track_type: 0, codec_id: String::new(),
            codec_private: Vec::new(), language: "und".to_string(),
            name: String::new(), pixel_width: 0, pixel_height: 0,
            sample_rate: 0.0, channels: 0,
        };

        while cursor < end {
            let (id, el_size, hdr_len) = try_read_header(&self.buf, cursor)?;
            let el_size = el_size? as usize;
            let dp = cursor + hdr_len;
            if dp + el_size > end { break; }

            match id {
                TRACK_NUMBER_ID => info.number = read_uint(&self.buf, dp, el_size)?,
                TRACK_TYPE_ID => info.track_type = read_uint(&self.buf, dp, el_size)?,
                CODEC_ID_ID => info.codec_id = read_string(&self.buf, dp, el_size)?,
                CODEC_PRIVATE_ID => info.codec_private = self.buf[dp..dp+el_size].to_vec(),
                LANGUAGE_ID => info.language = read_string(&self.buf, dp, el_size).unwrap_or_else(|| "und".into()),
                NAME_ID => info.name = read_string(&self.buf, dp, el_size).unwrap_or_default(),
                VIDEO_ID => self.parse_video_sub(&self.buf[dp..dp+el_size], &mut info),
                AUDIO_ID => self.parse_audio_sub(&self.buf[dp..dp+el_size], &mut info),
                _ => {}
            }
            cursor = dp + el_size;
        }
        Some(info)
    }

    fn parse_video_sub(&self, data: &[u8], info: &mut TrackInfo) {
        let mut cursor = 0;
        while cursor < data.len() {
            let Some((id, el_size, hdr_len)) = try_read_header(data, cursor) else { break };
            let Some(el_size) = el_size else { break };
            let el_size = el_size as usize;
            let dp = cursor + hdr_len;
            if dp + el_size > data.len() { break; }
            match id {
                PIXEL_WIDTH_ID => info.pixel_width = read_uint(data, dp, el_size).unwrap_or(0) as u32,
                PIXEL_HEIGHT_ID => info.pixel_height = read_uint(data, dp, el_size).unwrap_or(0) as u32,
                _ => {}
            }
            cursor = dp + el_size;
        }
    }

    fn parse_audio_sub(&self, data: &[u8], info: &mut TrackInfo) {
        let mut cursor = 0;
        while cursor < data.len() {
            let Some((id, el_size, hdr_len)) = try_read_header(data, cursor) else { break };
            let Some(el_size) = el_size else { break };
            let el_size = el_size as usize;
            let dp = cursor + hdr_len;
            if dp + el_size > data.len() { break; }
            match id {
                SAMPLING_FREQ_ID => info.sample_rate = read_float(data, dp, el_size).unwrap_or(0.0),
                CHANNELS_ID => info.channels = read_uint(data, dp, el_size).unwrap_or(0),
                _ => {}
            }
            cursor = dp + el_size;
        }
    }

    /// Discard already-processed bytes from buffer.
    fn commit(&mut self) {
        if self.pos > 0 {
            self.buf.drain(..self.pos);
            self.committed += self.pos;
            self.pos = 0;
        }
    }

    /// Total bytes consumed from the stream.
    pub fn bytes_consumed(&self) -> usize {
        self.committed + self.pos
    }
}
