#ifndef CODEC_API_H
#define CODEC_API_H

#include <stdint.h>
#include <emscripten/emscripten.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct {
    int32_t  w;
    int32_t  h;
    int32_t  bpp;        /* bits per component: 8, 10, 12 */
    int32_t  pix_fmt;    /* 0=YUV420, 1=YUV422, 2=YUV444, 3=NV12 */
    int64_t  pts;
    uint32_t plane0_ptr;
    int32_t  plane0_stride;
    uint32_t plane1_ptr;
    int32_t  plane1_stride;
    uint32_t plane2_ptr;
    int32_t  plane2_stride;
} codec_frame_t;

#define CODEC_MAX_FRAMES 16

/* Init/destroy */
EMSCRIPTEN_KEEPALIVE int  codec_init(void);
EMSCRIPTEN_KEEPALIVE void codec_free(void);
EMSCRIPTEN_KEEPALIVE void codec_reset(void);

/* Push codec-specific config box (hvcC / avcC / av1C raw bytes) */
EMSCRIPTEN_KEEPALIVE int codec_configure(const uint8_t* data, int len);

/* Push one MP4 sample. nal_length_size=0 for non-NAL codecs (AV1). */
EMSCRIPTEN_KEEPALIVE int codec_push_sample(const uint8_t* data, int len,
                                           int nal_length_size, int64_t pts);

/* Signal end of stream */
EMSCRIPTEN_KEEPALIVE int codec_flush(void);

/* Run one decode step. Returns: 1=more, 0=needs input, -1=error */
EMSCRIPTEN_KEEPALIVE int codec_decode(void);

/* Collect decoded frames. Returns count (0..CODEC_MAX_FRAMES). */
EMSCRIPTEN_KEEPALIVE int codec_collect_frames(void);

/* Pointer to frame info array */
EMSCRIPTEN_KEEPALIVE codec_frame_t* codec_get_frames(void);

/* Size of codec_frame_t for JS to iterate */
EMSCRIPTEN_KEEPALIVE int codec_frame_size(void);

#ifdef __cplusplus
}
#endif

#endif
