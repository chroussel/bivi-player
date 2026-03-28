use js_sys::Uint8Array;
use wasm_bindgen::prelude::*;

pub(crate) struct Frame {
    pub pts: f64,
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

#[wasm_bindgen]
pub struct FrameBuffer {
    frames: Vec<Frame>,
    pub(crate) current: Option<Frame>,
    max_size: usize,
    min_reorder: usize,
    last_shown_pts: f64,
}

#[wasm_bindgen]
impl FrameBuffer {
    #[wasm_bindgen(constructor)]
    pub fn new(max_size: usize, min_reorder: usize) -> FrameBuffer {
        FrameBuffer {
            frames: Vec::new(),
            current: None,
            max_size,
            min_reorder,
            last_shown_pts: -1.0,
        }
    }

    pub fn push(
        &mut self,
        pts: f64,
        y: Vec<u8>,
        u: Vec<u8>,
        v: Vec<u8>,
        width: u32,
        height: u32,
    ) {
        if self.frames.len() >= self.max_size {
            return;
        }
        let pos = self.frames.partition_point(|f| f.pts < pts);
        self.frames.insert(
            pos,
            Frame {
                pts,
                y,
                u,
                v,
                width,
                height,
            },
        );
    }

    /// Push from raw decoder shared memory. Handles 10-bit→8-bit and stride stripping.
    pub fn push_raw(
        &mut self,
        pts: f64,
        plane_data: &Uint8Array,
        y_ptr: u32,
        y_stride: u32,
        u_ptr: u32,
        u_stride: u32,
        v_ptr: u32,
        v_stride: u32,
        width: u32,
        height: u32,
        bpp: u32,
    ) {
        if self.frames.len() >= self.max_size {
            return;
        }
        let w = width as usize;
        let h = height as usize;
        let cw = w >> 1;
        let ch = h >> 1;
        let shift = if bpp > 8 { bpp - 8 } else { 0 };

        let mut y_out = vec![0u8; w * h];
        let mut u_out = vec![0u8; cw * ch];
        let mut v_out = vec![0u8; cw * ch];

        if shift > 0 {
            let mut row_buf = vec![0u8; (w * 2).max(cw * 2)];
            for r in 0..h {
                let offset = (y_ptr + r as u32 * y_stride) as u32;
                plane_data
                    .slice(offset, offset + (w * 2) as u32)
                    .copy_to(&mut row_buf[..w * 2]);
                for c in 0..w {
                    let val = (row_buf[c * 2] as u32) | ((row_buf[c * 2 + 1] as u32) << 8);
                    y_out[r * w + c] = (val >> shift) as u8;
                }
            }
            for r in 0..ch {
                let u_off = (u_ptr + r as u32 * u_stride) as u32;
                let v_off = (v_ptr + r as u32 * v_stride) as u32;
                plane_data
                    .slice(u_off, u_off + (cw * 2) as u32)
                    .copy_to(&mut row_buf[..cw * 2]);
                for c in 0..cw {
                    let val = (row_buf[c * 2] as u32) | ((row_buf[c * 2 + 1] as u32) << 8);
                    u_out[r * cw + c] = (val >> shift) as u8;
                }
                plane_data
                    .slice(v_off, v_off + (cw * 2) as u32)
                    .copy_to(&mut row_buf[..cw * 2]);
                for c in 0..cw {
                    let val = (row_buf[c * 2] as u32) | ((row_buf[c * 2 + 1] as u32) << 8);
                    v_out[r * cw + c] = (val >> shift) as u8;
                }
            }
        } else {
            // 8-bit: bulk copy when stride == width, row-by-row otherwise
            if y_stride == w as u32 {
                plane_data
                    .slice(y_ptr, y_ptr + (w * h) as u32)
                    .copy_to(&mut y_out);
                plane_data
                    .slice(u_ptr, u_ptr + (cw * ch) as u32)
                    .copy_to(&mut u_out);
                plane_data
                    .slice(v_ptr, v_ptr + (cw * ch) as u32)
                    .copy_to(&mut v_out);
            } else {
                for r in 0..h {
                    let offset = y_ptr + r as u32 * y_stride;
                    plane_data
                        .slice(offset, offset + w as u32)
                        .copy_to(&mut y_out[r * w..(r + 1) * w]);
                }
                for r in 0..ch {
                    let u_off = u_ptr + r as u32 * u_stride;
                    let v_off = v_ptr + r as u32 * v_stride;
                    plane_data
                        .slice(u_off, u_off + cw as u32)
                        .copy_to(&mut u_out[r * cw..(r + 1) * cw]);
                    plane_data
                        .slice(v_off, v_off + cw as u32)
                        .copy_to(&mut v_out[r * cw..(r + 1) * cw]);
                }
            }
        }

        let pos = self.frames.partition_point(|f| f.pts < pts);
        self.frames.insert(
            pos,
            Frame {
                pts,
                y: y_out,
                u: u_out,
                v: v_out,
                width,
                height,
            },
        );
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_ready(&self) -> bool {
        self.frames.len() > self.min_reorder
    }

    pub fn pop_frame(&mut self, elapsed_us: f64, flushing: bool) -> bool {
        let min = if flushing { 0 } else { self.min_reorder };
        let mut found = false;
        while self.frames.len() > min {
            if let Some(f) = self.frames.first() {
                if f.pts > elapsed_us {
                    break;
                }
                let frame = self.frames.remove(0);
                if frame.pts >= self.last_shown_pts {
                    self.last_shown_pts = frame.pts;
                    self.current = Some(frame);
                    found = true;
                }
            } else {
                break;
            }
        }
        found
    }

    pub fn current_pts(&self) -> f64 {
        self.current.as_ref().map_or(0.0, |f| f.pts)
    }

    pub fn current_width(&self) -> u32 {
        self.current.as_ref().map_or(0, |f| f.width)
    }

    pub fn current_height(&self) -> u32 {
        self.current.as_ref().map_or(0, |f| f.height)
    }

    pub fn current_y(&self) -> Vec<u8> {
        self.current.as_ref().map_or_else(Vec::new, |f| f.y.clone())
    }

    pub fn current_u(&self) -> Vec<u8> {
        self.current.as_ref().map_or_else(Vec::new, |f| f.u.clone())
    }

    pub fn current_v(&self) -> Vec<u8> {
        self.current.as_ref().map_or_else(Vec::new, |f| f.v.clone())
    }

    /// After seek: skip frames with PTS before this value
    pub fn set_skip_until(&mut self, pts: f64) {
        self.last_shown_pts = pts - 1.0; // allow the frame at exactly pts
    }

    pub fn reset(&mut self) {
        self.frames.clear();
        self.current = None;
        self.last_shown_pts = -1.0;
    }
}
