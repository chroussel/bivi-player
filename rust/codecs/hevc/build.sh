#!/bin/bash
set -e
cd "$(dirname "$0")/../.."

export PATH="/usr/lib/emscripten:$PATH"

LIBDE265_DIR=../vendor/libde265/libde265
SOURCES=$(ls $LIBDE265_DIR/*.cc | grep -v 'en265\|visualize\|image-io')

echo "Building codec-hevc..."
em++ -O3 -msimd128 \
  -pthread \
  -I ../vendor/libde265 \
  -I codecs/include \
  -DHAVE_STDINT_H \
  -std=c++17 \
  -fexceptions \
  -s DISABLE_EXCEPTION_CATCHING=0 \
  -s WASM=1 \
  -s MODULARIZE=1 \
  -s EXPORT_NAME="createCodec" \
  -s EXPORTED_FUNCTIONS="[ \
    '_codec_init', '_codec_free', '_codec_reset', \
    '_codec_configure', '_codec_push_sample', \
    '_codec_flush', '_codec_decode', \
    '_codec_collect_frames', '_codec_get_frames', '_codec_frame_size', \
    '_malloc', '_free' \
  ]" \
  -s EXPORTED_RUNTIME_METHODS="['HEAPU8','getValue']" \
  -s ALLOW_MEMORY_GROWTH=1 \
  -s INITIAL_MEMORY=134217728 \
  -s STACK_SIZE=2097152 \
  -s NO_FILESYSTEM=1 \
  -s ENVIRONMENT=web,worker \
  -s EXPORT_ES6=1 \
  -s PTHREAD_POOL_SIZE=8 \
  $SOURCES \
  codecs/hevc/wrapper.c \
  -o ../dist/codec-hevc.js

echo "Done: dist/codec-hevc.js + dist/codec-hevc.wasm"
