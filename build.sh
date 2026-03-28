#!/bin/bash
set -e

echo "=== Building Rust WASM (MP4 demuxer) ==="
wasm-pack build --target web --release
cp -r pkg www/pkg

echo ""
echo "=== Building libde265 WASM (HEVC decoder) ==="
./decoder/build.sh

echo ""
echo "Done! To run (COOP/COEP headers required for WASM threads):"
echo "  cd www && python3 server.py 8080"
echo "  Open http://localhost:8080"
