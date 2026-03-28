use crate::yuv;

const NUM_SLOTS: usize = 32;
const MAX_Y_BUF: usize = 3840 * 2160; // up to 4K
const MAX_C_BUF: usize = 1920 * 1080; // chroma at half res

/// Frame info matching codec_api.h codec_frame_t layout
#[repr(C)]
pub struct FrameInfo {
    pub w: i32,
    pub h: i32,
    pub bpp: i32,
    pub pix_fmt: i32,
    pub pts: i64,
    pub plane0_ptr: u32,
    pub plane0_stride: i32,
    pub plane1_ptr: u32,
    pub plane1_stride: i32,
    pub plane2_ptr: u32,
    pub plane2_stride: i32,
}

pub struct RingBuffer {
    y_bufs: Vec<Vec<u8>>,
    u_bufs: Vec<Vec<u8>>,
    v_bufs: Vec<Vec<u8>>,
    idx: usize,
}

impl RingBuffer {
    pub fn new() -> Self {
        let mut y_bufs = Vec::with_capacity(NUM_SLOTS);
        let mut u_bufs = Vec::with_capacity(NUM_SLOTS);
        let mut v_bufs = Vec::with_capacity(NUM_SLOTS);
        for _ in 0..NUM_SLOTS {
            y_bufs.push(vec![0u8; MAX_Y_BUF]);
            u_bufs.push(vec![0u8; MAX_C_BUF]);
            v_bufs.push(vec![0u8; MAX_C_BUF]);
        }
        RingBuffer {
            y_bufs,
            u_bufs,
            v_bufs,
            idx: 0,
        }
    }

    /// Copy + convert a decoded frame into the next ring slot.
    /// Returns pointers to the 8-bit, stride-stripped plane data.
    pub fn store_frame(
        &mut self,
        y_src: *const u8,
        u_src: *const u8,
        v_src: *const u8,
        y_stride: usize,
        u_stride: usize,
        v_stride: usize,
        width: usize,
        height: usize,
        bpp: u32,
    ) -> (u32, u32, u32) {
        let slot = self.idx;
        self.idx = (self.idx + 1) % NUM_SLOTS;

        let cw = width >> 1;
        let ch = height >> 1;

        let y_size = if bpp > 8 { y_stride * height } else { y_stride * height };
        let u_size = if bpp > 8 { u_stride * ch } else { u_stride * ch };

        // Safety: pointers come from the codec's decoded frame buffers
        let y_slice = unsafe { core::slice::from_raw_parts(y_src, y_size) };
        let u_slice = unsafe { core::slice::from_raw_parts(u_src, u_size) };
        let v_slice = unsafe { core::slice::from_raw_parts(v_src, u_size) };

        let y_dst = &mut self.y_bufs[slot];
        let u_dst = &mut self.u_bufs[slot];
        let v_dst = &mut self.v_bufs[slot];

        if bpp > 8 {
            yuv::convert_10bit_to_8bit(
                y_slice, u_slice, v_slice, y_dst, u_dst, v_dst, width, height, y_stride,
                u_stride, v_stride, bpp,
            );
        } else {
            yuv::strip_stride(
                y_slice, u_slice, v_slice, y_dst, u_dst, v_dst, width, height, y_stride,
                u_stride, v_stride,
            );
        }

        (
            y_dst.as_ptr() as u32,
            u_dst.as_ptr() as u32,
            v_dst.as_ptr() as u32,
        )
    }
}

// Global ring buffer instance
static mut RING: Option<RingBuffer> = None;

fn ring() -> &'static mut RingBuffer {
    unsafe {
        if RING.is_none() {
            RING = Some(RingBuffer::new());
        }
        RING.as_mut().unwrap()
    }
}

/// Called from C codec wrappers to store a decoded frame.
/// Returns pointers to the 8-bit plane data via the out params.
#[unsafe(no_mangle)]
pub extern "C" fn codec_ring_store(
    y_src: *const u8,
    u_src: *const u8,
    v_src: *const u8,
    y_stride: i32,
    u_stride: i32,
    v_stride: i32,
    width: i32,
    height: i32,
    bpp: i32,
    out_y: *mut u32,
    out_u: *mut u32,
    out_v: *mut u32,
) {
    let (yp, up, vp) = ring().store_frame(
        y_src,
        u_src,
        v_src,
        y_stride as usize,
        u_stride as usize,
        v_stride as usize,
        width as usize,
        height as usize,
        bpp as u32,
    );
    unsafe {
        *out_y = yp;
        *out_u = up;
        *out_v = vp;
    }
}
