#!/usr/bin/env python3
"""Phase 6 失敗モードの実測（detail/09 §13 / §5.2 / §5.3）。

前提: 001〜027 適用済み DB（既定 systivra_phase6_verify）、API（既定 18088）、
worker が起動済み。SAMPLE 静的サイトと遅延サーバーは本スクリプトが起動・停止する。

  PGHOST=localhost PGUSER=<admin> PGDATABASE=systivra_phase6_verify \
  SYSTIVRA_BASE_URL=http://127.0.0.1:18088 python3 scripts/phase6-failure-eval.py

実測項目:
  - storage_state 期限切れ / 欠落で実行前に ENV 中断（run を作らない）
  - 先頭 3 件全滅（NETWORK）で ENV へ切替えて run 停止、残りは SKIPPED
  - DATA 失敗で reset を強制し run 停止
  - run 開始時の reset / seed（RESET_ENDPOINT / SEED）
  - 成果物 0 件で UPLOAD_FAILED を記録し合否は保つ
  - TIMEOUT → attempt=2 PASSED の flaky 集計（再試行で通れば ENV 中断しない）
  - 同一 environment_id の 2 run目がロック待ち（queued_behind）から起動
"""

from __future__ import annotations

import functools
import http.cookiejar
import http.server
import json
import os
import pathlib
import shutil
import socketserver
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[1]
BASE_URL = os.environ.get("SYSTIVRA_BASE_URL", "http://127.0.0.1:18088")
WEB_PORT = int(os.environ.get("SAMPLE_PORT", "18099"))
DELAY_PORT = int(os.environ.get("DELAY_PORT", "18098"))
DATA_PORT = int(os.environ.get("DATA_PORT", "18097"))
WEB_BASE = f"http://127.0.0.1:{WEB_PORT}"
DELAY_BASE = f"http://127.0.0.1:{DELAY_PORT}"
DATA_BASE = f"http://127.0.0.1:{DATA_PORT}"
DB = os.environ.get("PGDATABASE", "systivra_phase6_verify")
LOGIN_ID = os.environ.get("SYSTIVRA_LOGIN_ID", "alice")
PASSWORD = os.environ.get("SYSTIVRA_PASSWORD", "SystivraDemo1!")
APPROVE_COUNT = 8
ARTIFACT_ROOT = ROOT / "data" / "phase6-runner"
REPORT_COPY = ROOT / "data" / "phase6-failure"

cookies = http.cookiejar.CookieJar()
opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(cookies))


def call(method: str, route: str, body: dict | None = None) -> dict:
    data = None
    headers = {}
    if body is not None:
        data = json.dumps(body, ensure_ascii=False).encode("utf-8")
        headers["content-type"] = "application/json"
    request = urllib.request.Request(
        f"{BASE_URL}/api/v1{route}", data=data, headers=headers, method=method
    )
    try:
        with opener.open(request, timeout=60) as response:
            raw = response.read()
            return json.loads(raw) if raw else {}
    except urllib.error.HTTPError as error:
        raise RuntimeError(
            f"{method} {route}: HTTP {error.code}: {error.read().decode('utf-8', 'replace')}"
        ) from error


def psql(sql: str) -> str:
    result = subprocess.run(
        ["psql", "-d", DB, "-v", "ON_ERROR_STOP=1", "-tA", "-c", sql],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(f"psql failed: {result.stderr.strip()}")
    return result.stdout.strip()


def dollar(value: dict) -> str:
    return "$j$" + json.dumps(value, ensure_ascii=False) + "$j$::jsonb"


def reset() -> None:
    psql(
        "TRUNCATE test.validation_rule, test.test_plan, test.test_scenario, "
        "test.test_case, test.test_step, test.test_data_set, test.test_data_row, "
        "test.test_run, test.test_execution, test.defect, test.coverage_snapshot "
        "RESTART IDENTITY CASCADE;"
    )
    psql("DELETE FROM core.job WHERE kind = 'TEST_RUN';")
    psql("DELETE FROM graph_meta.relationship WHERE revision = 'test';")
    psql("DELETE FROM graph_meta.entity WHERE revision = 'test';")
    psql("DELETE FROM impact.assessment_draft;")


RULE_SOURCES = [
    {"kind": "CODE", "field_ref": "user.addr", "text": "if (s.length() > 20) errors.add(...)",
     "evidence_ids": ["c1c1c1c1-c1c1-4c1c-8c1c-c1c1c1c1c1c1"]},
    {"kind": "SPEC", "field_ref": "user.name", "text": "必須項目", "evidence_ids": []},
    {"kind": "SPEC", "field_ref": "user.age", "text": "min=1 max=99", "evidence_ids": []},
    {"kind": "SPEC", "field_ref": "user.memo", "text": "全角20文字以内", "evidence_ids": []},
    {"kind": "SPEC", "field_ref": "user.code", "text": "pattern=\"^[ぁ-ん]+$\" 正規表現",
     "evidence_ids": []},
    {"kind": "SPEC", "field_ref": "user.period", "text": "開始日 < 終了日", "evidence_ids": []},
]


def prepare(pid: str) -> list[dict]:
    call("POST", f"/projects/{pid}/test/rules:extract", {"sources": RULE_SOURCES})
    impact_id = "d0000000-0000-4000-8000-000000000000"
    report = {
        "items": [
            {"item_id": "e0000000-0000-4000-8000-000000000001",
             "label": "UserService.save(UserDto)", "category": "CODE", "why": "変更 Method"},
            {"item_id": "e0000000-0000-4000-8000-000000000002",
             "label": "ユーザ編集画面", "category": "SCREENS", "why": "到達画面"},
        ],
        "degraded": ["tests_not_available"],
        "approval_state": "DRAFT",
    }
    psql(
        "INSERT INTO impact.assessment_draft (id, project_id, input, status, report) "
        f"VALUES ('{impact_id}', '{pid}', '{{}}', 'SUCCEEDED', {dollar(report)}) "
        "ON CONFLICT (id) DO UPDATE SET report = EXCLUDED.report;"
    )
    call(
        "POST",
        f"/projects/{pid}/test/plans:derive",
        {"impact_id": impact_id, "name": "失敗モード評価", "max_drafts": 100},
    )
    overview = call("GET", f"/projects/{pid}/test/overview")
    drafts = [case for case in overview["cases"] if case["status"] == "DRAFT"]
    assert len(drafts) >= APPROVE_COUNT, f"草稿が足りない: {len(drafts)}"
    for case in drafts[:APPROVE_COUNT]:
        if case["validation_errors"]:
            raise AssertionError(f"不正 DSL: {case['code']} {case['validation_errors']}")
        call("POST", f"/projects/{pid}/test/cases/{case['id']}/approve")
    approved = [
        case for case in call("GET", f"/projects/{pid}/test/overview")["cases"]
        if case["status"] == "APPROVED"
    ]
    assert len(approved) == APPROVE_COUNT
    return approved


def set_case_dsl(case_id: str, dsl: dict) -> None:
    psql(f"UPDATE test.test_case SET dsl = {dollar(dsl)} WHERE id = '{case_id}';")


def mutate_first(pid: str, approved: list[dict], marker: str) -> tuple[str, dict]:
    """先頭ケースの判定を marker 付き field_error に差し替え、元の DSL を返す。

    assert_text は buildUrl が label クエリで期待値を画面へ描いてしまい、
    SAMPLE では常に合格する。field_error は実エラー文言を見るので失敗する。
    """
    target = approved[0]
    original = json.loads(json.dumps(target["dsl"]))
    dsl = json.loads(json.dumps(target["dsl"]))
    replaced = False
    for index, step in enumerate(dsl["steps"]):
        if step["action"].startswith("assert_") or step["action"] == "field_error":
            dsl["steps"][index] = {
                "action": "field_error",
                "target": {"label": "user.addr"},
                "value": marker,
            }
            replaced = True
            break
    assert replaced, f"判定ステップが無い: {target['code']}"
    set_case_dsl(target["id"], dsl)
    return target["id"], original


def run_runner(env_extra: dict[str, str], timeout: int = 300) -> tuple[int, dict]:
    env = dict(os.environ)
    env.update(
        {
            "SYSTIVRA_BASE_URL": BASE_URL,
            "WEB_BASE": WEB_BASE,
            "PARALLEL": "4",
            "LIMIT": str(APPROVE_COUNT),
        }
    )
    env.update(env_extra)
    run = subprocess.run(
        ["node", str(ROOT / "workers" / "runner" / "runner.mjs")],
        cwd=str(ROOT),
        env=env,
        capture_output=True,
        text=True,
        timeout=timeout,
    )
    report_path = ARTIFACT_ROOT / "report.json"
    report = json.loads(report_path.read_text(encoding="utf-8")) if report_path.exists() else {}
    if run.returncode != 0 and not report:
        raise RuntimeError(f"runner failed:\n{run.stdout[-2000:]}\n{run.stderr[-2000:]}")
    return run.returncode, report


def wait_http(url: str) -> None:
    for _ in range(50):
        try:
            urllib.request.urlopen(url, timeout=1).close()
            return
        except Exception:  # noqa: BLE001 - 起動待ち
            time.sleep(0.1)
    raise RuntimeError(f"サーバーが起動しない: {url}")


class SampleHandler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *args):
        pass


class DelayHandler(http.server.SimpleHTTPRequestHandler):
    """パス単位の初回アクセスだけ遅延させ、TIMEOUT → 再試行合格を作る。"""

    seen: set[str] = set()
    lock = threading.Lock()
    delay_sec = 2.5

    def do_GET(self):
        with self.lock:
            first = self.path not in self.seen
            self.seen.add(self.path)
        if first:
            time.sleep(self.delay_sec)
        try:
            super().do_GET()
        except (BrokenPipeError, ConnectionResetError):
            pass

    def log_message(self, *args):
        pass


class DataHandler(http.server.BaseHTTPRequestHandler):
    counts = {"reset": 0, "seed": 0}
    lock = threading.Lock()

    def do_POST(self):
        key = "reset" if self.path == "/reset" else "seed" if self.path == "/seed" else None
        if key is None:
            self.send_error(404)
            return
        with self.lock:
            self.counts[key] += 1
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.end_headers()
        self.wfile.write(b'{"ok":true}')

    def log_message(self, *args):
        pass


def start_server(handler, port: int, directory: pathlib.Path | None = None):
    if directory is not None:
        handler = functools.partial(handler, directory=str(directory))
    socketserver.TCPServer.allow_reuse_address = True
    server = http.server.ThreadingHTTPServer(("127.0.0.1", port), handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    return server


def run_status(pid: str, run_id: str) -> str:
    overview = call("GET", f"/projects/{pid}/test/overview")
    return next(r["status"] for r in overview["runs"] if r["id"] == run_id)


def main() -> int:
    reset()
    REPORT_COPY.mkdir(parents=True, exist_ok=True)
    if ARTIFACT_ROOT.exists():
        for child in ARTIFACT_ROOT.iterdir():
            if child.is_dir():
                shutil.rmtree(child)
    ARTIFACT_ROOT.mkdir(parents=True, exist_ok=True)

    login = call("POST", "/auth/login", {"login_id": LOGIN_ID, "password": PASSWORD})
    pid = login["memberships"][0]["project_id"]
    approved = prepare(pid)
    results: dict[str, dict] = {}

    sample = start_server(SampleHandler, WEB_PORT, ROOT / "samples" / "sample-web")
    data_server = start_server(DataHandler, DATA_PORT)
    wait_http(f"{WEB_BASE}/")

    try:
        state_file = REPORT_COPY / "storage-state.json"
        state_file.write_text('{"cookies":[],"origins":[]}', encoding="utf-8")
        old = time.time() - 9 * 3600
        os.utime(state_file, (old, old))
        code, report = run_runner({
            "STORAGE_STATE": str(state_file),
            "STORAGE_STATE_MAX_AGE_MIN": "480",
        })
        assert code == 1 and report["env_abort"]["reason"] == "storage_state_expired", report
        assert report["run_id"] is None
        results["storage_state_expired"] = report
        print("ok storage_state_expired")

        code, report = run_runner({"STORAGE_STATE": str(REPORT_COPY / "missing.json")})
        assert code == 1 and report["env_abort"]["reason"] == "storage_state_missing", report
        results["storage_state_missing"] = report
        print("ok storage_state_missing")

        code, report = run_runner({"WEB_BASE": "http://127.0.0.1:1"})
        assert code == 0, report
        assert report["env_abort"]["reason"] == "first_three_env_suspect", report
        assert report["failure_kinds"].get("ENV", 0) == 3, report
        assert report["skipped_not_executed"] == APPROVE_COUNT - 4, report
        assert report["import"]["rejected"] == [], report
        assert run_status(pid, report["run_id"]) == "DONE"
        results["env_abort"] = report
        print("ok env_abort")

        case_id, original = mutate_first(pid, approved, "一意制約違反: 重複があります")
        try:
            code, report = run_runner({
                "DATA_MODE": "NONE",
                "RESET_URL": f"{DATA_BASE}/reset",
            })
            assert code == 0, report
            assert report["data_abort"]["reset"] == "ok", report
            assert report["data_resets"] == 1, report
            assert report["failure_kinds"].get("DATA", 0) >= 1, report
            assert report["skipped_not_executed"] >= 1, report
            assert DataHandler.counts["reset"] == 1, DataHandler.counts
            assert run_status(pid, report["run_id"]) == "DONE"
            results["data_reset"] = report
            print("ok data_reset")
        finally:
            set_case_dsl(case_id, original)

        code, report = run_runner({
            "DATA_MODE": "RESET_ENDPOINT",
            "RESET_URL": f"{DATA_BASE}/reset",
        })
        assert code == 0 and report["data_preparation"]["reset"] == "ok", report
        assert report["passed"] == APPROVE_COUNT, report
        assert DataHandler.counts["reset"] == 2, DataHandler.counts
        results["run_start_reset"] = report
        print("ok run_start_reset")

        code, report = run_runner({
            "DATA_MODE": "SEED",
            "SEED_URL": f"{DATA_BASE}/seed",
        })
        assert code == 0 and report["data_preparation"]["seed"] == "ok", report
        assert report["passed"] == APPROVE_COUNT, report
        assert DataHandler.counts["seed"] == 1, DataHandler.counts
        results["run_start_seed"] = report
        print("ok run_start_seed")

        created = call("POST", f"/projects/{pid}/test/runs", {
            "name": "upload-failed-eval",
            "plan_id": None,
            "environment": {
                "id": "env-upload-eval",
                "base_url": WEB_BASE,
                "guard_kind": "STAGING",
                "allowed_origins": ["127.0.0.1"],
                "parallel": 1,
            },
        })
        upload_run = created["run_id"]
        case_id, original = mutate_first(pid, approved, "期待はずれ-UPLOAD-FAILED")
        case_dir = ARTIFACT_ROOT / upload_run / approved[0]["code"]
        case_dir.mkdir(parents=True, exist_ok=True)
        case_dir.chmod(0o555)
        try:
            code, report = run_runner({
                "RUN_ID": upload_run,
                "LIMIT": "1",
                "PARALLEL": "1",
            })
            assert code == 0, report
            assert report["artifacts"]["upload_failed"] == 1, report
            assert len(report["warnings"]) == 1 and report["warnings"][0].startswith("UPLOAD_FAILED:"), report
            assert report["failed"] == 1, report
            overview = call("GET", f"/projects/{pid}/test/overview")
            execution = next(
                e for e in overview["executions"]
                if e["test_run_id"] == upload_run and e["execution_source"] == "PLAYWRIGHT"
            )
            assert execution["status"] == "FAILED"
            assert any(a["kind"] == "UPLOAD_FAILED" for a in execution["artifacts"]), execution
            results["upload_failed"] = report
            print("ok upload_failed")
        finally:
            case_dir.chmod(0o755)
            set_case_dsl(case_id, original)

        delay = start_server(DelayHandler, DELAY_PORT, ROOT / "samples" / "sample-web")
        try:
            code, report = run_runner({
                "WEB_BASE": DELAY_BASE,
                "STEP_TIMEOUT_MS": "1500",
                "STORAGE_STATE": str(state_file),
                "STORAGE_STATE_MAX_AGE_MIN": "600",
            })
            assert code == 0, report
            assert report["env_abort"] is None, report
            assert report["flaky_count"] >= 1, report
            assert report["failure_kinds"].get("TIMEOUT", 0) == report["flaky_count"], report
            assert report["passed"] == APPROVE_COUNT, report
            assert report["execution_count"] == APPROVE_COUNT + report["flaky_count"], report
            assert report["first_attempt_pass_rate"] == (APPROVE_COUNT - report["flaky_count"]) / APPROVE_COUNT, report
            assert report["import"]["rejected"] == [], report
            assert report["storage_state"]["age_seconds"] < 36000, report
            assert run_status(pid, report["run_id"]) == "DONE"
            results["flaky"] = report
            print(f"ok flaky (flaky={report['flaky_count']}/{APPROVE_COUNT})")
        finally:
            delay.shutdown()
            delay.server_close()

        DelayHandler.seen = set()
        delay = start_server(DelayHandler, DELAY_PORT, ROOT / "samples" / "sample-web")
        try:
            env = dict(os.environ)
            env.update({
                "SYSTIVRA_BASE_URL": BASE_URL,
                "WEB_BASE": DELAY_BASE,
                "PARALLEL": "4",
                "LIMIT": str(APPROVE_COUNT),
                "STEP_TIMEOUT_MS": "1500",
                "ENVIRONMENT_ID": "env-lock-eval",
            })
            proc_a = subprocess.Popen(
                ["node", str(ROOT / "workers" / "runner" / "runner.mjs")],
                cwd=str(ROOT), env=env,
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
            )
            time.sleep(1.5)
            code_b, report_b = run_runner({
                "WEB_BASE": DELAY_BASE,
                "STEP_TIMEOUT_MS": "1500",
                "ENVIRONMENT_ID": "env-lock-eval",
            })
            out_a, err_a = proc_a.communicate(timeout=120)
            assert proc_a.returncode == 0, f"runner A failed:\n{out_a[-2000:]}\n{err_a[-2000:]}"
            report_a = json.loads(out_a[out_a.index("{"):out_a.rindex("}") + 1])
            assert code_b == 0, report_b
            assert report_a["lock_wait_ms"] == 0, report_a
            assert report_b["lock_wait_ms"] > 0, report_b
            assert report_a["run_id"] != report_b["run_id"]
            assert run_status(pid, report_a["run_id"]) == "DONE"
            assert run_status(pid, report_b["run_id"]) == "DONE"
            results["environment_lock"] = {"first": report_a, "second": report_b}
            print(f"ok environment_lock (wait={report_b['lock_wait_ms']}ms)")
        finally:
            delay.shutdown()
            delay.server_close()
    finally:
        sample.shutdown()
        sample.server_close()
        data_server.shutdown()
        data_server.server_close()

    summary = {
        "approved_cases": APPROVE_COUNT,
        "data_endpoint_counts": dict(DataHandler.counts),
        "scenarios": results,
    }
    out = REPORT_COPY / "report.json"
    out.write_text(json.dumps(summary, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"\nwritten: {out}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as error:  # noqa: BLE001
        print(f"FAILED: {error}", file=sys.stderr)
        raise