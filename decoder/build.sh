#!/bin/bash
set -e
cd "$(dirname "$0")/.."

export PATH="/usr/lib/emscripten:$PATH"

LIBDE265_DIR=vendor/libde265/libde265
OUT_DIR=www

# Decoder source files (exclude encoder, visualize, image-io)
SOURCES=$(ls $LIBDE265_DIR/*.cc | grep -v 'en265\|visualize\|image-io')

echo "Compiling libde265 + wrapper to WASM (threaded)..."
em++ -O3 -msimd128 \
  -pthread \
  -I vendor/libde265 \
  -DHAVE_STDINT_H \
  -std=c++17 \
  -fexceptions \
  -s DISABLE_EXCEPTION_CATCHING=0 \
  -s WASM=1 \
  -s MODULARIZE=1 \
  -s EXPORT_NAME="createDecoder" \
  -s EXPORTED_FUNCTIONS="[ \
    '_decoder_init', \
    '_decoder_free', \
    '_decoder_reset', \
    '_decoder_push_nal', \
    '_decoder_push_mp4_sample', \
    '_decoder_flush', \
    '_decoder_decode', \
    '_decoder_get_next_picture', \
    '_decoder_get_width', \
    '_decoder_get_height', \
    '_decoder_get_bits_per_pixel', \
    '_decoder_get_picture_pts', \
    '_decoder_get_chroma_format', \
    '_decoder_get_plane', \
    '_decoder_yuv_to_rgba', \
    '_malloc', \
    '_free' \
  ]" \
  -s EXPORTED_RUNTIME_METHODS="['ccall','cwrap','HEAPU8','HEAP32','getValue']" \
  -s ALLOW_MEMORY_GROWTH=0 \
  -s INITIAL_MEMORY=536870912 \
  -s STACK_SIZE=2097152 \
  -s NO_FILESYSTEM=1 \
  -s ENVIRONMENT=web,worker \
  -s EXPORT_ES6=1 \
  -s PTHREAD_POOL_SIZE=4 \
  $SOURCES \
  decoder/wrapper.c \
  -o "$OUT_DIR/decoder.js"

echo "Done! Output: $OUT_DIR/decoder.js + $OUT_DIR/decoder.wasm + $OUT_DIR/decoder.worker.js"
