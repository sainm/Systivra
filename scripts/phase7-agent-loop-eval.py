#!/usr/bin/env python3
"""Phase 7 追補: Agent のツール呼び出しループを mock LLM で実測する。

独立 DB・独立 API(18088)・独立 worker を起動し、終了時に片付ける。
既存の 18086 のサービスには触れない。

  PGHOST=localhost PGUSER=<admin> python3 scripts/phase7-agent-loop-eval.py
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
DB = "systivra_phase7_ai_verify"
API_PORT = 18088
LLM_PORT = 11534
WEB_PORT = 15174
API_BASE = f"http://127.0.0.1:{API_PORT}"
LLM_BASE = f"http://127.0.0.1:{LLM_PORT}"
WEB_BASE = f"http://localhost:{WEB_PORT}"
LLM_TRACE = ROOT / "data" / "phase7-eval" / "mock-llm-trace.jsonl"
LOGIN_ID = "alice"
PASSWORD = "SystivraDemo1!"
REPORT: dict = {}


class Client:
    def __init__(self) -> None:
        self.jar = http.cookiejar.CookieJar()
        self.opener = urllib.request.build_opener(
            urllib.request.HTTPCookieProcessor(self.jar)
        )

    def call(self, method: str, path: str, body: dict | None = None):
        data = None
        headers = {}
        if body is not None:
            data = json.dumps(body, ensure_ascii=False).encode("utf-8")
            headers["content-type"] = "application/json"
        request = urllib.request.Request(
            f"{API_BASE}/api/v1{path}", data=data, headers=headers, method=method
        )
        try:
            with self.opener.open(request, timeout=30) as response:
                raw = response.read()
                return response.status, json.loads(raw) if raw else None
        except urllib.error.HTTPError as error:
            raw = error.read()
            try:
                parsed = json.loads(raw) if raw else None
            except json.JSONDecodeError:
                parsed = {"raw": raw.decode("utf-8", "replace")}
            return error.code, parsed

    def must(self, method: str, path: str, body: dict | None = None) -> dict:
        status, payload = self.call(method, path, body)
        if status >= 400:
            raise AssertionError(f"{method} {path} -> HTTP {status}: {payload}")
        return payload or {}

    def raw(self, path: str) -> tuple[int, str]:
        request = urllib.request.Request(f"{API_BASE}/api/v1{path}")
        with self.opener.open(request, timeout=30) as response:
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
    LLM_TRACE.unlink(missing_ok=True)
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


def wait_http(url: str, attempts: int = 100) -> None:
    for _ in range(attempts):
        try:
            urllib.request.urlopen(url, timeout=1).close()
            return
        except Exception:  # noqa: BLE001 - 起動待ち
            time.sleep(0.1)
    raise RuntimeError(f"{url} did not become ready")


def wait_job(client: Client, job_id: str) -> dict:
    terminal = {"SUCCEEDED", "SUCCEEDED_WITH_ERRORS", "FAILED", "CANCELLED", "TIMEOUT"}
    for _ in range(240):
        job = client.must("GET", f"/jobs/{job_id}")
        if job["status"] in terminal:
            if job["status"] in {"FAILED", "CANCELLED", "TIMEOUT"}:
                raise AssertionError(f"job {job_id} ended as {job['status']}: {job.get('errors')}")
            return job
        time.sleep(0.25)
    raise TimeoutError(f"job {job_id} did not finish")


def main() -> int:
    api = worker = worker2 = llm = vite = None
    try:
        prepare_db()
        ai_env = {
            "AI_BASE_URL": f"{LLM_BASE}/v1",
            "AI_MODEL_REASONING": "mock-1",
            "AI_MODEL_FAST": "mock-1",
            # 100 万トークンあたり 1 USD。mock は 11 + 7 トークンなので
            # 1 コール 1.8e-5 USD。既存の予算（1.0 USD）には影響しない。
            "AI_PRICE_INPUT_PER_1M": "1",
            "AI_PRICE_OUTPUT_PER_1M": "1",
        }

        llm = subprocess.Popen(
            [sys.executable, str(ROOT / "scripts" / "mock-llm.py"), str(LLM_PORT)],
            env=dict(
                os.environ,
                MOCK_LLM_TRACE=str(LLM_TRACE),
                MOCK_LLM_SLOW_SEC="4",
            ),
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        time.sleep(0.6)  # mock は GET を持たないので起動待ちは固定

        api_env = dict(os.environ, **ai_env)
        api_env.update(
            {
                "PORT": str(API_PORT),
                "DATABASE_URL": f"postgres://systivra_app@localhost:5432/{DB}",
                "SESSION_SECRET": "phase7-agent-loop-secret-0123456789abcdef0123456789abcdef",
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
        # **ポート衝突を検出する。** 前回の API が残っていると、新プロセスは
        # bind に失敗して即死するのに wait_http は旧プロセスに当たって成功し、
        # 「古い DB のデータ」を読んで意味不明な失敗になる。
        assert api.poll() is None, (
            f"API プロセスが起動に失敗しました（ポート {API_PORT} が使用中？）。"
            "古い systivra-api を終了してから再実行すること"
        )

        worker_env = dict(os.environ, **ai_env)
        worker_env.update(
            {
                "DATABASE_URL": f"postgres://systivra_scheduler@localhost:5432/{DB}",
                "WORKER_TICK_SEC": "1",
                # stale=3 秒 → 心拍は 1 秒間隔。遅い job でも奪われないことまで見る。
                "WORKER_STALE_SEC": "3",
                "SYSTIVRA_DATA_DIR": str(ROOT / "data"),
                "NEO4J_DISABLED": "1",
            }
        )
        worker = subprocess.Popen(
            [str(ROOT / "target" / "debug" / "systivra-worker")],
            env=worker_env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )

        client = Client()
        login = client.must(
            "POST", "/auth/login", {"login_id": LOGIN_ID, "password": PASSWORD}
        )
        pid = login["memberships"][0]["project_id"]

        # ── 1. ツール呼び出しが通る ──────────────────────
        agent = client.must(
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
                    "max_wall_time_sec": 60,
                    "max_cost_usd": 1.0,
                },
            },
        )
        assert agent["current_version"] == 1, agent

        accepted = client.must(
            "POST",
            f"/projects/{pid}/agents/impact-analyst/runs",
            {"question": "プロジェクトの状態を確認して要約して"},
        )
        wait_job(client, accepted["job_id"])
        run = client.must("GET", f"/projects/{pid}/agent-runs/{accepted['run_id']}")
        assert run["status"] == "SUCCEEDED", run
        assert run["output"] and run["output"]["narrative"], run
        assert run["output"]["inferred"] is True
        assert run["output"]["hypotheses"][0]["resolution"] == "INFERRED"
        assert run["output"]["hypotheses"][0]["basis"], run["output"]
        capability_steps = [step for step in run["steps"] if step["kind"] == "CAPABILITY"]
        assert capability_steps, run["steps"]
        assert capability_steps[0]["name"] == "project_list"
        assert capability_steps[0]["status"] == "PASSED"
        assert run["cost"]["tool_calls"] >= 1
        assert run["cost"]["priced"] is True, run["cost"]
        assert run["cost"]["cost_usd"] > 0, run["cost"]
        assert "wall_time_sec" in run["cost"], run["cost"]
        REPORT["tool_loop"] = {
            "status": run["status"],
            "steps": [step["name"] + ":" + step["status"] for step in run["steps"]],
            "tool_calls": run["cost"]["tool_calls"],
            "cost_usd": round(run["cost"]["cost_usd"], 9),
            "priced": run["cost"]["priced"],
            "narrative": run["output"]["narrative"],
        }

        # ── 1.5 身元が要る能力も Agent から呼べる ──────────
        # knowledge.search は「誰が検索したか」を監査に残すため身元が要る。
        # worker は run.requested_by を主体として復元する（Agent ではない）。
        client.must(
            "POST",
            f"/projects/{pid}/agents",
            {
                "code": "knowledge-agent",
                "model_profile": "fast",
                "prompt_ref": "prompt://knowledge-agent@1",
                "prompt_body": "必要なら knowledge_search で根拠を集める。",
                "capabilities": ["knowledge.search", "project.list"],
            },
        )
        search_run = client.must(
            "POST",
            f"/projects/{pid}/agents/knowledge-agent/runs",
            {"question": "MOCK_KNOWLEDGE_SEARCH UserService の根拠を集めて"},
        )
        wait_job(client, search_run["job_id"])
        searched = client.must(
            "GET", f"/projects/{pid}/agent-runs/{search_run['run_id']}"
        )
        assert searched["status"] == "SUCCEEDED", searched
        search_steps = [step for step in searched["steps"] if step["kind"] == "CAPABILITY"]
        assert search_steps and search_steps[0]["name"] == "knowledge_search", searched["steps"]
        assert search_steps[0]["status"] == "PASSED", search_steps
        traced = LLM_TRACE.read_text(encoding="utf-8")
        assert "SRH-" in traced, traced[-500:]
        REPORT["tool_loop_identity"] = {
            "capability": search_steps[0]["name"],
            "status": search_steps[0]["status"],
            "search_id_returned": True,
        }

        # ── 2. ホワイトリスト外のツールは run を止める ─────
        client.must(
            "POST",
            f"/projects/{pid}/agents",
            {
                "code": "narrow-agent",
                "model_profile": "reasoning",
                "prompt_ref": "prompt://narrow@1",
                "prompt_body": "coverage だけを見る。",
                "capabilities": ["coverage.get"],
            },
        )
        denied_run = client.must(
            "POST",
            f"/projects/{pid}/agents/narrow-agent/runs",
            {"question": "状態を要約して"},
        )
        wait_job(client, denied_run["job_id"])
        denied = client.must(
            "GET", f"/projects/{pid}/agent-runs/{denied_run['run_id']}"
        )
        assert denied["status"] == "FAILED", denied
        assert denied["error"]["code"] == "AGENT_CAPABILITY_DENIED", denied["error"]
        assert denied["output"] is None
        REPORT["capability_denied"] = {
            "status": denied["status"],
            "error": denied["error"]["code"],
            "output": denied["output"],
        }

        # ── 2.2 コスト上限も実際に効く ────────────────────
        # 予算 4 項目のうち wall time / cost は、測っていなければ 0 のままで
        # 上限が効かない。単価を設定したうえで、極小の予算を当てて確かめる。
        client.must(
            "POST",
            f"/projects/{pid}/agents",
            {
                "code": "budget-agent",
                "model_profile": "fast",
                "prompt_ref": "prompt://budget@1",
                "prompt_body": "短く答える。",
                "capabilities": ["project.list"],
                "budgets": {
                    "max_tool_calls": 4,
                    "max_tokens": 100000,
                    "max_wall_time_sec": 60,
                    "max_cost_usd": 0.000001,
                },
            },
        )
        budget_run = client.must(
            "POST",
            f"/projects/{pid}/agents/budget-agent/runs",
            {"question": "状態を要約して"},
        )
        wait_job(client, budget_run["job_id"])
        over = client.must("GET", f"/projects/{pid}/agent-runs/{budget_run['run_id']}")
        assert over["status"] == "BUDGET_EXCEEDED", over
        assert over["error"]["budget"] == "max_cost_usd", over["error"]
        assert over["cost"]["priced"] is True, over["cost"]
        assert over["cost"]["cost_usd"] > 0, over["cost"]
        REPORT["budget_cost"] = {
            "status": over["status"],
            "budget": over["error"]["budget"],
            "cost_usd": round(over["cost"]["cost_usd"], 9),
        }

        # ── 2.3 処理中の心拍（長い job を奪わせない） ────────
        # これまでは「stale 窓を超える job が検証環境に無い」として未実測だった。
        # mock LLM に 4 秒の遅延を入れ、2 台目の worker を立てて確かめる。
        worker2 = subprocess.Popen(
            [str(ROOT / "target" / "debug" / "systivra-worker")],
            env=worker_env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        client.must(
            "POST",
            f"/projects/{pid}/agents",
            {
                "code": "heartbeat-agent",
                "model_profile": "fast",
                "prompt_ref": "prompt://heartbeat@1",
                "prompt_body": "MOCK_SLOW_HEARTBEAT を含む依頼。短く答える。",
                "capabilities": ["project.list"],
                "budgets": {
                    "max_tool_calls": 4,
                    "max_tokens": 100000,
                    "max_wall_time_sec": 120,
                    "max_cost_usd": 1.0,
                },
            },
        )
        slow_run = client.must(
            "POST",
            f"/projects/{pid}/agents/heartbeat-agent/runs",
            {"question": "状態を要約して"},
        )
        job_id = slow_run["job_id"]
        beats: list[str] = []
        deadline = time.time() + 30
        while time.time() < deadline:
            row = psql(
                "SELECT status || '|' || COALESCE(heartbeat_at::text, '-') "
                f"|| '|' || attempt FROM core.job WHERE id = '{job_id}';",
                database=DB,
            )
            if not row:
                break
            status, beat, attempt = row.split("|")
            if beat != "-" and (not beats or beats[-1] != beat):
                beats.append(beat)
            if status not in ("RUNNING", "QUEUED", "PENDING"):
                break
            time.sleep(0.4)
        # 心拍が 2 回以上更新されていれば、処理中も打ち続けている。
        assert len(beats) >= 2, beats
        final = psql(
            "SELECT status || '|' || attempt FROM core.job "
            f"WHERE id = '{job_id}';",
            database=DB,
        )
        final_status, final_attempt = final.split("|")
        assert final_status in ("SUCCEEDED", "SUCCEEDED_WITH_ERRORS"), final
        # 2 台目が「落ちた」と誤判定して再キューしていれば attempt が 2 になる。
        assert final_attempt == "1", final
        REPORT["heartbeat_during_processing"] = {
            "beats": len(beats),
            "status": final_status,
            "attempt": int(final_attempt),
            "workers": 2,
        }
        worker2.terminate()
        worker2.wait(timeout=5)
        worker2 = None

        # ── 2.5 Q-01 の叙述（AI 可用） ────────────────────
        inquiry_id = "f0f0f0f0-f0f0-4f0f-8f0f-f0f0f0f0f0f0"
        evidence_entity = "d0d0d0d0-d0d0-4d0d-8d0d-d0d0d0d0d0d0"
        report = {
            "summary": {
                "headline": "テスト変更です。画面 0 件、設計書 0 箇所、コード 1 箇所、テスト 0 件に波及します。",
                "approval_state": "DRAFT",
                "code_modified": False,
                "tests_executed": False,
                "risk": None,
                "counts": {"screens": 0, "documents": 0, "code": 1, "tests": 0, "other": 0},
                "truncated": None,
                "degraded": [],
                "narrative": None,
                "narrative_source": None,
                "hypotheses": [],
            },
            "screens": {"total": 0, "items": [], "degraded": []},
            "documents": {"total": 0, "items": [], "degraded": []},
            "code": {
                "total": 1,
                "items": [
                    {
                        "entity_id": evidence_entity,
                        "natural_key": "jp.example.web.UserEditAction",
                        "label": "UserEditAction",
                        "kind": "Method",
                        "resolution": "RESOLVED",
                        "confidence": 0.95,
                        "why": "変更のシードとして指定された",
                        "path": [evidence_entity],
                        "quote": "class UserEditAction {}",
                        "locator": {
                            "path": "src/UserEditAction.java",
                            "byte_start": 0,
                            "byte_end": 24,
                        },
                    }
                ],
                "degraded": [],
            },
            "tests": {
                "existing": [], "drafts": [], "steps": [],
                "data_boundaries": [], "not_covered": [], "degraded": [],
            },
        }
        payload = json.dumps(report, ensure_ascii=False)
        psql(
            "INSERT INTO impact.change_inquiry "
            "(id, project_id, question, input_kind, status, report) VALUES "
            f"('{inquiry_id}', '{pid}', 'q', 'ENTITY', 'SUCCEEDED', $j${payload}$j$::jsonb) "
            "ON CONFLICT (id) DO UPDATE SET report = EXCLUDED.report;",
            DB,
        )
        authoritative_before = {
            "summary": {
                "counts": report["summary"]["counts"],
                "risk": report["summary"]["risk"],
                "approval_state": report["summary"]["approval_state"],
                "code_modified": report["summary"]["code_modified"],
                "tests_executed": report["summary"]["tests_executed"],
            },
            "screens": report["screens"],
            "documents": report["documents"],
            "code": report["code"],
            "tests": report["tests"],
        }
        pre_narrate = client.must(
            "GET", f"/projects/{pid}/change-inquiries/{inquiry_id}"
        )
        assert pre_narrate.get("code"), pre_narrate
        assert pre_narrate["code"]["items"], pre_narrate["code"]
        assert pre_narrate["code"]["items"][0]["entity_id"] == evidence_entity
        narrate = client.must(
            "POST", f"/projects/{pid}/change-inquiries/{inquiry_id}/narrate"
        )
        assert narrate["narrative"], narrate
        assert len(narrate["hypotheses"]) == 1, narrate
        hypothesis = narrate["hypotheses"][0]
        assert hypothesis["resolution"] == "INFERRED", hypothesis
        assert hypothesis["basis"], hypothesis
        assert hypothesis["basis"][0]["entity_id"] == evidence_entity, hypothesis
        assert hypothesis["basis"][0]["quote"] == "class UserEditAction {}", hypothesis
        assert narrate["model"] == "mock-1", narrate
        assert "ai_inferred" in narrate["degraded"], narrate
        reloaded = client.must(
            "GET", f"/projects/{pid}/change-inquiries/{inquiry_id}"
        )
        assert reloaded["summary"]["narrative"], reloaded["summary"]
        assert reloaded["summary"]["hypotheses"][0]["resolution"] == "INFERRED"
        authoritative_after = {
            "summary": {
                "counts": reloaded["summary"]["counts"],
                "risk": reloaded["summary"]["risk"],
                "approval_state": reloaded["summary"]["approval_state"],
                "code_modified": reloaded["summary"]["code_modified"],
                "tests_executed": reloaded["summary"]["tests_executed"],
            },
            "screens": reloaded["screens"],
            "documents": reloaded["documents"],
            "code": reloaded["code"],
            "tests": reloaded["tests"],
        }
        assert authoritative_after == authoritative_before, (
            authoritative_before,
            authoritative_after,
        )
        assert "ai_unavailable" not in reloaded["summary"]["degraded"], reloaded["summary"]
        markdown = client.raw(
            f"/projects/{pid}/change-inquiries/{inquiry_id}/report.md"
        )
        assert markdown[0] == 200, markdown
        assert "AI の推定（INFERRED）" in markdown[1], markdown[1]
        assert "AI の要約（INFERRED）" in markdown[1], markdown[1]
        assert narrate["narrative"] in markdown[1], markdown[1]
        assert hypothesis["claim"] in markdown[1], markdown[1]
        assert hypothesis["basis"][0]["quote"] in markdown[1], markdown[1]
        REPORT["inquiry_narrative"] = {
            "narrative": narrate["narrative"],
            "hypotheses": narrate["hypotheses"],
            "model": narrate["model"],
            "degraded": narrate["degraded"],
            "authoritative_unchanged": True,
        }

        # ── 2.6 Q-01 の不正 AI 出力は主表を変えず降格 ─────
        invalid_id = "f1f1f1f1-f1f1-4f1f-8f1f-f1f1f1f1f1f1"
        invalid_report = json.loads(json.dumps(report))
        invalid_report["summary"]["headline"] = "MOCK_INVALID_INQUIRY"
        invalid_payload = json.dumps(invalid_report, ensure_ascii=False)
        psql(
            "INSERT INTO impact.change_inquiry "
            "(id, project_id, question, input_kind, status, report) VALUES "
            f"('{invalid_id}', '{pid}', 'invalid', 'ENTITY', 'SUCCEEDED', $j${invalid_payload}$j$::jsonb) "
            "ON CONFLICT (id) DO UPDATE SET report = EXCLUDED.report;",
            DB,
        )
        invalid_before = client.must(
            "GET", f"/projects/{pid}/change-inquiries/{invalid_id}"
        )
        invalid_narrate = client.must(
            "POST", f"/projects/{pid}/change-inquiries/{invalid_id}/narrate"
        )
        assert invalid_narrate["narrative"] is None, invalid_narrate
        assert invalid_narrate["hypotheses"] == [], invalid_narrate
        assert any(
            reason.startswith("ai_schema_violation")
            for reason in invalid_narrate["degraded"]
        ), invalid_narrate
        invalid_after = client.must(
            "GET", f"/projects/{pid}/change-inquiries/{invalid_id}"
        )
        assert invalid_after["summary"]["counts"] == invalid_before["summary"]["counts"]
        assert invalid_after["summary"]["risk"] == invalid_before["summary"]["risk"]
        for section in ("screens", "documents", "code", "tests"):
            assert invalid_after[section] == invalid_before[section], section
        assert any(
            reason.startswith("ai_schema_violation")
            for reason in invalid_after["summary"]["degraded"]
        )
        REPORT["invalid_ai_output"] = {
            "narrative": invalid_narrate["narrative"],
            "hypotheses": invalid_narrate["hypotheses"],
            "degraded": invalid_narrate["degraded"],
            "authoritative_unchanged": True,
        }

        # ── 2.7 Impact の叙述（AI 可用） ──────────────────
        impact_id = "a7a7a7a7-a7a7-4a7a-8a7a-a7a7a7a7a7a7"
        impact_report = {
            "seeds": [],
            "items": [],
            "unverified": [],
            "stats": {
                "total": 0, "code": 0, "screens": 0, "documents": 0, "data": 0,
                "resolved": 0, "partial": 0, "inferred": 0, "unresolved": 0, "conflict": 0,
            },
            "risk": {"score": 0.0, "level": "LOW", "factors": []},
            "truncated": {"hubs": 0, "depth": False, "budget": False, "no_logical_change": True},
            "degraded": [],
            "base_revision_id": None,
            "head_revision_id": None,
            "engine_version": "test",
            "ontology_version": "v1",
            "approval_state": "DRAFT",
            "code_modified": False,
            "tests_executed": False,
        }
        impact_payload = json.dumps(impact_report, ensure_ascii=False)
        psql(
            "INSERT INTO impact.assessment_draft (id, project_id, input, status, report) VALUES "
            f"('{impact_id}', '{pid}', '{{}}', 'SUCCEEDED', $j${impact_payload}$j$::jsonb) "
            "ON CONFLICT (id) DO UPDATE SET report = EXCLUDED.report;",
            DB,
        )
        impact_narrate = client.must("POST", f"/impacts/{impact_id}/narrate")
        assert impact_narrate["narrative"], impact_narrate
        assert "ai_inferred" in impact_narrate["degraded"], impact_narrate
        impact_after = client.must("GET", f"/impacts/{impact_id}")
        assert impact_after["report"]["narrative"], impact_after["report"]
        assert impact_after["report"]["narrative_source"] == "AI"
        impact_md = client.raw(f"/impacts/{impact_id}/report.md")
        assert impact_md[0] == 200, impact_md
        assert "AI の要約（INFERRED）" in impact_md[1], impact_md[1]
        assert impact_narrate["narrative"] in impact_md[1], impact_md[1]
        REPORT["impact_narrative"] = {
            "narrative": impact_narrate["narrative"],
            "model": impact_narrate["model"],
            "saved": True,
            "markdown": True,
        }

        # ── 3. 画面でも推定層を折り畳み、主表を変えない ──
        vite_env = dict(os.environ, VITE_API_PROXY=API_BASE)
        vite = subprocess.Popen(
            [
                str(ROOT / "apps" / "web" / "node_modules" / ".bin" / "vite"),
                "--port",
                str(WEB_PORT),
                "--strictPort",
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
            PROJECT_ID=pid,
            INQUIRY_ID=inquiry_id,
        )
        ui_result = subprocess.run(
            ["npm", "run", "check:ui:phase7-inquiry"],
            cwd=ROOT / "apps" / "web",
            env=ui_env,
            capture_output=True,
            text=True,
        )
        if ui_result.returncode != 0:
            raise RuntimeError(
                f"phase7 inquiry UI failed:\n{ui_result.stdout}\n{ui_result.stderr}"
            )
        ui_line = next(
            (
                line
                for line in reversed(ui_result.stdout.splitlines())
                if line.startswith("{")
            ),
            "{}",
        )
        REPORT["ui"] = json.loads(ui_line)

        # ── 4. AI 未設定でも Q-01 の 5 段を維持する ───────
        api.terminate()
        api.wait(timeout=5)
        no_ai_env = dict(os.environ)
        for key in (
            "AI_BASE_URL",
            "AI_MODEL_FAST",
            "AI_MODEL_REASONING",
            "AI_MODEL_LONG_CONTEXT",
        ):
            no_ai_env.pop(key, None)
        no_ai_env.update(
            {
                "PORT": str(API_PORT),
                "DATABASE_URL": f"postgres://systivra_app@localhost:5432/{DB}",
                "SESSION_SECRET": "phase7-agent-loop-secret-0123456789abcdef0123456789abcdef",
                "SESSION_COOKIE_SECURE": "false",
                "PUBLIC_BASE_URL": API_BASE,
                "SYSTIVRA_DATA_DIR": str(ROOT / "data"),
                "NEO4J_DISABLED": "1",
            }
        )
        api = subprocess.Popen(
            [str(ROOT / "target" / "debug" / "systivra-api")],
            env=no_ai_env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        wait_http(f"{API_BASE}/healthz")
        unavailable = client.must(
            "POST", f"/projects/{pid}/change-inquiries/{invalid_id}/narrate"
        )
        assert unavailable["narrative"] is None, unavailable
        assert unavailable["hypotheses"] == [], unavailable
        assert any(
            reason.startswith("ai_unavailable") for reason in unavailable["degraded"]
        ), unavailable
        unavailable_view = client.must(
            "GET", f"/projects/{pid}/change-inquiries/{invalid_id}"
        )
        assert unavailable_view["summary"]["counts"] == invalid_before["summary"]["counts"]
        assert unavailable_view["summary"]["risk"] == invalid_before["summary"]["risk"]
        for section in ("screens", "documents", "code", "tests"):
            assert unavailable_view[section] == invalid_before[section], section
        REPORT["ai_unavailable"] = {
            "narrative": unavailable["narrative"],
            "hypotheses": unavailable["hypotheses"],
            "degraded": unavailable["degraded"],
            "authoritative_unchanged": True,
        }

        # ── 5. 監査に残る ────────────────────────────────
        audits = int(
            psql(
                "SELECT count(*) FROM core.audit_log WHERE action LIKE 'ai.%';",
                DB,
            )
            or 0
        )
        assert audits >= 4, audits
        REPORT["audit_rows"] = audits

        out = ROOT / "data" / "phase7-eval" / "agent-loop.json"
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(REPORT, ensure_ascii=False, indent=2), encoding="utf-8")
        print(json.dumps(REPORT, ensure_ascii=False, indent=2))
        print(f"\nwritten: {out}")
        return 0
    finally:
        for process in (vite, worker2, worker, api, llm):
            if process is not None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
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
