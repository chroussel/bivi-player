/// Convert a 10/12-bit YUV420 frame to 8-bit with stride stripping.
/// Writes into pre-allocated output slices.
pub fn convert_10bit_to_8bit(
    y_src: &[u8],
    u_src: &[u8],
    v_src: &[u8],
    y_dst: &mut [u8],
    u_dst: &mut [u8],
    v_dst: &mut [u8],
    width: usize,
    height: usize,
    y_stride: usize,
    u_stride: usize,
    v_stride: usize,
    bpp: u32,
) {
    let shift = bpp - 8;
    let cw = width >> 1;
    let ch = height >> 1;

    // Y plane
    for r in 0..height {
        let src_row = &y_src[r * y_stride..];
        let dst_row = &mut y_dst[r * width..r * width + width];
        for c in 0..width {
            let val = (src_row[c * 2] as u32) | ((src_row[c * 2 + 1] as u32) << 8);
            dst_row[c] = (val >> shift) as u8;
        }
    }

    // U plane
    for r in 0..ch {
        let src_row = &u_src[r * u_stride..];
        let dst_row = &mut u_dst[r * cw..r * cw + cw];
        for c in 0..cw {
            let val = (src_row[c * 2] as u32) | ((src_row[c * 2 + 1] as u32) << 8);
            dst_row[c] = (val >> shift) as u8;
        }
    }

    // V plane
    for r in 0..ch {
        let src_row = &v_src[r * v_stride..];
        let dst_row = &mut v_dst[r * cw..r * cw + cw];
        for c in 0..cw {
            let val = (src_row[c * 2] as u32) | ((src_row[c * 2 + 1] as u32) << 8);
            dst_row[c] = (val >> shift) as u8;
        }
    }
}

/// Strip stride from 8-bit YUV420 planes (copy only width bytes per row).
pub fn strip_stride(
    y_src: &[u8],
    u_src: &[u8],
    v_src: &[u8],
    y_dst: &mut [u8],
    u_dst: &mut [u8],
    v_dst: &mut [u8],
    width: usize,
    height: usize,
    y_stride: usize,
    u_stride: usize,
    v_stride: usize,
) {
    let cw = width >> 1;
    let ch = height >> 1;

    if y_stride == width {
        y_dst[..width * height].copy_from_slice(&y_src[..width * height]);
    } else {
        for r in 0..height {
            y_dst[r * width..(r + 1) * width]
                .copy_from_slice(&y_src[r * y_stride..r * y_stride + width]);
        }
    }

    if u_stride == cw {
        u_dst[..cw * ch].copy_from_slice(&u_src[..cw * ch]);
        v_dst[..cw * ch].copy_from_slice(&v_src[..cw * ch]);
    } else {
        for r in 0..ch {
            u_dst[r * cw..(r + 1) * cw]
                .copy_from_slice(&u_src[r * u_stride..r * u_stride + cw]);
            v_dst[r * cw..(r + 1) * cw]
                .copy_from_slice(&v_src[r * v_stride..r * v_stride + cw]);
        }
    }
}
