#!/usr/bin/env python3
"""Local OpenID Connect provider for the Phase 8 SSO end-to-end test.

Not a product component. It signs RS256 ID tokens with a throwaway key and
implements only discovery, authorize, token and JWKS. Run it on localhost.
"""

import base64
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import time
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(os.environ.get("MOCK_IDP_PORT", "11540"))
ISSUER = os.environ.get("MOCK_IDP_ISSUER", f"http://localhost:{PORT}")
CLIENT_ID = os.environ.get("MOCK_IDP_CLIENT_ID", "systivra-eval")
CLIENT_SECRET = os.environ.get("MOCK_IDP_CLIENT_SECRET", "eval-secret")
SUBJECT = os.environ.get("MOCK_IDP_SUBJECT", "sso-eval-subject")
EMAIL = os.environ.get("MOCK_IDP_EMAIL", "sso-eval@example.test")
BAD_NONCE = os.environ.get("MOCK_IDP_BAD_NONCE") == "1"
KEY_DIR = os.environ.get("MOCK_IDP_KEY_DIR") or tempfile.mkdtemp(prefix="systivra-idp-")

KEY_FILE = os.path.join(KEY_DIR, "key.pem")
CODES = {}


def b64url(raw: bytes) -> str:
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode("ascii")


def ensure_key() -> tuple[str, str]:
    os.makedirs(KEY_DIR, exist_ok=True)
    if not os.path.exists(KEY_FILE):
        subprocess.run(
            [
                "openssl", "genpkey", "-algorithm", "RSA",
                "-pkeyopt", "rsa_keygen_bits:2048", "-out", KEY_FILE,
            ],
            check=True,
            capture_output=True,
        )
    modulus = subprocess.run(
        ["openssl", "rsa", "-in", KEY_FILE, "-noout", "-modulus"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    n = bytes.fromhex(modulus.split("=", 1)[1])
    return b64url(n), b64url((65537).to_bytes(3, "big"))


N, E = ensure_key()


def sign_jwt(claims: dict) -> str:
    header = {"alg": "RS256", "kid": "mock-key-1", "typ": "JWT"}
    signing_input = ".".join(
        b64url(json.dumps(part, separators=(",", ":")).encode())
        for part in (header, claims)
    )
    signature = subprocess.run(
        ["openssl", "dgst", "-sha256", "-sign", KEY_FILE],
        input=signing_input.encode(),
        check=True,
        capture_output=True,
    ).stdout
    return f"{signing_input}.{b64url(signature)}"


def id_token(nonce: str) -> str:
    now = int(time.time())
    return sign_jwt(
        {
            "iss": ISSUER,
            "aud": CLIENT_ID,
            "sub": SUBJECT,
            "sid": "idp-session-1",
            "iat": now,
            "exp": now + 300,
            "nonce": "wrong-nonce" if BAD_NONCE else nonce,
            "email": EMAIL,
            "email_verified": True,
            "groups": ["systivra-admins"],
            "preferred_username": "sso-eval",
        }
    )


class Handler(BaseHTTPRequestHandler):
    server_version = "MockOidc/1"

    def log_message(self, *_args):
        pass

    def json_response(self, status: int, body: dict):
        raw = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def redirect(self, location: str):
        self.send_response(302)
        self.send_header("Location", location)
        self.end_headers()

    def do_GET(self):
        parsed = urllib.parse.urlparse(self.path)
        query = urllib.parse.parse_qs(parsed.query)

        if parsed.path == "/healthz":
            self.json_response(200, {"status": "ok"})
            return
        if parsed.path == "/.well-known/openid-configuration":
            self.json_response(
                200,
                {
                    "issuer": ISSUER,
                    "authorization_endpoint": f"{ISSUER}/authorize",
                    "token_endpoint": f"{ISSUER}/token",
                    "jwks_uri": f"{ISSUER}/jwks",
                    "response_types_supported": ["code"],
                    "subject_types_supported": ["public"],
                    "id_token_signing_alg_values_supported": ["RS256"],
                    "scopes_supported": ["openid", "email", "profile"],
                    "token_endpoint_auth_methods_supported": [
                        "client_secret_basic",
                        "client_secret_post",
                    ],
                    "code_challenge_methods_supported": ["S256"],
                },
            )
            return
        if parsed.path == "/jwks":
            self.json_response(
                200,
                {
                    "keys": [
                        {
                            "kty": "RSA",
                            "kid": "mock-key-1",
                            "use": "sig",
                            "alg": "RS256",
                            "n": N,
                            "e": E,
                        }
                    ]
                },
            )
            return
        if parsed.path == "/authorize":
            if query.get("client_id", [""])[0] != CLIENT_ID:
                self.json_response(400, {"error": "invalid_client"})
                return
            if query.get("response_type", [""])[0] != "code":
                self.json_response(400, {"error": "unsupported_response_type"})
                return
            scope = query.get("scope", [""])[0].split()
            if "openid" not in scope:
                self.json_response(400, {"error": "invalid_scope"})
                return
            redirect_uri = query.get("redirect_uri", [""])[0]
            if not redirect_uri:
                self.json_response(400, {"error": "invalid_request"})
                return
            code = b64url(os.urandom(24))
            CODES[code] = {
                "redirect_uri": redirect_uri,
                "nonce": query.get("nonce", [""])[0],
                "challenge": query.get("code_challenge", [""])[0],
                "method": query.get("code_challenge_method", ["S256"])[0],
                "created": time.time(),
                "used": False,
            }
            target = redirect_uri + "?" + urllib.parse.urlencode(
                {"code": code, "state": query.get("state", [""])[0]}
            )
            self.redirect(target)
            return

        self.json_response(404, {"error": "not_found"})

    def do_POST(self):
        parsed = urllib.parse.urlparse(self.path)
        if parsed.path == "/logout-token":
            length = int(self.headers.get("Content-Length", "0"))
            raw = self.rfile.read(length) if length else b"{}"
            try:
                body = json.loads(raw or b"{}")
            except json.JSONDecodeError:
                body = {}
            now = int(time.time())
            claims = {
                "iss": ISSUER,
                "aud": CLIENT_ID,
                "sub": body.get("sub", SUBJECT),
                "sid": body.get("sid", "idp-session-1"),
                "iat": now,
                "exp": now + 300,
                "jti": body.get("jti") or b64url(os.urandom(16)),
                "events": {
                    "http://schemas.openid.net/event/backchannel-logout": {}
                },
            }
            self.json_response(200, {"logout_token": sign_jwt(claims)})
            return

        if parsed.path != "/token":
            self.json_response(404, {"error": "not_found"})
            return

        length = int(self.headers.get("Content-Length", "0"))
        form = urllib.parse.parse_qs(self.rfile.read(length).decode())
        client_id = form.get("client_id", [""])[0]
        client_secret = form.get("client_secret", [""])[0]

        auth = self.headers.get("Authorization", "")
        if auth.startswith("Basic "):
            try:
                decoded = base64.b64decode(auth[6:]).decode()
                client_id, client_secret = decoded.split(":", 1)
            except Exception:
                self.json_response(401, {"error": "invalid_client"})
                return
        if client_id != CLIENT_ID or client_secret != CLIENT_SECRET:
            self.json_response(401, {"error": "invalid_client"})
            return

        code = form.get("code", [""])[0]
        entry = CODES.get(code)
        if not entry or entry["used"] or time.time() - entry["created"] > 300:
            self.json_response(400, {"error": "invalid_grant"})
            return
        if form.get("redirect_uri", [""])[0] != entry["redirect_uri"]:
            self.json_response(400, {"error": "invalid_grant"})
            return

        verifier = form.get("code_verifier", [""])[0]
        challenge = b64url(hashlib.sha256(verifier.encode()).digest())
        if entry["method"] != "S256" or challenge != entry["challenge"]:
            self.json_response(400, {"error": "invalid_grant"})
            return

        entry["used"] = True
        self.json_response(
            200,
            {
                "access_token": b64url(os.urandom(24)),
                "token_type": "Bearer",
                "expires_in": 300,
                "id_token": id_token(entry["nonce"]),
                "scope": "openid email profile",
            },
        )


def main():
    server = ThreadingHTTPServer(("127.0.0.1", PORT), Handler)
    print(f"mock oidc listening on {ISSUER}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        sys.exit(0)
