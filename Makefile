.PHONY: all rust codecs hevc clean serve

EMCC_PATH ?= /usr/lib/emscripten
export PATH := $(EMCC_PATH):$(PATH)

all: rust codecs

# ── Module A: Rust WASM (demuxer, frame buffer, clock, renderer) ──

rust:
	wasm-pack build --target web --release
	rm -rf www/pkg && cp -r pkg www/pkg

# ── Module B: Codec WASM (emscripten, implementing codec_api.h) ──

codecs: hevc

hevc:
	./codecs/hevc/build.sh

# ── Serve ──

serve:
	cd www && python3 server.py 8081

# ── Clean ──

clean:
	cargo clean
	rm -rf pkg www/pkg
	rm -f www/codec-*.js www/codec-*.wasm
