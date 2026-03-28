# Bivi Player

A web video player that plays any format directly in the browser — no transcoding, no server-side processing. Drop in a URL and it just works.

Bivi demuxes containers and decodes video entirely client-side using Rust/WASM and libde265, meaning HEVC/H.265 content plays natively in browsers that have never supported it.

## How it works

```
Video file (MP4/MKV)
    ↓  HTTP Range requests (fetch only what's needed)
Streaming demuxer (Rust/WASM)
    ↓  Raw NAL units — no re-encoding
HEVC decoder (libde265/WASM)
    ↓  YUV420 frames
WebGL renderer (GPU color conversion)
    ↓
Screen
```

1. **Streaming demux** — Rust/WASM parses MP4 (moov/mdat) or MKV (EBML) containers on the fly, extracting encoded video samples via HTTP Range requests without downloading the full file.
2. **HEVC decode** — Encoded NAL units are sent to a Web Worker running libde265 (compiled to WASM via Emscripten with 8 threads), keeping the main thread free.
3. **WebGL render** — Decoded YUV420 frames are uploaded as GPU textures and converted to RGB in a fragment shader. No CPU-side color conversion.
4. **Audio** — AAC tracks are decoded through the browser's WebCodecs AudioDecoder and scheduled against the playback clock.
5. **Subtitles** — ASS/SSA tracks embedded in MKV are parsed and rendered as overlays.

## Features

- **HEVC/H.265 playback** in any modern browser
- **MP4 and MKV** container support
- **Streaming** — plays while downloading via Range requests
- **Multi-track audio** and **ASS subtitles** (MKV)
- **Keyboard controls** — play/pause, arrow-key seeking with debounce
- **Variable playback speed**
- **Seek to keyframe** with B-frame reordering
- **Web Component** — use as `<hevc-player src="video.mp4"></hevc-player>`

## Quick start

### Prerequisites

- Rust + wasm-pack
- Emscripten SDK (for libde265)
- Make

### Build

```bash
make all
```

This produces:

| Artifact | Description |
|---|---|
| `dist/pkg/videoplayer.js` + `.wasm` | Rust demuxer/renderer (wasm-pack) |
| `dist/codec-hevc.js` + `.wasm` | HEVC decoder (Emscripten) |
| `dist/hevc-player.js` | Web Component + player core |

### Usage

Serve the `dist/` directory and include the player in your page:

```html
<script type="module" src="hevc-player.js"></script>

<hevc-player src="https://example.com/video.mp4"></hevc-player>
```

### Examples

```bash
make examples/lib   # copies build artifacts into examples/
cd examples && python3 -m http.server 8080
```

Then open `http://localhost:8080` in your browser.

## Testing

```bash
make test-unit    # 26 Rust unit tests (demuxer, clock, subtitles, format detection)
make test-e2e     # 35 E2E tests (requires Playwright + Chromium)
```

## Architecture

```
rust/src/
├── session.rs           # Main WASM API — owns loader + demuxer, drives buffering
├── stream_loader.rs     # HTTP Range requests, format detection
├── media_source.rs      # Unified interface over MP4/MKV demuxers
├── streaming.rs         # MP4 streaming demuxer (moov parsing, sample tables)
├── streaming_mkv.rs     # MKV streaming demuxer wrapper
├── matroska/            # EBML parser (VINTs, element IDs, SimpleBlock extraction)
├── demuxer.rs           # Low-level MP4 box parsing
├── renderer.rs          # WebGL YUV→RGB shader
├── clock.rs             # Playback timing with speed control
├── frame_buffer.rs      # Ordered frame queue with B-frame reordering
├── subtitle_engine.rs   # ASS/SSA parser
└── format_detect.rs     # Magic-byte format detection

rust/codecs/hevc/
├── wrapper.c            # libde265 C API wrapper
└── build.sh             # Emscripten build script

js/src/
├── hevc-player.js       # <hevc-player> Web Component (Shadow DOM, controls UI)
├── player-core.js       # Core playback engine (fetch loop, decode scheduling)
├── decode-worker.js     # Web Worker for off-thread HEVC decoding
└── debounce.js          # Seek input debouncing
```

## Supported formats

| | Containers | Codecs |
|---|---|---|
| **Video** | MP4, MKV | HEVC/H.265 (Main, Main 10) |
| **Audio** | MP4, MKV | AAC |
| **Subtitles** | MKV | ASS/SSA |

## Tech stack

- **Rust + wasm-bindgen** — streaming demuxer, WebGL renderer, playback clock
- **C/C++ + Emscripten** — libde265 HEVC decoder (multi-threaded WASM)
- **JavaScript** — Web Component UI, Web Worker decode bridge, WebCodecs audio
- **WebGL** — GPU-accelerated YUV→RGB rendering
- **Playwright** — E2E browser testing

## License

See [LICENSE](LICENSE) for details.
