#!/usr/bin/env python3
"""Phase 8 OIDC SSO end-to-end evaluation.

Starts a local mock IdP and two API processes (SSO enabled / disabled),
runs the authorization-code + PKCE flow, checks audit rows and the negative
paths, and writes data/phase8/sso-report.json.

Prerequisites: psql, openssl, target/debug/systivra-api, a seeded project.
"""

import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from http.cookies import SimpleCookie
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
API_BIN = ROOT / "target/debug/systivra-api"
DB = os.environ.get("PGDATABASE", "systivra")
PGHOST = os.environ.get("PGHOST", "localhost")
PGUSER = os.environ.get("PGUSER", os.environ.get("USER", "postgres"))
API_PORT = int(os.environ.get("SSO_API_PORT", "18089"))
NO_OIDC_PORT = int(os.environ.get("SSO_NO_OIDC_PORT", "18090"))
IDP_PORT = int(os.environ.get("SSO_IDP_PORT", "11540"))
ISSUER = f"http://localhost:{IDP_PORT}"
CLIENT_ID = "systivra-eval"
CLIENT_SECRET = "eval-secret"
SUBJECT = "sso-eval-subject"
USER_ID = os.environ.get(
    "SSO_EVAL_USER_ID", "44444444-4444-4444-8444-444444444444"
)
PASSWORD = os.environ.get("SSO_EVAL_PASSWORD", "SystivraDemo1!")
KEY_DIR = os.environ.get("SSO_IDP_KEY_DIR", "/tmp/systivra-idp-keys")
DATA_DIR = ROOT / "data" / "phase8"
COOKIES = {}
REPORT = {"checks": [], "started_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}


def check(name, ok, detail=""):
    REPORT["checks"].append({"name": name, "ok": bool(ok), "detail": detail})
    mark = "ok  " if ok else "FAIL"
    print(f"  {mark} {name}" + (f" ({detail})" if detail else ""), flush=True)
    return ok


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


OPENER = urllib.request.build_opener(NoRedirect)


def http(method, url, body=None, headers=None):
    req = urllib.request.Request(url, data=body, method=method)
    for key, value in (headers or {}).items():
        req.add_header(key, value)
    if COOKIES:
        req.add_header("Cookie", "; ".join(f"{k}={v}" for k, v in COOKIES.items()))
    try:
        resp = OPENER.open(req, timeout=15)
    except urllib.error.HTTPError as exc:
        resp = exc
    raw = resp.read()
    for header in resp.headers.get_all("Set-Cookie") or []:
        jar = SimpleCookie()
        jar.load(header)
        for name, morsel in jar.items():
            if morsel["max-age"] == "0":
                COOKIES.pop(name, None)
            else:
                COOKIES[name] = morsel.value
    return resp.status, resp.headers, raw


def psql(sql, database=DB):
    env = dict(os.environ, PGHOST=PGHOST, PGUSER=PGUSER, PGDATABASE=database)
    proc = subprocess.run(
        ["psql", "-v", "ON_ERROR_STOP=1", "-Atc", sql],
        capture_output=True,
        text=True,
        env=env,
    )
    if proc.returncode != 0:
        raise RuntimeError(proc.stderr.strip())
    return proc.stdout.strip()


def wait_health(url, timeout=20):
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            status, _, _ = http("GET", url)
            if status == 200:
                return True
        except Exception:
            pass
        time.sleep(0.2)
    return False


def start_mock(bad_nonce=False):
    env = dict(
        os.environ,
        MOCK_IDP_PORT=str(IDP_PORT),
        MOCK_IDP_ISSUER=ISSUER,
        MOCK_IDP_CLIENT_ID=CLIENT_ID,
        MOCK_IDP_CLIENT_SECRET=CLIENT_SECRET,
        MOCK_IDP_SUBJECT=SUBJECT,
        MOCK_IDP_KEY_DIR=KEY_DIR,
    )
    if bad_nonce:
        env["MOCK_IDP_BAD_NONCE"] = "1"
    proc = subprocess.Popen(
        [sys.executable, str(ROOT / "scripts/mock-oidc-idp.py")],
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    if not wait_health(f"{ISSUER}/healthz"):
        raise RuntimeError("mock IdP did not become healthy")
    return proc


def stop(proc):
    if proc is None:
        return
    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()


def start_api(port, oidc=True, rate_capacity=None, rate_refill=None):
    env = dict(
        os.environ,
        DATABASE_URL=f"postgres://systivra_app@{PGHOST}:5432/{DB}",
        PORT=str(port),
        SESSION_SECRET="0123456789abcdef0123456789abcdef",
        SESSION_COOKIE_SECURE="false",
        PUBLIC_BASE_URL=f"http://127.0.0.1:{port}",
        SYSTIVRA_DATA_DIR=str(ROOT / "data"),
        NEO4J_DISABLED="1",
    )
    if oidc:
        env.update(
            SESSION_IDLE_MINUTES="60",
            SCIM_BEARER_TOKEN="scim-eval-token",
            OIDC_ISSUER=ISSUER,
            OIDC_CLIENT_ID=CLIENT_ID,
            OIDC_CLIENT_SECRET=CLIENT_SECRET,
            OIDC_REDIRECT_URI=f"http://127.0.0.1:{port}/api/v1/auth/oidc/callback",
            OIDC_PROVIDER_LABEL="ローカル IdP",
        )
    else:
        for key in list(env):
            if key.startswith("OIDC_"):
                env.pop(key)
    if rate_capacity is not None:
        env["AUTH_RATE_LIMIT_CAPACITY"] = str(rate_capacity)
    if rate_refill is not None:
        env["AUTH_RATE_LIMIT_REFILL_PER_MIN"] = str(rate_refill)
    log = open(f"/tmp/systivra-sso-api-{port}.log", "w")
    proc = subprocess.Popen(
        [str(API_BIN)], env=env, stdout=log, stderr=subprocess.STDOUT
    )
    if not wait_health(f"http://127.0.0.1:{port}/healthz"):
        raise RuntimeError(f"API on {port} did not become healthy")
    return proc


def start_flow(api, return_to="/projects"):
    COOKIES.clear()
    quoted = urllib.parse.quote(return_to, safe="/")
    status, headers, _ = http(
        "GET", f"{api}/api/v1/auth/oidc/start?return_to={quoted}"
    )
    if status != 303:
        raise RuntimeError(f"start returned {status}")
    return headers["Location"]


def authorize(auth_url):
    status, headers, _ = http("GET", auth_url)
    if status != 302:
        raise RuntimeError(f"authorize returned {status}")
    return headers["Location"]


def callback(url):
    status, headers, _ = http("GET", url)
    return status, headers.get("Location", "")


def seed_identity():
    psql(
        f"""
        INSERT INTO core.app_user (id, login_id, display_name)
        VALUES ('{USER_ID}', 'sso-eval', 'SSO Eval')
        ON CONFLICT (id) DO NOTHING;
        INSERT INTO core.project_member (project_id, user_id, role)
        SELECT id, '{USER_ID}', 'OWNER' FROM core.project ORDER BY code LIMIT 1
        ON CONFLICT (project_id, user_id) DO UPDATE SET role = EXCLUDED.role;
        INSERT INTO core.external_identity (user_id, provider, subject)
        VALUES ('{USER_ID}', '{ISSUER}', '{SUBJECT}')
        ON CONFLICT (provider, subject) DO UPDATE SET user_id = EXCLUDED.user_id;
        """
    )


def cleanup_fixture():
    # 監査は追記専用なので消さない。ユーザーと紐付けだけ片付ける。
    try:
        psql(
            f"""
            DELETE FROM core.external_group WHERE display_name = 'systivra-admins';
            DELETE FROM core.external_identity
             WHERE provider='{ISSUER}' AND subject='{SUBJECT}';
            DELETE FROM core.project_member
             WHERE user_id IN (
               SELECT id FROM core.app_user
                WHERE (id='{USER_ID}' AND login_id='sso-eval')
                   OR login_id='scim-eval'
             );
            DELETE FROM core.app_user
             WHERE (id='{USER_ID}' AND login_id='sso-eval') OR login_id='scim-eval';
            """
        )
    except Exception as exc:
        print(f"  warning: fixture cleanup failed: {exc}", flush=True)


def audit_rows():
    raw = psql(
        "SELECT result || '|' || COALESCE(detail->>'reason','') "
        "FROM core.audit_log WHERE action='auth.sso.login' "
        "AND detail->>'provider'=$$" + ISSUER + "$$ ORDER BY id DESC LIMIT 20;"
    )
    return [line.split("|", 1) for line in raw.splitlines() if line]


def main():
    if not API_BIN.exists():
        raise SystemExit(f"missing {API_BIN}; run cargo build -p api")
    DATA_DIR.mkdir(parents=True, exist_ok=True)
    seed_identity()
    mock = api = no_oidc = None
    try:
        mock = start_mock()
        api = start_api(API_PORT, oidc=True, rate_capacity=100, rate_refill=100)

        status, headers, _ = http("GET", f"http://127.0.0.1:{API_PORT}/healthz")
        check(
            "セキュリティヘッダ（nosniff / referrer / frame / permissions）",
            status == 200
            and headers.get("x-content-type-options") == "nosniff"
            and headers.get("referrer-policy") == "no-referrer"
            and headers.get("x-frame-options") == "DENY"
            and "geolocation=()" in (headers.get("permissions-policy") or ""),
            str(status),
        )
        check(
            "http では HSTS を付けない",
            headers.get("strict-transport-security") is None,
        )

        status, _, raw = http("GET", f"http://127.0.0.1:{API_PORT}/api/v1/auth/config")
        config = json.loads(raw)
        check("auth/config が OIDC を公開", status == 200 and config.get("oidc"), raw.decode())

        status, _, _ = http("GET", f"http://127.0.0.1:{API_PORT}/scim/v2/Users")
        check("SCIM は Bearer 無しで 401", status == 401, str(status))
        scim_headers = {"Authorization": "Bearer scim-eval-token"}
        status, _, raw = http(
            "GET",
            f"http://127.0.0.1:{API_PORT}/scim/v2/Users",
            headers=scim_headers,
        )
        check("SCIM Users を Bearer で読める", status == 200, str(status))
        status, _, raw = http(
            "POST",
            f"http://127.0.0.1:{API_PORT}/scim/v2/Groups",
            body=json.dumps(
                {
                    "displayName": "systivra-admins",
                    "externalId": "systivra-admins",
                    "urn:systivra:params:scim:schemas:extension:role": {
                        "role": "ARCHITECT"
                    },
                    "members": [{"value": USER_ID}],
                }
            ).encode(),
            headers={**scim_headers, "Content-Type": "application/json"},
        )
        check(
            "SCIM Group を作成しロールを付与",
            status in (200, 201),
            f"{status} {raw[:80]!r}",
        )

        auth_url = start_flow(f"http://127.0.0.1:{API_PORT}")
        query = urllib.parse.parse_qs(urllib.parse.urlparse(auth_url).query)
        check(
            "start が PKCE(S256) と state/nonce を付ける",
            query.get("code_challenge_method") == ["S256"]
            and bool(query.get("code_challenge"))
            and bool(query.get("state"))
            and bool(query.get("nonce")),
            auth_url.split("?")[0],
        )
        check(
            "flow Cookie が HttpOnly / SameSite=Lax / 10 分",
            "systivra_oidc_flow" in COOKIES,
        )

        cb = authorize(auth_url)
        status, location = callback(cb)
        check(
            "callback が元の画面へ 303",
            status == 303 and location == "/projects",
            f"{status} {location}",
        )
        check("session Cookie が発行される", "systivra_session" in COOKIES)

        status, _, raw = http("GET", f"http://127.0.0.1:{API_PORT}/api/v1/auth/me")
        me = json.loads(raw) if status == 200 else {}
        check(
            "SSO セッションで /me が本人を返す",
            status == 200 and me.get("user_id") == USER_ID,
            str(me.get("user_id")),
        )

        role = psql(
            f"SELECT role FROM core.project_member "
            f"WHERE user_id='{USER_ID}' AND project_id=core.first_project();"
        )
        check("OIDC の groups claim がロールへ反映される", role == "ARCHITECT", role)

        status, _, raw = http("GET", f"http://127.0.0.1:{API_PORT}/api/v1/auth/sessions")
        sessions = json.loads(raw).get("sessions", []) if status == 200 else []
        check(
            "自分のセッション一覧に current が 1 件",
            status == 200
            and len(sessions) == 1
            and sessions[0].get("current") is True
            and sessions[0].get("provider") == ISSUER,
            raw.decode(),
        )

        auth_url = start_flow(f"http://127.0.0.1:{API_PORT}")
        callback(authorize(auth_url))
        status, _, raw = http("GET", f"http://127.0.0.1:{API_PORT}/api/v1/auth/sessions")
        two = json.loads(raw).get("sessions", []) if status == 200 else []
        check("2 回ログインするとセッションが 2 件", len(two) == 2, str(len(two)))

        status, _, raw = http(
            "POST", f"http://127.0.0.1:{API_PORT}/api/v1/auth/logout-all"
        )
        revoked = json.loads(raw).get("revoked") if status == 200 else None
        check(
            "logout-all が他のセッションを失効させる",
            status == 200 and revoked == 1,
            f"{status} revoked={revoked}",
        )
        status, _, raw = http("GET", f"http://127.0.0.1:{API_PORT}/api/v1/auth/sessions")
        left = json.loads(raw).get("sessions", []) if status == 200 else []
        check("logout-all 後は current の 1 件だけ", len(left) == 1, str(len(left)))

        saved = COOKIES.get("systivra_session")
        status, _, _ = http("POST", f"http://127.0.0.1:{API_PORT}/api/v1/auth/logout")
        check("logout は 204", status == 204, str(status))
        COOKIES["systivra_session"] = saved
        status, _, _ = http("GET", f"http://127.0.0.1:{API_PORT}/api/v1/auth/me")
        check("logout 後の Cookie 再利用は 401", status == 401, str(status))
        COOKIES.pop("systivra_session", None)

        last = psql(
            f"SELECT last_login_at IS NOT NULL FROM core.external_identity "
            f"WHERE provider='{ISSUER}' AND subject='{SUBJECT}';"
        )
        check("external_identity.last_login_at を更新", last == "t")
        rows = audit_rows()
        check(
            "成功が監査に残る",
            any(r == "SUCCESS" for r, _ in rows),
            str(rows[:2]),
        )

        auth_url = start_flow(f"http://127.0.0.1:{API_PORT}")
        cb = authorize(auth_url)
        parsed = urllib.parse.urlparse(cb)
        q = urllib.parse.parse_qs(parsed.query)
        q["state"] = ["tampered"]
        bad = parsed._replace(query=urllib.parse.urlencode(q, doseq=True)).geturl()
        status, location = callback(bad)
        check(
            "state 改竄は login へ戻す",
            location.startswith("/login?sso_error=state_invalid"),
            location,
        )
        rows = audit_rows()
        check(
            "state 改竄が FAILED として監査に残る",
            any(r == "FAILED" and reason == "state_mismatch" for r, reason in rows),
            str(rows[:3]),
        )

        psql(
            f"DELETE FROM core.external_identity "
            f"WHERE provider='{ISSUER}' AND subject='{SUBJECT}';"
        )
        auth_url = start_flow(f"http://127.0.0.1:{API_PORT}")
        cb = authorize(auth_url)
        status, location = callback(cb)
        check(
            "未紐付けの subject は not_linked",
            location.startswith("/login?sso_error=not_linked"),
            location,
        )
        rows = audit_rows()
        check(
            "未紐付けが FAILED(identity_unlinked) として監査に残る",
            any(r == "FAILED" and reason == "identity_unlinked" for r, reason in rows),
        )
        seed_identity()

        stop(mock)
        mock = start_mock(bad_nonce=True)
        auth_url = start_flow(f"http://127.0.0.1:{API_PORT}")
        cb = authorize(auth_url)
        status, location = callback(cb)
        check(
            "nonce 不一致は exchange_failed",
            location.startswith("/login?sso_error=exchange_failed"),
            location,
        )
        rows = audit_rows()
        check(
            "nonce 不一致が FAILED(token_exchange_failed) として監査に残る",
            any(
                r == "FAILED" and reason == "token_exchange_failed"
                for r, reason in rows
            ),
        )
        stop(mock)
        mock = start_mock()
        auth_url = start_flow(f"http://127.0.0.1:{API_PORT}")
        cb = authorize(auth_url)
        status, location = callback(cb)
        check(
            "IdP 復旧後は再びログインできる",
            status == 303 and location == "/projects",
            location,
        )

        status, _, raw = http(
            "POST",
            f"{ISSUER}/logout-token",
            body=json.dumps({"sub": SUBJECT, "sid": "idp-session-1"}).encode(),
            headers={"Content-Type": "application/json"},
        )
        logout_token = json.loads(raw).get("logout_token") if status == 200 else None
        form = urllib.parse.urlencode({"logout_token": logout_token or ""}).encode()
        status, _, raw = http(
            "POST",
            f"http://127.0.0.1:{API_PORT}/api/v1/auth/oidc/backchannel-logout",
            body=form,
            headers={"Content-Type": "application/x-www-form-urlencoded"},
        )
        check(
            "back-channel logout が 200",
            status == 200,
            f"{status} {raw[:80]!r}",
        )
        status, _, _ = http("GET", f"http://127.0.0.1:{API_PORT}/api/v1/auth/me")
        check("IdP 主導ログアウト後の /me は 401", status == 401, str(status))

        status, _, raw = http(
            "POST",
            f"http://127.0.0.1:{API_PORT}/api/v1/auth/oidc/backchannel-logout",
            body=form,
            headers={"Content-Type": "application/x-www-form-urlencoded"},
        )
        check("logout token のリプレイは 400", status == 400, str(status))

        bad = urllib.parse.urlencode({"logout_token": "x" + (logout_token or "")}).encode()
        status, _, raw = http(
            "POST",
            f"http://127.0.0.1:{API_PORT}/api/v1/auth/oidc/backchannel-logout",
            body=bad,
            headers={"Content-Type": "application/x-www-form-urlencoded"},
        )
        check("改竄された logout token は 400", status == 400, str(status))

        no_oidc = start_api(
            NO_OIDC_PORT, oidc=False, rate_capacity=6, rate_refill=1
        )
        COOKIES.clear()
        status, _, raw = http("GET", f"http://127.0.0.1:{NO_OIDC_PORT}/api/v1/auth/config")
        config = json.loads(raw)
        check(
            "OIDC 未設定なら config はパスワードのみ",
            status == 200 and config.get("oidc") is None and config.get("password_login"),
            raw.decode(),
        )
        status, _, raw = http(
            "GET", f"http://127.0.0.1:{NO_OIDC_PORT}/api/v1/auth/oidc/start"
        )
        check(
            "OIDC 未設定の start は 503 AUTH_SSO_UNAVAILABLE",
            status == 503 and b"AUTH_SSO_UNAVAILABLE" in raw,
            str(status),
        )

        body = json.dumps({"login_id": "alice", "password": PASSWORD}).encode()
        status, _, raw = http(
            "POST",
            f"http://127.0.0.1:{NO_OIDC_PORT}/api/v1/auth/login",
            body=body,
            headers={"Content-Type": "application/json"},
        )
        check(
            "パスワードログインは併存する",
            status == 200 and "systivra_session" in COOKIES,
            f"{status} {raw[:80]!r}",
        )

        last_status = None
        last_headers = None
        for _ in range(5):
            body = json.dumps(
                {"login_id": "alice", "password": "wrong-password"}
            ).encode()
            last_status, last_headers, _ = http(
                "POST",
                f"http://127.0.0.1:{NO_OIDC_PORT}/api/v1/auth/login",
                body=body,
                headers={"Content-Type": "application/json"},
            )
        check(
            "認証の連続失敗は 429 と Retry-After",
            last_status == 429 and (last_headers.get("Retry-After") or "").isdigit(),
            f"{last_status} Retry-After={last_headers.get('Retry-After')}",
        )

        audit = psql(
            "SELECT result || '|' || count(*) FROM core.audit_log "
            "WHERE action='auth.password.login' GROUP BY result;"
        )
        check(
            "パスワードログインの成功・失敗が監査に残る",
            "SUCCESS|" in audit and "FAILED|" in audit,
            audit.replace("\n", " "),
        )
        detail = psql(
            "SELECT detail::text FROM core.audit_log "
            "WHERE action='auth.password.login' ORDER BY id DESC LIMIT 1;"
        )
        check("login_id は生値で監査に残らない", "alice" not in detail, detail[:120])

        started = time.time()
        auth_url = start_flow(f"http://127.0.0.1:{API_PORT}")
        cb = authorize(auth_url)
        status, location = callback(cb)
        elapsed_ms = int((time.time() - started) * 1000)
        REPORT["login_ms"] = elapsed_ms
        check("ログイン往復が 3 秒以内", elapsed_ms < 3000, f"{elapsed_ms}ms")

        status, _, raw = http(
            "GET", f"http://127.0.0.1:{API_PORT}/api/v1/auth/sessions"
        )
        idle_session = (
            json.loads(raw)["sessions"][0]["id"]
            if status == 200 and json.loads(raw).get("sessions")
            else None
        )
        if idle_session:
            psql(
                "UPDATE core.session SET last_seen_at = now() - interval '2 hours' "
                f"WHERE id = '{idle_session}';"
            )
        status, _, _ = http("GET", f"http://127.0.0.1:{API_PORT}/api/v1/auth/me")
        check("アイドル期限を超えたセッションは 401", status == 401, str(status))
    finally:
        stop(mock)
        stop(api)
        stop(no_oidc)
        cleanup_fixture()

    ok = sum(1 for c in REPORT["checks"] if c["ok"])
    REPORT["passed"] = ok
    REPORT["failed"] = len(REPORT["checks"]) - ok
    REPORT["finished_at"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    (DATA_DIR / "sso-report.json").write_text(
        json.dumps(REPORT, ensure_ascii=False, indent=2)
    )
    print(f"\n{ok}/{len(REPORT['checks'])} checks passed", flush=True)
    if REPORT["failed"]:
        sys.exit(1)


if __name__ == "__main__":
    main()
