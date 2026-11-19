#!/usr/bin/env python3
"""ジョブ / Agent run のキャンセルを E2E で実測する。

独立 DB・独立 API(18090)・独立 worker・mock LLM・vite を起動し、終了時に片付ける。

  PGHOST=localhost PGUSER=<admin> python3 scripts/job-cancel-eval.py
"""

from __future__ import annotations

import http.cookiejar
import json
import os
import pathlib
import subprocess
import sys
import time
import urllib.error
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[1]
DB = "systivra_job_cancel_verify"
API_PORT = 18090
LLM_PORT = 11536
WEB_PORT = 15176
API_BASE = f"http://127.0.0.1:{API_PORT}"
LLM_BASE = f"http://127.0.0.1:{LLM_PORT}"
WEB_BASE = f"http://localhost:{WEB_PORT}"
PASSWORD = "SystivraDemo1!"
FIXTURE = ROOT / "tests" / "fixtures" / "walking_skeleton_100p.pdf"
REPORT_DIR = ROOT / "data" / "job-cancel-eval"
TERMINAL = {"SUCCEEDED", "SUCCEEDED_WITH_ERRORS", "FAILED", "CANCELLED", "TIMEOUT"}
REPORT: dict = {}


class Client:
    def __init__(self) -> None:
        self.jar = http.cookiejar.CookieJar()
        self.opener = urllib.request.build_opener(
            urllib.request.HTTPCookieProcessor(self.jar)
        )

    def call(self, method: str, path: str, body: dict | None = None, headers: dict | None = None):
        data = None
        merged = dict(headers or {})
        if body is not None:
            data = json.dumps(body, ensure_ascii=False).encode("utf-8")
            merged["content-type"] = "application/json"
        request = urllib.request.Request(
            f"{API_BASE}/api/v1{path}", data=data, headers=merged, method=method
        )
        try:
            with self.opener.open(request, timeout=60) as response:
                raw = response.read()
                return response.status, json.loads(raw) if raw else None
        except urllib.error.HTTPError as error:
            raw = error.read()
            try:
                parsed = json.loads(raw) if raw else None
            except json.JSONDecodeError:
                parsed = {"raw": raw.decode("utf-8", "replace")}
            return error.code, parsed

    def must(self, method: str, path: str, body: dict | None = None, headers: dict | None = None) -> dict:
        status, payload = self.call(method, path, body, headers)
        if status >= 400:
            raise AssertionError(f"{method} {path} -> HTTP {status}: {payload}")
        return payload or {}

    def raw(self, path: str) -> tuple[int, str]:
        request = urllib.request.Request(f"{API_BASE}/api/v1{path}")
        with self.opener.open(request, timeout=60) as response:
            return response.status, response.read().decode("utf-8")


def psql(sql: str, database: str = "postgres") -> str:
    result = subprocess.run(
        ["psql", "-d", database, "-v", "ON_ERROR_STOP=1", "-tA", "-c", sql],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(f"psql failed: {result.stderr.strip()}")
    return result.stdout.strip()


def prepare_db() -> None:
    psql(f"DROP DATABASE IF EXISTS {DB};")
    psql(f"CREATE DATABASE {DB};")
    for migration in sorted((ROOT / "migrations").glob("0*.sql")):
        result = subprocess.run(
            ["psql", "-d", DB, "-v", "ON_ERROR_STOP=1", "-q", "-f", str(migration)],
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            raise RuntimeError(f"{migration.name}: {result.stderr.strip()}")
    subprocess.run(
        ["psql", "-d", DB, "-q", "-f", str(ROOT / "deployment" / "postgres" / "roles.sql")],
        check=True,
        capture_output=True,
    )
    env = dict(os.environ)
    env["PGDATABASE"] = DB
    env["PATH"] = f"{pathlib.Path.home()}/.cargo/bin:{env.get('PATH', '')}"
    subprocess.run(
        [str(ROOT / "scripts" / "seed-dev.sh")],
        check=True,
        capture_output=True,
        env=env,
    )
    psql(
        "INSERT INTO core.app_user (id, login_id, display_name, password_hash) "
        "SELECT '44444444-4444-4444-8444-444444444444', 'viewer', 'Viewer', password_hash "
        "FROM core.app_user WHERE login_id = 'alice';",
        DB,
    )
    psql(
        "INSERT INTO core.project_member (project_id, user_id, role) VALUES "
        "('22222222-2222-4222-8222-222222222222', "
        "'44444444-4444-4444-8444-444444444444', 'VIEWER');",
        DB,
    )


def wait_http(url: str, attempts: int = 100) -> None:
    for _ in range(attempts):
        try:
            urllib.request.urlopen(url, timeout=1).close()
            return
        except Exception:  # noqa: BLE001 - 起動待ち
            time.sleep(0.1)
    raise RuntimeError(f"{url} did not become ready")


def upload(client: Client, pid: str, data: bytes, filename: str) -> str:
    created = client.must(
        "POST",
        f"/projects/{pid}/documents:upload-url",
        {"filename": filename, "size_bytes": len(data), "mime_type": "application/pdf"},
    )
    url = created["upload_url"]
    if url.startswith("/"):
        url = API_BASE + url
    request = urllib.request.Request(
        url,
        data=data,
        method="PUT",
        headers={"Content-Type": "application/octet-stream"},
    )
    client.opener.open(request, timeout=120).read()
    return created["object_key"]


def register_parse(client: Client, pid: str, title: str) -> tuple[str, str, str]:
    data = FIXTURE.read_bytes() + f"\n%{title}\n".encode("utf-8")
    object_key = upload(client, pid, data, f"{title}.pdf")
    registered = client.must(
        "POST",
        f"/projects/{pid}/documents",
        {
            "object_key": object_key,
            "doc_type": "BASIC_DESIGN",
            "title": title,
            "mime_type": "application/pdf",
            "source_revision": "v1",
        },
    )
    did = registered["document"]["id"]
    rid = registered["document"]["current_revision"]["id"]
    accepted = client.must(
        "POST",
        f"/documents/{did}/revisions/{rid}:parse",
        headers={"idempotency-key": f"jc-{rid}"},
    )
    return did, rid, accepted["job_id"]


def wait_job(client: Client, job_id: str, want: str, timeout: float = 240) -> dict:
    deadline = time.time() + timeout
    while time.time() < deadline:
        job = client.must("GET", f"/jobs/{job_id}")
        if job["status"] == want:
            return job
        if job["status"] in TERMINAL and job["status"] != want:
            raise AssertionError(f"job {job_id} ended as {job['status']}, wanted {want}: {job.get('errors')}")
        time.sleep(0.3)
    raise TimeoutError(f"job {job_id} did not reach {want}")


def wait_running_parse(client: Client, job_id: str) -> dict:
    deadline = time.time() + 120
    while time.time() < deadline:
        job = client.must("GET", f"/jobs/{job_id}")
        if job["status"] == "RUNNING" and job["progress"]["files_done"] >= 1:
            return job
        if job["status"] in TERMINAL:
            raise AssertionError(f"parse job ended early: {job['status']}")
        time.sleep(0.3)
    raise TimeoutError(f"parse job {job_id} did not start processing pages")


def start_worker(ai_env: dict, page_delay_ms: int | None) -> subprocess.Popen:
    env = dict(os.environ, **ai_env)
    env.update(
        {
            "DATABASE_URL": f"postgres://systivra_scheduler@localhost:5432/{DB}",
            "WORKER_TICK_SEC": "1",
            "WORKER_STALE_SEC": "3",
            "SYSTIVRA_DATA_DIR": str(ROOT / "data"),
            "NEO4J_DISABLED": "1",
        }
    )
    if page_delay_ms is not None:
        env["WORKER_PAGE_DELAY_MS"] = str(page_delay_ms)
    else:
        env.pop("WORKER_PAGE_DELAY_MS", None)
    return subprocess.Popen(
        [str(ROOT / "target" / "debug" / "systivra-worker")],
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )


def stop(process: subprocess.Popen | None) -> None:
    if process is None:
        return
    process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()


def main() -> int:
    api = worker = llm = vite = None
    stamp = int(time.time())
    try:
        prepare_db()
        ai_env = {
            "AI_BASE_URL": f"{LLM_BASE}/v1",
            "AI_MODEL_REASONING": "mock-1",
            "AI_MODEL_FAST": "mock-1",
            "AI_PRICE_INPUT_PER_1M": "1",
            "AI_PRICE_OUTPUT_PER_1M": "1",
        }
        llm = subprocess.Popen(
            [sys.executable, str(ROOT / "scripts" / "mock-llm.py"), str(LLM_PORT)],
            env=dict(os.environ, MOCK_LLM_SLOW_SEC="6"),
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        time.sleep(0.6)

        api_env = dict(os.environ, **ai_env)
        api_env.update(
            {
                "PORT": str(API_PORT),
                "DATABASE_URL": f"postgres://systivra_app@localhost:5432/{DB}",
                "SESSION_SECRET": "job-cancel-eval-secret-0123456789abcdef0123456789abcdef",
                "SESSION_COOKIE_SECURE": "false",
                "PUBLIC_BASE_URL": API_BASE,
                "SYSTIVRA_DATA_DIR": str(ROOT / "data"),
                "NEO4J_DISABLED": "1",
            }
        )
        api = subprocess.Popen(
            [str(ROOT / "target" / "debug" / "systivra-api")],
            env=api_env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        wait_http(f"{API_BASE}/healthz")
        assert api.poll() is None, (
            f"API プロセスが起動に失敗しました（ポート {API_PORT} が使用中？）。"
            "古い systivra-api を終了してから再実行すること"
        )

        client = Client()
        login = client.must("POST", "/auth/login", {"login_id": "alice", "password": PASSWORD})
        pid = login["memberships"][0]["project_id"]
        viewer = Client()
        viewer.must("POST", "/auth/login", {"login_id": "viewer", "password": PASSWORD})

        # ── 1. QUEUED は即時 CANCELLED（worker 未起動） ─────
        title_a = f"JC-A-{stamp}"
        _, rid_a, job_a = register_parse(client, pid, title_a)
        queued = client.must("GET", f"/jobs/{job_a}")
        assert queued["status"] in ("PENDING", "QUEUED"), queued
        cancelled = client.must("POST", f"/jobs/{job_a}:cancel")
        assert cancelled["status"] == "CANCELLED", cancelled
        assert cancelled["errors"][0]["code"] == "JOB_CANCELLED", cancelled
        again = client.must("POST", f"/jobs/{job_a}:cancel")
        assert again["status"] == "CANCELLED", again
        status, problem = viewer.call("POST", f"/jobs/{job_a}:cancel")
        assert status == 404 and problem["code"] == "NOT_FOUND_JOB", (status, problem)
        row = psql(
            "SELECT status || '|' || attempt || '|' || COALESCE(locked_by, '-') "
            f"FROM core.job WHERE id = '{job_a}';",
            DB,
        )
        assert row == "CANCELLED|0|-", row
        parse_run = psql(
            f"SELECT status FROM corpus.parse_run WHERE id = (SELECT input->>'parse_run_id' FROM core.job WHERE id = '{job_a}')::uuid;",
            DB,
        )
        assert parse_run == "CANCELLED", parse_run
        audits = int(psql("SELECT count(*) FROM core.audit_log WHERE action = 'job.cancel';", DB))
        assert audits == 2, audits
        REPORT["queued_immediate"] = {
            "status": cancelled["status"],
            "error": cancelled["errors"][0]["code"],
            "idempotent_status": again["status"],
            "viewer_http": status,
            "audit_rows": audits,
            "never_claimed": row,
            "parse_run": parse_run,
        }

        # ── 2. RUNNING はページ境界で協調停止 ───────────────
        worker = start_worker(ai_env, 1200)
        time.sleep(3)
        row = psql(f"SELECT status || '|' || attempt FROM core.job WHERE id = '{job_a}';", DB)
        assert row == "CANCELLED|0", row

        title_b = f"JC-B-{stamp}"
        _, rid_b, job_b = register_parse(client, pid, title_b)
        running = wait_running_parse(client, job_b)
        flagged = client.must("POST", f"/jobs/{job_b}:cancel")
        assert flagged["status"] in ("RUNNING", "CANCELLED"), flagged
        stopped = wait_job(client, job_b, "CANCELLED", timeout=60)
        assert stopped["progress"]["phase"] == "CANCELLED", stopped["progress"]
        assert stopped["progress"]["files_done"] < 100, stopped["progress"]
        assert stopped["errors"][0]["code"] == "JOB_CANCELLED", stopped
        parse_status = psql(
            f"SELECT status FROM corpus.parse_run WHERE revision_id = '{rid_b}' "
            "ORDER BY started_at DESC LIMIT 1;",
            DB,
        )
        assert parse_status == "CANCELLED", parse_status
        sse_status, sse_body = client.raw(f"/jobs/{job_b}/events")
        assert sse_status == 200, sse_status
        assert "event: error" in sse_body and "JOB_CANCELLED" in sse_body, sse_body[-800:]
        REPORT["running_midstop"] = {
            "pages_done": stopped["progress"]["files_done"],
            "phase": stopped["progress"]["phase"],
            "parse_run": parse_status,
            "sse_terminal": True,
            "sse_has_cancel_error": True,
        }

        # ── 3. Agent run の取消（run / job / 承認を閉じる） ──
        client.must(
            "POST",
            f"/projects/{pid}/agents",
            {
                "code": "impact-analyst",
                "model_profile": "reasoning",
                "prompt_ref": "prompt://impact-analyst@1",
                "prompt_body": "narrative と hypotheses を JSON で返す。必要ならツールを使う。",
                "capabilities": ["project.list", "coverage.get"],
                "budgets": {
                    "max_tool_calls": 4,
                    "max_tokens": 100000,
                    "max_wall_time_sec": 120,
                    "max_cost_usd": 1.0,
                },
            },
        )
        q1 = client.must(
            "POST",
            f"/projects/{pid}/agents/impact-analyst/runs",
            {"question": f"MOCK_SLOW_HEARTBEAT JC-Q1-{stamp} 状態を要約して"},
        )
        run_id, run_job = q1["run_id"], q1["job_id"]
        deadline = time.time() + 60
        while time.time() < deadline:
            run = client.must("GET", f"/projects/{pid}/agent-runs/{run_id}")
            if run["status"] == "RUNNING":
                break
            time.sleep(0.3)
        assert run["status"] == "RUNNING", run
        approval_id = "55555555-5555-4555-8555-555555555555"
        psql(
            "INSERT INTO ai.approval_request "
            "(id, project_id, run_id, action, payload_digest, status, requested_by) VALUES "
            f"('{approval_id}', '{pid}', '{run_id}', 'tool.execute', "
            f"'\\x00'::bytea, 'PENDING', '33333333-3333-4333-8333-333333333333');",
            DB,
        )
        cancelled_run = client.must("POST", f"/projects/{pid}/agent-runs/{run_id}:cancel")
        assert cancelled_run["status"] == "CANCELLED", cancelled_run
        assert cancelled_run["error"]["code"] == "JOB_CANCELLED", cancelled_run
        run_job_final = wait_job(client, run_job, "CANCELLED", timeout=60)
        assert run_job_final["errors"][0]["code"] == "JOB_CANCELLED", run_job_final
        run_after = client.must("GET", f"/projects/{pid}/agent-runs/{run_id}")
        assert run_after["status"] == "CANCELLED", run_after
        assert run_after["output"] is None, run_after
        approval = psql(
            f"SELECT status || '|' || COALESCE(decided_by, '-') FROM ai.approval_request WHERE id = '{approval_id}';",
            DB,
        )
        assert approval == "REJECTED|SYSTEM", approval
        again_run = client.must("POST", f"/projects/{pid}/agent-runs/{run_id}:cancel")
        assert again_run["status"] == "CANCELLED", again_run
        status, problem = viewer.call("POST", f"/projects/{pid}/agent-runs/{run_id}:cancel")
        assert status == 403 and problem["code"] == "FORBIDDEN_PROJECT_SCOPE", (status, problem)
        agent_audits = int(
            psql("SELECT count(*) FROM core.audit_log WHERE action = 'ai.agent_run.cancel';", DB)
        )
        assert agent_audits >= 2, agent_audits
        REPORT["agent_cancel"] = {
            "run_status": run_after["status"],
            "run_error": run_after["error"]["code"],
            "job_status": run_job_final["status"],
            "approval": approval,
            "idempotent_status": again_run["status"],
            "viewer_http": status,
            "audit_rows": agent_audits,
        }

        # ── 4. 終態は 409（run / job とも） ─────────────────
        q2 = client.must(
            "POST",
            f"/projects/{pid}/agents/impact-analyst/runs",
            {"question": f"JC-Q2-{stamp} 状態を要約して"},
        )
        wait_job(client, q2["job_id"], "SUCCEEDED", timeout=120)
        done_run = client.must("GET", f"/projects/{pid}/agent-runs/{q2['run_id']}")
        assert done_run["status"] == "SUCCEEDED", done_run
        status, problem = client.call("POST", f"/projects/{pid}/agent-runs/{q2['run_id']}:cancel")
        assert status == 409 and problem["code"] == "CONFLICT_JOB_TERMINAL", (status, problem)
        status_job, problem_job = client.call("POST", f"/jobs/{q2['job_id']}:cancel")
        assert status_job == 409 and problem_job["code"] == "CONFLICT_JOB_TERMINAL", (status_job, problem_job)
        REPORT["terminal_409"] = {
            "run_http": status,
            "run_code": problem["code"],
            "job_http": status_job,
            "job_code": problem_job["code"],
        }

        # ── 5. stalled 再キューでも復活しない ───────────────
        title_c = f"JC-C-{stamp}"
        _, rid_c, job_c = register_parse(client, pid, title_c)
        wait_running_parse(client, job_c)
        worker.kill()
        worker.wait(timeout=5)
        worker = None
        client.must("POST", f"/jobs/{job_c}:cancel")
        mid = psql(
            f"SELECT status || '|' || (cancel_requested_at IS NOT NULL) FROM core.job WHERE id = '{job_c}';",
            DB,
        )
        assert mid == "RUNNING|true", mid
        time.sleep(5)
        worker = start_worker(ai_env, 1200)
        revived = wait_job(client, job_c, "CANCELLED", timeout=90)
        assert revived["errors"][0]["code"] == "JOB_CANCELLED", revived
        attempt = int(psql(f"SELECT attempt FROM core.job WHERE id = '{job_c}';", DB))
        assert attempt >= 2, attempt
        succeeded_events = int(
            psql(
                f"SELECT count(*) FROM core.job_event WHERE job_id = '{job_c}' "
                "AND data->>'status' = 'SUCCEEDED';",
                DB,
            )
        )
        assert succeeded_events == 0, succeeded_events
        parse_c = psql(
            f"SELECT status FROM corpus.parse_run WHERE revision_id = '{rid_c}' "
            "ORDER BY started_at DESC LIMIT 1;",
            DB,
        )
        assert parse_c == "CANCELLED", parse_c
        REPORT["stalled_no_revival"] = {
            "status": revived["status"],
            "attempt": attempt,
            "succeeded_events": succeeded_events,
            "parse_run": parse_c,
        }

        # ── 6. 解析済みジョブへの取消も 409 ─────────────────
        stop(worker)
        worker = start_worker(ai_env, None)
        title_d = f"JC-D-{stamp}"
        _, _, job_d = register_parse(client, pid, title_d)
        wait_job(client, job_d, "SUCCEEDED", timeout=240)
        status, problem = client.call("POST", f"/jobs/{job_d}:cancel")
        assert status == 409 and problem["code"] == "CONFLICT_JOB_TERMINAL", (status, problem)
        REPORT["parse_terminal_409"] = {"http": status, "code": problem["code"]}

        # ── 7. UI（1440 / 607） ─────────────────────────────
        stop(worker)
        worker = start_worker(ai_env, 1200)
        vite_env = dict(os.environ, VITE_API_PROXY=API_BASE)
        vite = subprocess.Popen(
            [
                str(ROOT / "apps" / "web" / "node_modules" / ".bin" / "vite"),
                "--port", str(WEB_PORT), "--strictPort",
            ],
            cwd=ROOT / "apps" / "web",
            env=vite_env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        wait_http(f"{WEB_BASE}/")
        ui_env = dict(
            os.environ,
            WEB_BASE=WEB_BASE,
            DOC_TITLE=title_c,
            QUESTION=f"MOCK_SLOW_HEARTBEAT JC-UI-{stamp} 状態を要約して",
        )
        ui_result = subprocess.run(
            ["npm", "run", "check:ui:cancel"],
            cwd=ROOT / "apps" / "web",
            env=ui_env,
            capture_output=True,
            text=True,
        )
        if ui_result.returncode != 0:
            raise RuntimeError(f"cancel UI failed:\n{ui_result.stdout}\n{ui_result.stderr}")
        ui_line = next(
            (line for line in reversed(ui_result.stdout.splitlines()) if line.startswith("{")),
            "{}",
        )
        REPORT["ui"] = json.loads(ui_line)

        REPORT["audit_totals"] = {
            action: int(psql(f"SELECT count(*) FROM core.audit_log WHERE action = '{action}';", DB))
            for action in ("job.cancel", "ai.agent_run.cancel", "document.parse.cancelled")
        }
        REPORT_DIR.mkdir(parents=True, exist_ok=True)
        out = REPORT_DIR / "report.json"
        out.write_text(json.dumps(REPORT, ensure_ascii=False, indent=2), encoding="utf-8")
        print(json.dumps(REPORT, ensure_ascii=False, indent=2))
        print(f"\nwritten: {out}")
        return 0
    finally:
        for process in (vite, worker, api, llm):
            stop(process)
        try:
            psql(f"DROP DATABASE IF EXISTS {DB};")
        except Exception:  # noqa: BLE001 - 後片付けは best effort
            pass


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as error:  # noqa: BLE001
        print(f"FAILED: {error}", file=sys.stderr)
        raise