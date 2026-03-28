EMCC_PATH ?= /usr/lib/emscripten
export PATH := $(EMCC_PATH):$(PATH)

RUST_SRC := $(shell find rust/src -name '*.rs')
JS_SRC := $(wildcard js/src/*.js)

all: dist/pkg/videoplayer.js dist/codec-hevc.js dist/hevc-player.js examples/lib

# ── Rust WASM ──

dist/pkg/videoplayer.js: $(RUST_SRC) rust/Cargo.toml
	cd rust && wasm-pack build --target web --release
	rm -rf dist/pkg && cp -r rust/pkg dist/pkg

# ── Codecs (emscripten) ──

dist/codec-hevc.js: rust/codecs/hevc/wrapper.c rust/codecs/include/codec_api.h
	./rust/codecs/hevc/build.sh

# ── JS library ──

dist/hevc-player.js: $(JS_SRC)
	@mkdir -p dist
	cp js/src/*.js dist/

examples/lib:
	ln -sfn ../dist examples/lib

# ── Serve examples ──

.PHONY: serve clean

serve: all
	cd examples && python3 server.py 8081

clean:
	cd rust && cargo clean
	rm -rf rust/pkg dist examples/lib
