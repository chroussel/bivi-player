#include <libde265/de265.h>
#include <emscripten/emscripten.h>
#include <stdlib.h>
#include <string.h>

#ifdef __cplusplus
extern "C" {
#endif

static de265_decoder_context* ctx = NULL;

EMSCRIPTEN_KEEPALIVE
int decoder_init(void) {
    ctx = de265_new_decoder();
    if (!ctx) return -1;
    // Use multiple threads for parallel CTB row decoding
    de265_start_worker_threads(ctx, 8);
    return 0;
}

EMSCRIPTEN_KEEPALIVE
void decoder_free(void) {
    if (ctx) {
        de265_free_decoder(ctx);
        ctx = NULL;
    }
}

EMSCRIPTEN_KEEPALIVE
void decoder_reset(void) {
    if (ctx) {
        de265_reset(ctx);
    }
}

static const uint8_t start_code[] = {0, 0, 0, 1};

// Push a single NAL unit as Annex B (prepends start code, uses de265_push_data)
EMSCRIPTEN_KEEPALIVE
int decoder_push_nal(const uint8_t* data, int length, int64_t pts) {
    if (!ctx) return -1;
    de265_error err;
    err = de265_push_data(ctx, start_code, 4, pts, NULL);
    if (err != DE265_OK) return (int)err;
    err = de265_push_data(ctx, data, length, pts, NULL);
    if (err != DE265_OK) return (int)err;
    de265_push_end_of_NAL(ctx);
    return 0;
}

// Push MP4 sample: converts length-prefixed NALs to Annex B and pushes via de265_push_data
EMSCRIPTEN_KEEPALIVE
int decoder_push_mp4_sample(const uint8_t* data, int length, int nal_length_size, int64_t pts) {
    if (!ctx) return -1;
    int pos = 0;
    while (pos + nal_length_size <= length) {
        int nal_len = 0;
        for (int i = 0; i < nal_length_size; i++) {
            nal_len = (nal_len << 8) | data[pos + i];
        }
        pos += nal_length_size;
        if (pos + nal_len > length) break;

        de265_error err;
        err = de265_push_data(ctx, start_code, 4, pts, NULL);
        if (err != DE265_OK) return (int)err;
        err = de265_push_data(ctx, data + pos, nal_len, pts, NULL);
        if (err != DE265_OK) return (int)err;
        de265_push_end_of_NAL(ctx);

        pos += nal_len;
    }
    de265_push_end_of_frame(ctx);
    return 0;
}

// Signal end of stream
EMSCRIPTEN_KEEPALIVE
int decoder_flush(void) {
    if (!ctx) return -1;
    return (int)de265_flush_data(ctx);
}

// Decode pending data.
// Returns: 0 = needs more input, 1 = has more to process, -1 = error
EMSCRIPTEN_KEEPALIVE
int decoder_decode(void) {
    if (!ctx) return 0;
    int more = 0;
    de265_error err = de265_decode(ctx, &more);
    if (err == DE265_ERROR_WAITING_FOR_INPUT_DATA) {
        return 0;  // need more input data, stop calling decode
    }
    if (err != DE265_OK) {
        return -1;
    }
    return more;
}

// Get the next decoded picture. Returns 0 if no picture available.
// Writes plane pointers and strides to output params.
static const struct de265_image* current_img = NULL;

EMSCRIPTEN_KEEPALIVE
int decoder_get_next_picture(void) {
    if (!ctx) return 0;
    current_img = de265_get_next_picture(ctx);
    return current_img ? 1 : 0;
}

EMSCRIPTEN_KEEPALIVE
int decoder_get_width(void) {
    return current_img ? de265_get_image_width(current_img, 0) : 0;
}

EMSCRIPTEN_KEEPALIVE
int decoder_get_height(void) {
    return current_img ? de265_get_image_height(current_img, 0) : 0;
}

EMSCRIPTEN_KEEPALIVE
int decoder_get_bits_per_pixel(void) {
    return current_img ? de265_get_bits_per_pixel(current_img, 0) : 8;
}

EMSCRIPTEN_KEEPALIVE
int64_t decoder_get_picture_pts(void) {
    return current_img ? de265_get_image_PTS(current_img) : 0;
}

EMSCRIPTEN_KEEPALIVE
int decoder_get_chroma_format(void) {
    return current_img ? (int)de265_get_chroma_format(current_img) : 1;
}

// Get Y/U/V plane data pointer and stride for current picture
EMSCRIPTEN_KEEPALIVE
const uint8_t* decoder_get_plane(int channel, int* out_stride) {
    if (!current_img) return NULL;
    return de265_get_image_plane(current_img, channel, out_stride);
}

// Convenience: write RGBA pixels into a provided buffer (handles 8 and 10-bit)
// This runs in WASM so it's reasonably fast.
EMSCRIPTEN_KEEPALIVE
int decoder_yuv_to_rgba(uint8_t* rgba_out) {
    if (!current_img) return -1;

    int w = de265_get_image_width(current_img, 0);
    int h = de265_get_image_height(current_img, 0);
    int bpp = de265_get_bits_per_pixel(current_img, 0);
    enum de265_chroma chroma = de265_get_chroma_format(current_img);

    int y_stride, u_stride, v_stride;
    const uint8_t* y_plane = de265_get_image_plane(current_img, 0, &y_stride);
    const uint8_t* u_plane = de265_get_image_plane(current_img, 1, &u_stride);
    const uint8_t* v_plane = de265_get_image_plane(current_img, 2, &v_stride);

    if (!y_plane || !u_plane || !v_plane) return -1;

    // Chroma subsampling factors
    int chroma_w_shift = (chroma == de265_chroma_444) ? 0 : 1;
    int chroma_h_shift = (chroma == de265_chroma_420) ? 1 : 0;
    int bit_shift = bpp - 8;

    for (int row = 0; row < h; row++) {
        int crow = row >> chroma_h_shift;
        for (int col = 0; col < w; col++) {
            int ccol = col >> chroma_w_shift;
            int Y, U, V;

            if (bpp > 8) {
                // 10/12-bit: samples stored as uint16_t
                Y = ((const uint16_t*)(y_plane + row * y_stride))[col] >> bit_shift;
                U = ((const uint16_t*)(u_plane + crow * u_stride))[ccol] >> bit_shift;
                V = ((const uint16_t*)(v_plane + crow * v_stride))[ccol] >> bit_shift;
            } else {
                Y = y_plane[row * y_stride + col];
                U = u_plane[crow * u_stride + ccol];
                V = v_plane[crow * v_stride + ccol];
            }

            // BT.601 YUV -> RGB
            int C = Y - 16;
            int D = U - 128;
            int E = V - 128;
            int R = (298 * C + 409 * E + 128) >> 8;
            int G = (298 * C - 100 * D - 208 * E + 128) >> 8;
            int B = (298 * C + 516 * D + 128) >> 8;

            if (R < 0) R = 0; if (R > 255) R = 255;
            if (G < 0) G = 0; if (G > 255) G = 255;
            if (B < 0) B = 0; if (B > 255) B = 255;

            int idx = (row * w + col) * 4;
            rgba_out[idx + 0] = R;
            rgba_out[idx + 1] = G;
            rgba_out[idx + 2] = B;
            rgba_out[idx + 3] = 255;
        }
    }
    return 0;
}

#ifdef __cplusplus
}
#endif
