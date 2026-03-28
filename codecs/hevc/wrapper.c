#include "codec_api.h"
#include <libde265/de265.h>
#include <stdlib.h>
#include <string.h>

static de265_decoder_context* ctx = NULL;
static const uint8_t start_code[] = {0, 0, 0, 1};

static codec_frame_t pending[CODEC_MAX_FRAMES];
static int num_pending = 0;

/* Ring buffer of plane copies — survives across collect_frames calls */
#define NUM_PLANE_SLOTS 32
#define MAX_PLANE_BUF (1920 * 1080 * 2)
#define MAX_CHROMA_BUF (960 * 540 * 2)
static uint8_t* plane_bufs[NUM_PLANE_SLOTS][3] = {{0}};
static int plane_bufs_allocated = 0;
static int ring_idx = 0;

static void ensure_plane_bufs(void) {
    if (plane_bufs_allocated) return;
    for (int i = 0; i < NUM_PLANE_SLOTS; i++) {
        plane_bufs[i][0] = (uint8_t*)malloc(MAX_PLANE_BUF);
        plane_bufs[i][1] = (uint8_t*)malloc(MAX_CHROMA_BUF);
        plane_bufs[i][2] = (uint8_t*)malloc(MAX_CHROMA_BUF);
    }
    plane_bufs_allocated = 1;
}

static int next_slot(void) {
    int s = ring_idx;
    ring_idx = (ring_idx + 1) % NUM_PLANE_SLOTS;
    return s;
}

int codec_init(void) {
    ctx = de265_new_decoder();
    if (!ctx) return -1;
    de265_start_worker_threads(ctx, 8);
    return 0;
}

void codec_free(void) {
    if (ctx) { de265_free_decoder(ctx); ctx = NULL; }
}

void codec_reset(void) {
    if (ctx) de265_reset(ctx);
}

int codec_configure(const uint8_t* data, int len) {
    if (!ctx || len < 23) return -1;
    int pos = 22;
    int num_arrays = data[pos++];
    int count = 0;

    for (int a = 0; a < num_arrays && pos + 3 <= len; a++) {
        pos++;
        int num_nalus = (data[pos] << 8) | data[pos + 1];
        pos += 2;
        for (int n = 0; n < num_nalus && pos + 2 <= len; n++) {
            int nalu_len = (data[pos] << 8) | data[pos + 1];
            pos += 2;
            if (pos + nalu_len > len) break;
            de265_push_data(ctx, start_code, 4, 0, NULL);
            de265_push_data(ctx, data + pos, nalu_len, 0, NULL);
            de265_push_end_of_NAL(ctx);
            count++;
            pos += nalu_len;
        }
    }
    /* Process parameter sets */
    int more = 1;
    while (more > 0) {
        de265_error err = de265_decode(ctx, &more);
        if (err == DE265_ERROR_WAITING_FOR_INPUT_DATA) break;
        if (err != DE265_OK) break;
    }
    return count;
}

int codec_push_sample(const uint8_t* data, int len, int nal_length_size, int64_t pts) {
    if (!ctx) return -1;
    int pos = 0;
    while (pos + nal_length_size <= len) {
        int nal_len = 0;
        for (int i = 0; i < nal_length_size; i++)
            nal_len = (nal_len << 8) | data[pos + i];
        pos += nal_length_size;
        if (pos + nal_len > len) break;
        de265_push_data(ctx, start_code, 4, pts, NULL);
        de265_push_data(ctx, data + pos, nal_len, pts, NULL);
        de265_push_end_of_NAL(ctx);
        pos += nal_len;
    }
    de265_push_end_of_frame(ctx);
    return 0;
}

int codec_flush(void) {
    if (!ctx) return -1;
    return (int)de265_flush_data(ctx);
}

int codec_decode(void) {
    if (!ctx) return 0;
    int more = 0;
    de265_error err = de265_decode(ctx, &more);
    if (err == DE265_ERROR_WAITING_FOR_INPUT_DATA) return 0;
    if (err != DE265_OK) return -1;
    return more;
}

int codec_collect_frames(void) {
    if (!ctx) return 0;
    ensure_plane_bufs();
    num_pending = 0;
    const struct de265_image* img;
    while (num_pending < CODEC_MAX_FRAMES && (img = de265_get_next_picture(ctx)) != NULL) {
        codec_frame_t* f = &pending[num_pending];
        f->w = de265_get_image_width(img, 0);
        f->h = de265_get_image_height(img, 0);
        f->bpp = de265_get_bits_per_pixel(img, 0);
        f->pix_fmt = 0; /* always YUV420 from libde265 */
        f->pts = de265_get_image_PTS(img);

        int y_stride, u_stride, v_stride;
        const uint8_t* y = de265_get_image_plane(img, 0, &y_stride);
        const uint8_t* u = de265_get_image_plane(img, 1, &u_stride);
        const uint8_t* v = de265_get_image_plane(img, 2, &v_stride);
        int cw = f->w >> 1;
        int ch = f->h >> 1;

        /* Copy + convert to 8-bit into ring buffer slot */
        int slot = next_slot();
        if (f->bpp > 8) {
            int shift = f->bpp - 8;
            for (int r = 0; r < f->h; r++) {
                const uint16_t* src = (const uint16_t*)(y + r * y_stride);
                uint8_t* dst = plane_bufs[slot][0] + r * f->w;
                for (int c = 0; c < f->w; c++) dst[c] = src[c] >> shift;
            }
            for (int r = 0; r < ch; r++) {
                const uint16_t* su = (const uint16_t*)(u + r * u_stride);
                const uint16_t* sv = (const uint16_t*)(v + r * v_stride);
                uint8_t* du = plane_bufs[slot][1] + r * cw;
                uint8_t* dv = plane_bufs[slot][2] + r * cw;
                for (int c = 0; c < cw; c++) { du[c] = su[c] >> shift; dv[c] = sv[c] >> shift; }
            }
        } else {
            for (int r = 0; r < f->h; r++)
                memcpy(plane_bufs[slot][0] + r * f->w, y + r * y_stride, f->w);
            for (int r = 0; r < ch; r++) {
                memcpy(plane_bufs[slot][1] + r * cw, u + r * u_stride, cw);
                memcpy(plane_bufs[slot][2] + r * cw, v + r * v_stride, cw);
            }
        }

        f->bpp = 8;
        f->plane0_ptr = (uint32_t)(uintptr_t)plane_bufs[slot][0];
        f->plane0_stride = f->w;
        f->plane1_ptr = (uint32_t)(uintptr_t)plane_bufs[slot][1];
        f->plane1_stride = cw;
        f->plane2_ptr = (uint32_t)(uintptr_t)plane_bufs[slot][2];
        f->plane2_stride = cw;
        num_pending++;
    }
    return num_pending;
}

codec_frame_t* codec_get_frames(void) { return pending; }
int codec_frame_size(void) { return sizeof(codec_frame_t); }
