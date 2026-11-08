#!/usr/bin/env python3
"""Local SIEM receiver for the audit forwarder test."""

import hashlib
import hmac
import json
import os
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(os.environ.get("MOCK_SIEM_PORT", "11550"))
OUT = os.environ.get("MOCK_SIEM_OUT", "/tmp/systivra-siem.ndjson")
SECRET = os.environ.get("MOCK_SIEM_SECRET", "siem-eval-secret")


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_GET(self):
        if self.path == "/healthz":
            raw = b'{"status":"ok"}'
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(raw)))
            self.end_headers()
            self.wfile.write(raw)
            return
        self.send_response(404)
        self.end_headers()

    def do_POST(self):
        if self.path != "/audit":
            self.send_response(404)
            self.end_headers()
            return
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length)
        if SECRET:
            expected = hmac.new(SECRET.encode(), body, hashlib.sha256).hexdigest()
            presented = self.headers.get("X-Systivra-Signature", "")
            if presented != f"sha256={expected}":
                self.send_response(401)
                self.end_headers()
                return
        with open(OUT, "ab") as handle:
            handle.write(body)
        self.send_response(204)
        self.end_headers()


def main():
    server = ThreadingHTTPServer(("127.0.0.1", PORT), Handler)
    print(f"mock siem listening on {PORT}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        sys.exit(0)
