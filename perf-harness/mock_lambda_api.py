#!/usr/bin/env python3
"""Minimal stand-in for the Lambda Extensions API + Runtime API.

Used by bench.py to run the real extension binary end-to-end locally,
without deploying anything to AWS. Handles just enough of the protocol for
the extension to complete registration and start waiting for the next event:

  POST /2020-01-01/extension/register  -> 200 + Lambda-Extension-Identifier
  PUT  /2022-07-01/telemetry           -> 200
  GET  /2020-01-01/extension/event/next -> hangs (no invocations to deliver)
"""
import http.server
import json
import sys
import time


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        pass

    def _send_json(self, body: bytes, headers: dict[str, str] | None = None) -> None:
        self.send_response(200)
        for key, value in (headers or {}).items():
            self.send_header(key, value)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        if self.path.endswith("/extension/register"):
            self._send_json(
                json.dumps({"accountId": "123456789012"}).encode(),
                {"Lambda-Extension-Identifier": "test-ext-id"},
            )
        else:
            self.send_response(404)
            self.end_headers()

    def do_PUT(self):
        if "/telemetry" in self.path:
            self._send_json(b"{}")
        else:
            self.send_response(404)
            self.end_headers()

    def do_GET(self):
        if "/event/next" in self.path:
            # Simulate waiting for the next invocation -- there isn't one.
            time.sleep(3600)
        else:
            self.send_response(404)
            self.end_headers()


def serve(port: int) -> http.server.ThreadingHTTPServer:
    server = http.server.ThreadingHTTPServer(("127.0.0.1", port), Handler)
    return server


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 9001
    serve(port).serve_forever()
