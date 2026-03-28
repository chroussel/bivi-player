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

// ── Frame info struct (written by decoder, read by JS) ──

typedef struct {
    int32_t w;
    int32_t h;
    int32_t bpp;
    int64_t pts;
    uint32_t y_ptr;
    int32_t y_stride;
    uint32_t u_ptr;
    int32_t u_stride;
    uint32_t v_ptr;
    int32_t v_stride;
} frame_info;

#define MAX_PENDING_FRAMES 16
static frame_info pending_frames[MAX_PENDING_FRAMES];
static int num_pending_frames = 0;

// Collect all available decoded pictures into pending_frames array.
// Returns number of frames collected.
EMSCRIPTEN_KEEPALIVE
int decoder_collect_frames(void) {
    if (!ctx) return 0;
    num_pending_frames = 0;
    const struct de265_image* img;
    while (num_pending_frames < MAX_PENDING_FRAMES && (img = de265_get_next_picture(ctx)) != NULL) {
        frame_info* f = &pending_frames[num_pending_frames];
        f->w = de265_get_image_width(img, 0);
        f->h = de265_get_image_height(img, 0);
        f->bpp = de265_get_bits_per_pixel(img, 0);
        f->pts = de265_get_image_PTS(img);

        int stride;
        f->y_ptr = (uint32_t)(uintptr_t)de265_get_image_plane(img, 0, &stride);
        f->y_stride = stride;
        f->u_ptr = (uint32_t)(uintptr_t)de265_get_image_plane(img, 1, &stride);
        f->u_stride = stride;
        f->v_ptr = (uint32_t)(uintptr_t)de265_get_image_plane(img, 2, &stride);
        f->v_stride = stride;

        num_pending_frames++;
    }
    return num_pending_frames;
}

// Get pointer to the pending_frames array (for JS to read via HEAP views)
EMSCRIPTEN_KEEPALIVE
frame_info* decoder_get_frame_info(void) {
    return pending_frames;
}

// Size of frame_info struct (for JS to step through the array)
EMSCRIPTEN_KEEPALIVE
int decoder_frame_info_size(void) {
    return sizeof(frame_info);
}

// ── Push parameter sets from hvcC data ──

EMSCRIPTEN_KEEPALIVE
int decoder_push_parameter_sets(const uint8_t* hvcc, int hvcc_len) {
    if (!ctx || hvcc_len < 23) return -1;

    int pos = 22;
    int num_arrays = hvcc[pos++];
    int nal_count = 0;

    for (int a = 0; a < num_arrays && pos + 3 <= hvcc_len; a++) {
        pos++; // array_completeness | reserved | nal_unit_type
        int num_nalus = (hvcc[pos] << 8) | hvcc[pos + 1];
        pos += 2;
        for (int n = 0; n < num_nalus && pos + 2 <= hvcc_len; n++) {
            int nalu_len = (hvcc[pos] << 8) | hvcc[pos + 1];
            pos += 2;
            if (pos + nalu_len > hvcc_len) break;

            de265_error err = de265_push_data(ctx, start_code, 4, 0, NULL);
            if (err == DE265_OK)
                err = de265_push_data(ctx, hvcc + pos, nalu_len, 0, NULL);
            if (err == DE265_OK)
                de265_push_end_of_NAL(ctx);

            nal_count++;
            pos += nalu_len;
        }
    }

    // Process parameter sets
    int more = 1;
    while (more > 0) {
        de265_error err = de265_decode(ctx, &more);
        if (err == DE265_ERROR_WAITING_FOR_INPUT_DATA) break;
        if (err != DE265_OK) break;
    }

    return nal_count;
}

#ifdef __cplusplus
}
#endif
