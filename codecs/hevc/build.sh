#!/bin/bash
set -e
cd "$(dirname "$0")/../.."

export PATH="/usr/lib/emscripten:$PATH"

LIBDE265_DIR=vendor/libde265/libde265
SOURCES=$(ls $LIBDE265_DIR/*.cc | grep -v 'en265\|visualize\|image-io')

# Build Rust codec-wrapper for emscripten
echo "Building Rust codec-wrapper..."
(cd codecs/codec-wrapper && \
    EMCC_CFLAGS="-s ERROR_ON_UNDEFINED_SYMBOLS=0" \
    cargo build --target wasm32-unknown-emscripten --release 2>&1 | grep -v "^warning")

RUST_LIB=codecs/codec-wrapper/target/wasm32-unknown-emscripten/release/libcodec_wrapper.a

echo "Building codec-hevc..."
em++ -O3 -msimd128 \
  -pthread \
  -I vendor/libde265 \
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
  -s ALLOW_MEMORY_GROWTH=0 \
  -s INITIAL_MEMORY=536870912 \
  -s STACK_SIZE=2097152 \
  -s NO_FILESYSTEM=1 \
  -s ENVIRONMENT=web,worker \
  -s EXPORT_ES6=1 \
  -s PTHREAD_POOL_SIZE=8 \
  $SOURCES \
  codecs/hevc/wrapper.c \
  "$RUST_LIB" \
  -o www/codec-hevc.js

echo "Done: www/codec-hevc.js + www/codec-hevc.wasm"
