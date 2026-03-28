.PHONY: all rust codecs hevc clean serve

EMCC_PATH ?= /usr/lib/emscripten
export PATH := $(EMCC_PATH):$(PATH)

all: rust codecs

# ── Rust WASM (demuxer + player core) ──

rust:
	wasm-pack build --target web --release
	cp -r pkg www/pkg

# ── Codecs ──

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
