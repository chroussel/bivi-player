.PHONY: all rust codecs js clean serve

EMCC_PATH ?= /usr/lib/emscripten
export PATH := $(EMCC_PATH):$(PATH)

all: rust codecs js

# ── Rust WASM ──

rust:
	cd rust && wasm-pack build --target web --release
	rm -rf dist/pkg && cp -r rust/pkg dist/pkg

# ── Codecs (emscripten) ──

codecs: hevc

hevc:
	./rust/codecs/hevc/build.sh

# ── JS library ──

js:
	cp js/src/*.js dist/
	@# Worker needs to be in same directory as library
	cp js/src/decode-worker.js dist/

# ── Serve examples ──

serve: all
	python3 examples/server.py 8081

# ── Clean ──

clean:
	cd rust && cargo clean
	rm -rf rust/pkg dist
