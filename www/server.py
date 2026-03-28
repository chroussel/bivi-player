#!/usr/bin/env python3
"""HTTP server with COOP/COEP headers for SharedArrayBuffer (required for WASM threads)."""
import http.server
import sys

class Handler(http.server.SimpleHTTPRequestHandler):
    def end_headers(self):
        self.send_header("Cross-Origin-Opener-Policy", "same-origin")
        self.send_header("Cross-Origin-Embedder-Policy", "require-corp")
        super().end_headers()

port = int(sys.argv[1]) if len(sys.argv) > 1 else 8080
print(f"Serving on http://localhost:{port} (with COOP/COEP headers)")
http.server.HTTPServer(("localhost", port), Handler).serve_forever()
