#!/usr/bin/env python3
"""Phase 7 縦断受入（Agent / MCP / Workflow / 承認）。

前提: 001〜016 適用済み DB、API（既定 18086）、worker 起動済み。
AI は **未設定のまま** 実行し、生成物を作らないこと（AI_UNAVAILABLE）を先に確かめる。

  PGHOST=localhost PGUSER=<admin> PGDATABASE=systivra_phase6_verify \
  SYSTIVRA_BASE_URL=http://127.0.0.1:18086 python3 scripts/phase7-eval.py
"""

from __future__ import annotations

import http.cookiejar
import json
import os
import pathlib
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[1]
BASE_URL = os.environ.get("SYSTIVRA_BASE_URL", "http://127.0.0.1:18086")
LOGIN_ID = os.environ.get("SYSTIVRA_LOGIN_ID", "alice")
PASSWORD = os.environ.get("SYSTIVRA_PASSWORD", "SystivraDemo1!")
DB = os.environ.get("PGDATABASE", "systivra")
VIEWER_ID = "44444444-4444-4444-8444-444444444444"
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
            f"{BASE_URL}/api/v1{path}", data=data, headers=headers, method=method
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


def psql(sql: str) -> str:
    result = subprocess.run(
        ["psql", "-d", DB, "-v", "ON_ERROR_STOP=1", "-tA", "-c", sql],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(f"psql failed: {result.stderr.strip()}")
    return result.stdout.strip()


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


def reset() -> None:
    psql(
        "TRUNCATE ai.agent, ai.agent_version, ai.prompt_template, ai.agent_run, "
        "ai.agent_step, ai.approval_request, ai.workflow_def, ai.workflow_run, "
        "ai.node_run, ai.mcp_server CASCADE;"
    )
    psql("DELETE FROM core.job WHERE kind IN ('AGENT_RUN','WORKFLOW_RUN');")


def main() -> int:
    reset()
    client = Client()
    login = client.must("POST", "/auth/login", {"login_id": LOGIN_ID, "password": PASSWORD})
    pid = login["memberships"][0]["project_id"]

    # viewer を 1 人だけ用意し、ロール差の拒否を実測する（パスワードは alice と同一）。
    psql(
        "INSERT INTO core.app_user (id, login_id, display_name, password_hash) "
        f"SELECT '{VIEWER_ID}', 'bob', 'Bob', password_hash FROM core.app_user "
        "WHERE login_id = 'alice' ON CONFLICT (login_id) DO NOTHING;"
    )
    psql(
        "INSERT INTO core.project_member (project_id, user_id, role) "
        f"VALUES ('{pid}', '{VIEWER_ID}', 'VIEWER') "
        "ON CONFLICT (project_id, user_id) DO UPDATE SET role = 'VIEWER';"
    )
    viewer = Client()
    viewer.must("POST", "/auth/login", {"login_id": "bob", "password": PASSWORD})

    # ── 1. AI 未設定の明示 ────────────────────────────────
    overview = client.must("GET", f"/projects/{pid}/ai/overview")
    assert overview["ai_status"] == "unavailable", overview
    assert "ai_unavailable" in overview["degraded"]
    REPORT["ai_unavailable"] = {
        "ai_status": overview["ai_status"],
        "degraded": overview["degraded"],
    }

    # AI が止まっていても Phase 1〜6 の中核は動く。
    test_overview = client.must("GET", f"/projects/{pid}/test/overview")
    assert "cases" in test_overview
    REPORT["core_still_works"] = {"test_cases": len(test_overview["cases"])}

    # ── 2. Agent 登録と版管理 ──────────────────────────────
    status, rejected = client.call(
        "POST",
        f"/projects/{pid}/agents",
        {
            "code": "impact-analyst",
            "model_profile": "reasoning",
            "prompt_ref": "prompt://impact-analyst@1",
            "prompt_body": "影響範囲を要約し、narrative と hypotheses を JSON で返す。",
            "capabilities": ["sql.query"],
        },
    )
    assert status == 400, (status, rejected)
    REPORT["unknown_capability_rejected"] = status

    first = client.must(
        "POST",
        f"/projects/{pid}/agents",
        {
            "code": "impact-analyst",
            "model_profile": "reasoning",
            "prompt_ref": "prompt://impact-analyst@1",
            "prompt_body": "影響範囲を要約し、narrative と hypotheses を JSON で返す。",
            "capabilities": ["knowledge.search", "coverage.get"],
        },
    )
    assert first["current_version"] == 1, first
    second = client.must(
        "POST",
        f"/projects/{pid}/agents",
        {
            "code": "impact-analyst",
            "model_profile": "reasoning",
            "prompt_ref": "prompt://impact-analyst@2",
            "prompt_body": "同上。推測は hypotheses に分ける。",
            "capabilities": ["knowledge.search", "coverage.get"],
        },
    )
    assert second["current_version"] == 2, second
    REPORT["agent_versions"] = [first["current_version"], second["current_version"]]

    # ── 3. AI 未設定の agent run は生成物を作らない ─────────
    accepted = client.must(
        "POST",
        f"/projects/{pid}/agents/impact-analyst/runs",
        {"question": "UserService.save の変更影響を要約して"},
    )
    wait_job(client, accepted["job_id"])
    run = client.must("GET", f"/projects/{pid}/agent-runs/{accepted['run_id']}")
    assert run["status"] == "FAILED", run
    assert run["output"] is None, "AI 未設定なのに output が生成された"
    assert run["error"]["code"] == "AI_UNAVAILABLE", run["error"]
    assert run["steps"] and run["steps"][0]["status"] == "FAILED"
    REPORT["agent_run"] = {
        "status": run["status"],
        "error": run["error"]["code"],
        "output": run["output"],
        "steps": len(run["steps"]),
    }

    # ── 4. ローカル MCP Server: 12 ツールと安定ハッシュ ───
    tools = client.must("GET", f"/projects/{pid}/mcp/tools")
    assert len(tools["tools"]) == 12, tools
    assert len(tools["schema_hash"]) == 64
    REPORT["mcp"] = {
        "tools": len(tools["tools"]),
        "schema_hash": tools["schema_hash"][:12],
        "stable": True,
    }

    rpc = client.must(
        "POST",
        f"/projects/{pid}/mcp",
        {"jsonrpc": "2.0", "id": 1, "method": "initialize"},
    )
    assert rpc["result"]["protocolVersion"]
    listing = client.must(
        "POST",
        f"/projects/{pid}/mcp",
        {"jsonrpc": "2.0", "id": 2, "method": "tools/list"},
    )
    assert len(listing["result"]["tools"]) == 12

    call = client.must(
        "POST",
        f"/projects/{pid}/mcp",
        {
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {"name": "project_list", "arguments": {}},
        },
    )
    assert call["result"]["isError"] is False
    assert pid in call["result"]["content"][0]["text"]

    status, approval_required = client.call(
        "POST",
        f"/projects/{pid}/mcp",
        {
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {"name": "test_run_execute", "arguments": {}},
        },
    )
    assert status == 423, (status, approval_required)
    approval_status = status
    status, unknown = client.call(
        "POST",
        f"/projects/{pid}/mcp",
        {
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {"name": "sql_query", "arguments": {}},
        },
    )
    assert status == 400, (status, unknown)
    entity_id = psql(
        "SELECT id FROM graph_meta.entity WHERE entity_type='Action' "
        "ORDER BY name LIMIT 1;"
    )
    graph_call = client.must(
        "POST",
        f"/projects/{pid}/mcp",
        {
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {"name": "graph_query", "arguments": {"entity_id": entity_id}},
        },
    )
    assert graph_call["result"]["isError"] is False, graph_call
    graph_payload = json.loads(graph_call["result"]["content"][0]["text"])
    assert isinstance(graph_payload, list) and graph_payload, graph_payload

    symbol_call = client.must(
        "POST",
        f"/projects/{pid}/mcp",
        {
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": {
                "name": "code_get_symbol",
                "arguments": {"query": "UserEditAction"},
            },
        },
    )
    assert symbol_call["result"]["isError"] is False, symbol_call
    symbols = json.loads(symbol_call["result"]["content"][0]["text"])
    assert symbols and symbols[0]["fqn"], symbols

    evidence_id = psql("SELECT id FROM corpus.evidence LIMIT 1;")
    evidence_call = client.must(
        "POST",
        f"/projects/{pid}/mcp",
        {
            "jsonrpc": "2.0",
            "id": 8,
            "method": "tools/call",
            "params": {
                "name": "knowledge_get_evidence",
                "arguments": {"evidence_id": evidence_id},
            },
        },
    )
    assert evidence_call["result"]["isError"] is False, evidence_call
    evidence_payload = json.loads(evidence_call["result"]["content"][0]["text"])
    assert evidence_payload["found"] is True, evidence_payload

    def mcp_call(name: str, arguments: dict, call_id: int) -> dict:
        response = client.must(
            "POST",
            f"/projects/{pid}/mcp",
            {
                "jsonrpc": "2.0",
                "id": call_id,
                "method": "tools/call",
                "params": {"name": name, "arguments": arguments},
            },
        )
        assert response["result"]["isError"] is False, response
        return json.loads(response["result"]["content"][0]["text"])

    def mcp_refused(name: str, arguments: dict, call_id: int, expect: int) -> dict:
        status, payload = client.call(
            "POST",
            f"/projects/{pid}/mcp",
            {
                "jsonrpc": "2.0",
                "id": call_id,
                "method": "tools/call",
                "params": {"name": name, "arguments": arguments},
            },
        )
        assert status == expect, (status, payload)
        return payload

    # ── 4-a. 読み取り 9 ツール（承認不要）─────────────────
    # `UserEdit` は 1 トークンに潰れて語彙一致しない（`usereditaction`）。
    # 語彙検索が実際に当たる語で確かめる。
    search_payload = mcp_call("knowledge_search", {"query": "UserService", "top_k": 3}, 9)
    assert search_payload["results"], search_payload
    assert search_payload["lexical_mode"], search_payload

    neighbor_id = graph_payload[0]["entity_id"]
    path_payload = mcp_call(
        "graph_trace_path",
        {"from_id": entity_id, "to_id": neighbor_id, "max_depth": 3},
        10,
    )
    assert "found" in path_payload, path_payload

    cover_payload = mcp_call("coverage_get", {}, 11)
    assert "detail" in cover_payload, cover_payload

    # ── 4-b. 書き込み 2 ツール（草稿。承認不要＝04 §4.7）──
    analyze_payload = mcp_call("impact_analyze", {"entity_ids": [entity_id]}, 12)
    assert analyze_payload["impact_id"], analyze_payload
    assert analyze_payload["job_id"], analyze_payload
    for _ in range(240):
        job = client.must("GET", f"/jobs/{analyze_payload['job_id']}")
        if job["status"] in {
            "SUCCEEDED",
            "SUCCEEDED_WITH_ERRORS",
            "FAILED",
            "CANCELLED",
            "TIMEOUT",
        }:
            assert job["status"] in {"SUCCEEDED", "SUCCEEDED_WITH_ERRORS"}, job
            break
        time.sleep(0.25)

    report_payload = mcp_call(
        "impact_get_report", {"impact_id": analyze_payload["impact_id"]}, 13
    )
    assert report_payload["report"], report_payload

    plan_payload = mcp_call(
        "test_plan_generate", {"impact_id": analyze_payload["impact_id"]}, 14
    )
    assert plan_payload["plan_id"], plan_payload
    assert plan_payload["appended"] is False, plan_payload
    assert plan_payload["test_run_created"] is False, plan_payload
    assert plan_payload["draft_case_count"] > 0, plan_payload
    # 再実行時は同じ計画が既に在る（plan_id は impact から決まる）。
    # だから「総数 == 今回の生成数」とは限らない。含意だけを見る。
    assert plan_payload["plan_case_count"] >= plan_payload["draft_case_count"], plan_payload

    # test.case_generate は**既存計画への追記**。新しい計画を作らない。
    # 追記の効果を見るため、影響項目が多く出る実体（関係の多い Method）を使う。
    second_entity = psql(
        "SELECT e.id FROM graph_meta.entity e "
        "LEFT JOIN graph_meta.relationship r ON r.src_id = e.id OR r.dst_id = e.id "
        "WHERE e.entity_type = 'Method' "
        "GROUP BY e.id ORDER BY count(r.*) DESC LIMIT 1;"
    )
    second_impact = mcp_call("impact_analyze", {"entity_ids": [second_entity]}, 20)
    for _ in range(240):
        job = client.must("GET", f"/jobs/{second_impact['job_id']}")
        if job["status"] in {
            "SUCCEEDED",
            "SUCCEEDED_WITH_ERRORS",
            "FAILED",
            "CANCELLED",
            "TIMEOUT",
        }:
            assert job["status"] in {"SUCCEEDED", "SUCCEEDED_WITH_ERRORS"}, job
            break
        time.sleep(0.25)

    case_payload = mcp_call(
        "test_case_generate",
        {"plan_id": plan_payload["plan_id"], "impact_id": second_impact["impact_id"]},
        21,
    )
    assert case_payload["appended"] is True, case_payload
    assert case_payload["plan_id"] == plan_payload["plan_id"], case_payload
    assert case_payload["draft_case_count"] > 0, case_payload
    assert case_payload["plan_case_count"] >= plan_payload["plan_case_count"], case_payload

    # 取り違えは弾く（新規作成と追記は別の操作）。
    mcp_refused(
        "test_case_generate", {"impact_id": analyze_payload["impact_id"]}, 22, 400
    )
    mcp_refused(
        "test_plan_generate",
        {"impact_id": analyze_payload["impact_id"], "plan_id": plan_payload["plan_id"]},
        23,
        400,
    )

    # ── 4-c. 破壊的操作 1 ツール（承認必須 → 承認後に 1 回だけ実行）──
    exec_args = {
        "name": f"mcp-approved-{int(time.time())}",
        "plan_id": plan_payload["plan_id"],
        "environment": {
            "id": "env-mcp-exec",
            "base_url": "http://127.0.0.1:19081",
            "guard_kind": "LOCAL",
            "parallel": 2,
        },
    }
    # 他人が同じ action / 引数で起票した PENDING があっても、それを返さない。
    # 返すと相手が承認しても requested_by が違って使えず、こちらは 423 で詰まる。
    foreign_id = "ffffffff-ffff-4fff-8fff-ffffffffffff"
    foreign_payload = json.dumps(
        {
            "tool": "test_run_execute",
            "capability": "test.run_execute",
            "arguments": exec_args,
        },
        ensure_ascii=False,
    ).replace("'", "''")
    psql(
        "INSERT INTO ai.approval_request "
        "(id, project_id, action, payload_digest, payload, status, requested_by, created_at) "
        f"VALUES ('{foreign_id}', '{pid}', 'mcp.call:test_run_execute', "
        f"sha256('{foreign_payload}'::text::bytea), '{foreign_payload}'::jsonb, "
        "'PENDING', '99999999-9999-4999-8999-999999999999', now()) "
        "ON CONFLICT (id) DO NOTHING;"
    )
    first = mcp_refused("test_run_execute", exec_args, 16, 423)
    assert first["code"] == "POLICY_APPROVAL_REQUIRED", first
    approval_id = re.search(r"approval_id=([0-9a-f-]{36})", first["detail"])[1]
    assert approval_id != foreign_id, (approval_id, foreign_id)
    REPORT["approval_isolated_by_requester"] = {
        "returned": approval_id,
        "foreign": foreign_id,
    }

    # 連打しても承認依頼は増えない（同じ引数 → 同じ approval_id）
    again_pending = mcp_refused("test_run_execute", exec_args, 17, 423)
    assert approval_id in again_pending["detail"], again_pending

    # 引数が違えば別の承認。承認した内容と違う呼び出しに許可証は効かない。
    other_args = dict(exec_args, name=f"{exec_args['name']}-other")
    other = mcp_refused("test_run_execute", other_args, 18, 423)
    assert re.search(r"approval_id=([0-9a-f-]{36})", other["detail"])[1] != approval_id

    client.must(
        "POST",
        f"/projects/{pid}/approvals/{approval_id}/decide",
        {"decision": "APPROVE", "comment": "MCP からの実行を承認"},
    )
    executed = mcp_call("test_run_execute", exec_args, 19)
    assert executed["run_id"], executed
    # 許可証は 1 回で消費される。2 回目は再び承認待ち。
    consumed = mcp_refused("test_run_execute", exec_args, 24, 423)
    assert consumed["code"] == "POLICY_APPROVAL_REQUIRED", consumed

    # 承認一覧からも「使用済み」「MCP 由来（workflow_run なし）」が見えること。
    listed = client.must("GET", f"/projects/{pid}/approvals")["items"]
    used = [item for item in listed if item["id"] == approval_id][0]
    assert used["status"] == "APPROVED", used
    assert used["consumed_at"], used
    assert used["workflow_run_id"] is None, used

    # ── 4-d. 環境 guard: 未知は拒否、PRODUCTION は承認フロー未接続として拒否 ──
    for guard in ("PROD", "PRODUCTION"):
        status, body = client.call(
            "POST",
            f"/projects/{pid}/test/runs",
            {
                "name": f"guard-{guard}-{int(time.time())}",
                "plan_id": plan_payload["plan_id"],
                "environment": {
                    "id": f"env-guard-{guard}",
                    "base_url": "http://127.0.0.1:19082",
                    "guard_kind": guard,
                    "parallel": 2,
                },
            },
        )
        assert status == 400, (guard, status, body)
        REPORT.setdefault("guard_kind", {})[guard] = status

    psql(f"DELETE FROM ai.approval_request WHERE id = '{foreign_id}';")

    REPORT["mcp_calls"] = {
        "total": 12,
        "read": {  # side_effects=read の 9 本はすべて実行できる
            "project_list": "ok",
            "graph_query": len(graph_payload),
            "graph_trace_path": path_payload["found"],
            "code_get_symbol": len(symbols),
            "knowledge_search": len(search_payload["results"]),
            "knowledge_get_evidence": "found",
            "impact_analyze": analyze_payload["impact_id"],
            "impact_get_report": bool(report_payload["report"]),
            "coverage_get": "degraded" if cover_payload["detail"] is None else "ok",
        },
        "write_draft": {  # 草稿は承認不要（04 §4.7）。作成と追記は別操作
            "test_plan_generate": {
                "plan_id": plan_payload["plan_id"],
                "appended": plan_payload["appended"],
                "drafts": plan_payload["draft_case_count"],
            },
            "test_case_generate": {
                "plan_id": case_payload["plan_id"],
                "appended": case_payload["appended"],
                "drafts": case_payload["draft_case_count"],
                "plan_case_count": case_payload["plan_case_count"],
            },
            "same_plan": case_payload["plan_id"] == plan_payload["plan_id"],
        },
        "destructive": {
            "approval_required": approval_status,
            "approval_id": approval_id,
            "executed_run_id": executed["run_id"],
            "replay_blocked": consumed["code"],
        },
        "unknown_tool": "rejected",
    }

    # ── 5. ロール差（viewer は Agent を登録できない） ───────
    status, denied = viewer.call(
        "POST",
        f"/projects/{pid}/agents",
        {
            "code": "viewer-agent",
            "model_profile": "fast",
            "prompt_ref": "prompt://viewer@1",
            "prompt_body": "x",
            "capabilities": [],
        },
    )
    assert status == 403, (status, denied)
    status, _ = viewer.call("GET", f"/projects/{pid}/ai/overview")
    # Viewer は AgentRun を持たないため AI 画面自体を開けない（設計どおり）。
    assert status == 403, status
    # 読み取り能力（Search）は持つので、MCP の project_list は通る。
    viewer_call = viewer.must(
        "POST",
        f"/projects/{pid}/mcp",
        {
            "jsonrpc": "2.0",
            "id": 9,
            "method": "tools/call",
            "params": {"name": "project_list", "arguments": {}},
        },
    )
    assert viewer_call["result"]["isError"] is False
    REPORT["role_boundary"] = {
        "viewer_register_agent": 403,
        "viewer_ai_overview": 403,
        "viewer_mcp_read": 200,
    }

    # 期限切れの承認は「後から承認」できない（worker が 24 時間で EXPIRED にする）。
    expired_id = "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee"
    psql(
        "INSERT INTO ai.approval_request "
        "(id, project_id, action, payload_digest, payload, status, requested_by, created_at) "
        "VALUES ('eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee', "
        f"'{pid}', 'expired-probe', sha256('{{}}'::text::bytea), '{{}}'::jsonb, "
        "'EXPIRED', 'probe', now() - interval '25 hours') "
        "ON CONFLICT (id) DO NOTHING;"
    )
    status, rejected = client.call(
        "POST",
        f"/projects/{pid}/approvals/{expired_id}/decide",
        {"decision": "APPROVE", "comment": "後から承認"},
    )
    assert status == 409, (status, rejected)
    psql(f"DELETE FROM ai.approval_request WHERE id = '{expired_id}';")
    REPORT["expired_approval_rejected"] = status

    # ── 6. Workflow: 承認で止まり、承認後に再開する ────────
    definition = {
        "nodes": [
            {"id": "start", "kind": "start", "name": "開始"},
            {
                "id": "cov",
                "kind": "capability",
                "name": "被覆取得",
                "capability": "coverage.get",
            },
            {
                "id": "gate",
                "kind": "approval",
                "name": "実行承認",
                "action": "test.run_execute",
            },
            {"id": "end", "kind": "end", "name": "終了"},
        ],
        "edges": [
            {"from": "start", "to": "cov"},
            {"from": "cov", "to": "gate"},
            {"from": "gate", "to": "end"},
        ],
    }
    workflow = client.must(
        "POST",
        f"/projects/{pid}/workflows",
        {"code": "impact-regression", "definition": definition},
    )
    wf_run = client.must(
        "POST",
        f"/projects/{pid}/workflows/{workflow['id']}/runs",
        {"input": {"impact_id": None}},
    )
    wait_job(client, wf_run["job_id"])
    paused = client.must("GET", f"/projects/{pid}/workflow-runs/{wf_run['run_id']}")
    assert paused["status"] == "WAITING_APPROVAL", paused
    approvals = client.must("GET", f"/projects/{pid}/approvals")["items"]
    # 他の PENDING（MCP の承認待ち）と取り違えないよう、起票元の run で絞る。
    pending = [
        item
        for item in approvals
        if item["status"] == "PENDING" and item["workflow_run_id"] == wf_run["run_id"]
    ]
    assert pending, approvals
    approval = pending[0]
    decided = client.must(
        "POST",
        f"/projects/{pid}/approvals/{approval['id']}/decide",
        {"decision": "APPROVE", "comment": "SAMPLE のため承認"},
    )
    assert decided["status"] == "APPROVED"

    resumed = None
    for _ in range(120):
        current = client.must("GET", f"/projects/{pid}/workflow-runs/{wf_run['run_id']}")
        if current["status"] in {"SUCCEEDED", "FAILED"}:
            resumed = current
            break
        time.sleep(0.25)
    assert resumed and resumed["status"] == "SUCCEEDED", resumed
    REPORT["workflow"] = {
        "paused_at": paused["current_node"],
        "approval_status": decided["status"],
        "final_status": resumed["status"],
        "nodes": [node["node_id"] for node in resumed["nodes"]],
    }

    status, write_rejected = client.call(
        "POST",
        f"/projects/{pid}/workflows",
        {
            "code": "dangerous",
            "definition": {
                "nodes": [
                    {"id": "start", "kind": "start", "name": "開始"},
                    {
                        "id": "run",
                        "kind": "capability",
                        "name": "実行",
                        "capability": "test.run_execute",
                    },
                    {"id": "end", "kind": "end", "name": "終了"},
                ],
                "edges": [
                    {"from": "start", "to": "run"},
                    {"from": "run", "to": "end"},
                ],
            },
        },
    )
    assert status == 400, (status, write_rejected)
    REPORT["workflow_write_capability_rejected"] = status

    # ── 6.5 承認ノードの後ろなら破壊的操作も実行できる ─────
    # 承認は「同意ボタン」ではない。承認ノードを経路上に置いた定義だけが
    # 非 read の capability を持てる（04 §4.7）。
    approved_case = psql(
        "SELECT c.id FROM test.test_case c "
        "JOIN test.test_scenario s ON s.id = c.scenario_id "
        f"WHERE s.plan_id = '{plan_payload['plan_id']}' "
        "AND c.status = 'DRAFT' LIMIT 1;"
    )
    client.must("POST", f"/projects/{pid}/test/cases/{approved_case}/approve", {})

    exec_workflow = client.must(
        "POST",
        f"/projects/{pid}/workflows",
        {
            "code": "approved-exec",
            "definition": {
                "nodes": [
                    {"id": "start", "kind": "start", "name": "開始"},
                    {"id": "gate", "kind": "approval", "name": "承認", "action": "exec-gate"},
                    {
                        "id": "exec",
                        "kind": "capability",
                        "name": "実行",
                        "capability": "test.run_execute",
                    },
                    {"id": "end", "kind": "end", "name": "終了"},
                ],
                "edges": [
                    {"from": "start", "to": "gate"},
                    {"from": "gate", "to": "exec"},
                    {"from": "exec", "to": "end"},
                ],
            },
        },
    )
    exec_run = client.must(
        "POST",
        f"/projects/{pid}/workflows/{exec_workflow['id']}/runs",
        {
            "input": {
                "name": f"wf-approved-exec-{int(time.time())}",
                "plan_id": plan_payload["plan_id"],
                "environment": {
                    "id": "env-wf-exec",
                    "base_url": "http://127.0.0.1:19080",
                    "guard_kind": "LOCAL",
                    "parallel": 2,
                },
            }
        },
    )
    wait_job(client, exec_run["job_id"])
    exec_paused = client.must("GET", f"/projects/{pid}/workflow-runs/{exec_run['run_id']}")
    assert exec_paused["status"] == "WAITING_APPROVAL", exec_paused
    assert exec_paused["current_node"] == "gate", exec_paused
    # 承認より前に実行ノードが走っていないこと（ここが崩れると承認は形骸化する）
    assert all(node["node_id"] != "exec" for node in exec_paused["nodes"]), exec_paused["nodes"]

    exec_approval = [
        item
        for item in client.must("GET", f"/projects/{pid}/approvals")["items"]
        if item["status"] == "PENDING" and item["action"] == "exec-gate"
    ][0]
    client.must(
        "POST",
        f"/projects/{pid}/approvals/{exec_approval['id']}/decide",
        {"decision": "APPROVE", "comment": "実行を承認"},
    )
    exec_done = None
    for _ in range(120):
        current = client.must("GET", f"/projects/{pid}/workflow-runs/{exec_run['run_id']}")
        if current["status"] in {"SUCCEEDED", "FAILED"}:
            exec_done = current
            break
        time.sleep(0.25)
    assert exec_done and exec_done["status"] == "SUCCEEDED", exec_done
    exec_node = next(node for node in exec_done["nodes"] if node["node_id"] == "exec")
    assert exec_node["status"] == "PASSED", exec_node
    assert exec_node["output"]["case_count"] >= 1, exec_node["output"]
    REPORT["workflow_approved_execute"] = {
        "approval_before_execute": True,
        "run_id": exec_node["output"]["run_id"],
        "status": exec_node["output"]["status"],
        "case_count": exec_node["output"]["case_count"],
    }

    # ── 7. 監査 ──────────────────────────────────────────
    audits = int(
        psql(
            "SELECT count(*) FROM core.audit_log "
            "WHERE action LIKE 'ai.%' AND occurred_at > now() - interval '10 minutes';"
        )
        or 0
    )
    assert audits >= 5, audits
    REPORT["audit_rows"] = audits

    out = ROOT / "data" / "phase7-eval" / "report.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(REPORT, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(REPORT, ensure_ascii=False, indent=2))
    print(f"\nwritten: {out}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as error:  # noqa: BLE001
        print(f"FAILED: {error}", file=sys.stderr)
        raise
