use hevc_sys::*;
use std::ptr;
use wasm_bindgen::prelude::*;

const START_CODE: [u8; 4] = [0, 0, 0, 1];

const NUM_SLOTS: usize = 32;
const MAX_Y_BUF: usize = 3840 * 2160;
const MAX_C_BUF: usize = 1920 * 1080;

#[wasm_bindgen]
pub struct HevcDecoder {
    ctx: *mut de265_decoder_context,
    // Ring-buffered 8-bit plane copies
    y_bufs: Vec<Vec<u8>>,
    u_bufs: Vec<Vec<u8>>,
    v_bufs: Vec<Vec<u8>>,
    ring_idx: usize,
}

/// Decoded frame info returned to JS
#[wasm_bindgen]
pub struct DecodedFrame {
    pts: f64,
    width: u32,
    height: u32,
    y_offset: usize, // offset into y_bufs ring slot
    u_offset: usize,
    v_offset: usize,
    slot: usize,
}

#[wasm_bindgen]
impl DecodedFrame {
    #[wasm_bindgen(getter)]
    pub fn pts(&self) -> f64 { self.pts }
    #[wasm_bindgen(getter)]
    pub fn width(&self) -> u32 { self.width }
    #[wasm_bindgen(getter)]
    pub fn height(&self) -> u32 { self.height }
}

#[wasm_bindgen]
impl HevcDecoder {
    #[wasm_bindgen(constructor)]
    pub fn new() -> HevcDecoder {
        let ctx = unsafe { de265_new_decoder() };
        unsafe { de265_start_worker_threads(ctx, 0) };

        let mut y_bufs = Vec::with_capacity(NUM_SLOTS);
        let mut u_bufs = Vec::with_capacity(NUM_SLOTS);
        let mut v_bufs = Vec::with_capacity(NUM_SLOTS);
        for _ in 0..NUM_SLOTS {
            y_bufs.push(vec![0u8; MAX_Y_BUF]);
            u_bufs.push(vec![0u8; MAX_C_BUF]);
            v_bufs.push(vec![0u8; MAX_C_BUF]);
        }

        HevcDecoder { ctx, y_bufs, u_bufs, v_bufs, ring_idx: 0 }
    }

    /// Push hvcC configuration (VPS/SPS/PPS)
    pub fn configure(&mut self, hvcc: &[u8]) -> i32 {
        if hvcc.len() < 23 { return -1; }
        let mut pos = 22usize;
        let num_arrays = hvcc[pos] as usize;
        pos += 1;
        let mut count = 0i32;

        for _ in 0..num_arrays {
            if pos + 3 > hvcc.len() { break; }
            pos += 1; // array header
            let num_nalus = ((hvcc[pos] as usize) << 8) | hvcc[pos + 1] as usize;
            pos += 2;
            for _ in 0..num_nalus {
                if pos + 2 > hvcc.len() { break; }
                let len = ((hvcc[pos] as usize) << 8) | hvcc[pos + 1] as usize;
                pos += 2;
                if pos + len > hvcc.len() { break; }
                unsafe {
                    de265_push_data(self.ctx, START_CODE.as_ptr(), 4, 0, ptr::null_mut());
                    de265_push_data(self.ctx, hvcc[pos..].as_ptr(), len as i32, 0, ptr::null_mut());
                    de265_push_end_of_NAL(self.ctx);
                }
                count += 1;
                pos += len;
            }
        }

        // Process parameter sets
        let mut more = 1i32;
        while more > 0 {
            let err = unsafe { de265_decode(self.ctx, &mut more) };
            if err == DE265_ERROR_WAITING_FOR_INPUT_DATA || err != DE265_OK { break; }
        }
        count
    }

    /// Push one MP4 sample
    pub fn push_sample(&mut self, data: &[u8], nal_length_size: u32, pts: f64) {
        let nls = nal_length_size as usize;
        let mut pos = 0usize;
        let pts_i64 = pts as i64;
        while pos + nls <= data.len() {
            let mut nal_len = 0usize;
            for i in 0..nls {
                nal_len = (nal_len << 8) | data[pos + i] as usize;
            }
            pos += nls;
            if pos + nal_len > data.len() { break; }
            unsafe {
                de265_push_data(self.ctx, START_CODE.as_ptr(), 4, pts_i64, ptr::null_mut());
                de265_push_data(self.ctx, data[pos..].as_ptr(), nal_len as i32, pts_i64, ptr::null_mut());
                de265_push_end_of_NAL(self.ctx);
            }
            pos += nal_len;
        }
        unsafe { de265_push_end_of_frame(self.ctx); }
    }

    /// Run decode loop until it needs more input. Returns number of frames collected.
    pub fn decode(&mut self) -> Vec<DecodedFrame> {
        let mut frames = Vec::new();
        let mut more = 1i32;
        while more > 0 {
            let err = unsafe { de265_decode(self.ctx, &mut more) };
            if err == DE265_ERROR_WAITING_FOR_INPUT_DATA { break; }
            if err != DE265_OK { break; }
            self.collect_frames(&mut frames);
        }
        self.collect_frames(&mut frames);
        frames
    }

    pub fn flush(&mut self) -> Vec<DecodedFrame> {
        unsafe { de265_flush_data(self.ctx); }
        let mut frames = Vec::new();
        let mut more = 1i32;
        while more > 0 {
            let err = unsafe { de265_decode(self.ctx, &mut more) };
            if err != DE265_OK && err != DE265_ERROR_WAITING_FOR_INPUT_DATA { break; }
            self.collect_frames(&mut frames);
        }
        self.collect_frames(&mut frames);
        frames
    }

    pub fn reset(&mut self) {
        unsafe { de265_reset(self.ctx); }
    }

    /// Get the Y plane data for a decoded frame
    pub fn frame_y(&self, frame: &DecodedFrame) -> Vec<u8> {
        let size = frame.width as usize * frame.height as usize;
        self.y_bufs[frame.slot][..size].to_vec()
    }

    /// Get the U plane data for a decoded frame
    pub fn frame_u(&self, frame: &DecodedFrame) -> Vec<u8> {
        let size = (frame.width as usize >> 1) * (frame.height as usize >> 1);
        self.u_bufs[frame.slot][..size].to_vec()
    }

    /// Get the V plane data for a decoded frame
    pub fn frame_v(&self, frame: &DecodedFrame) -> Vec<u8> {
        let size = (frame.width as usize >> 1) * (frame.height as usize >> 1);
        self.v_bufs[frame.slot][..size].to_vec()
    }

    fn collect_frames(&mut self, out: &mut Vec<DecodedFrame>) {
        loop {
            let img = unsafe { de265_get_next_picture(self.ctx) };
            if img.is_null() { break; }

            let w = unsafe { de265_get_image_width(img, 0) } as usize;
            let h = unsafe { de265_get_image_height(img, 0) } as usize;
            let bpp = unsafe { de265_get_bits_per_pixel(img, 0) } as u32;
            let pts = unsafe { de265_get_image_PTS(img) } as f64;

            let mut y_stride = 0i32;
            let mut u_stride = 0i32;
            let mut v_stride = 0i32;
            let y_ptr = unsafe { de265_get_image_plane(img, 0, &mut y_stride) };
            let u_ptr = unsafe { de265_get_image_plane(img, 1, &mut u_stride) };
            let v_ptr = unsafe { de265_get_image_plane(img, 2, &mut v_stride) };

            let slot = self.ring_idx;
            self.ring_idx = (self.ring_idx + 1) % NUM_SLOTS;

            let cw = w >> 1;
            let ch = h >> 1;

            // Convert + copy into ring buffer
            if bpp > 8 {
                let shift = bpp - 8;
                for r in 0..h {
                    let src = unsafe { std::slice::from_raw_parts(y_ptr.add(r * y_stride as usize) as *const u16, w) };
                    let dst = &mut self.y_bufs[slot][r * w..(r + 1) * w];
                    for c in 0..w { dst[c] = (src[c] >> shift) as u8; }
                }
                for r in 0..ch {
                    let su = unsafe { std::slice::from_raw_parts(u_ptr.add(r * u_stride as usize) as *const u16, cw) };
                    let sv = unsafe { std::slice::from_raw_parts(v_ptr.add(r * v_stride as usize) as *const u16, cw) };
                    let du = &mut self.u_bufs[slot][r * cw..(r + 1) * cw];
                    let dv = &mut self.v_bufs[slot][r * cw..(r + 1) * cw];
                    for c in 0..cw { du[c] = (su[c] >> shift) as u8; dv[c] = (sv[c] >> shift) as u8; }
                }
            } else {
                for r in 0..h {
                    let src = unsafe { std::slice::from_raw_parts(y_ptr.add(r * y_stride as usize), w) };
                    self.y_bufs[slot][r * w..(r + 1) * w].copy_from_slice(src);
                }
                for r in 0..ch {
                    let su = unsafe { std::slice::from_raw_parts(u_ptr.add(r * u_stride as usize), cw) };
                    let sv = unsafe { std::slice::from_raw_parts(v_ptr.add(r * v_stride as usize), cw) };
                    self.u_bufs[slot][r * cw..(r + 1) * cw].copy_from_slice(su);
                    self.v_bufs[slot][r * cw..(r + 1) * cw].copy_from_slice(sv);
                }
            }

            out.push(DecodedFrame {
                pts,
                width: w as u32,
                height: h as u32,
                y_offset: 0,
                u_offset: 0,
                v_offset: 0,
                slot,
            });
        }
    }
}

impl Drop for HevcDecoder {
    fn drop(&mut self) {
        if !self.ctx.is_null() {
            unsafe { de265_free_decoder(self.ctx); }
            self.ctx = ptr::null_mut();
        }
    }
}
