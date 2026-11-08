#!/usr/bin/env python3
"""Phase 6 Test Engineering 縦断実装の受入（API + worker E2E）。

前提: 001〜015 と roles.sql を適用した DB、API、worker が起動済み。
使い方:
  SYSTIVRA_BASE_URL=http://127.0.0.1:18086 PGDATABASE=systivra_phase6_verify \
    python3 scripts/phase6-test-eval.py

Runner は未接続なので、実行結果は `executions:import` で `execution_source=IMPORTED`
として取り込む。PASSED をこのスクリプトが「実測」と偽ることはしない。
"""

from __future__ import annotations

import http.cookiejar
import json
import os
import pathlib
import subprocess
import sys
import time
import uuid
import urllib.error
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[1]
BASE_URL = os.environ.get("SYSTIVRA_BASE_URL", "http://127.0.0.1:18080")
LOGIN_ID = os.environ.get("SYSTIVRA_LOGIN_ID", "alice")
PASSWORD = os.environ.get("SYSTIVRA_PASSWORD", "SystivraDemo1!")
DB = os.environ.get("PGDATABASE", "systivra")
IMPACT_ID = "a6a6a6a6-a6a6-4a6a-8a6a-a6a6a6a6a6a6"

cookies = http.cookiejar.CookieJar()
opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(cookies))
REPORT: dict = {}


def call(method: str, path: str, body: dict | None = None) -> tuple[int, dict | None]:
    data = None
    headers = {}
    if body is not None:
        data = json.dumps(body, ensure_ascii=False).encode("utf-8")
        headers["content-type"] = "application/json"
    request = urllib.request.Request(
        f"{BASE_URL}/api/v1{path}", data=data, headers=headers, method=method
    )
    try:
        with opener.open(request, timeout=30) as response:
            payload = response.read()
            return response.status, json.loads(payload) if payload else None
    except urllib.error.HTTPError as error:
        raw = error.read()
        try:
            parsed = json.loads(raw) if raw else None
        except json.JSONDecodeError:
            parsed = {"raw": raw.decode("utf-8", "replace")}
        return error.code, parsed


def must(method: str, path: str, body: dict | None = None) -> dict:
    status, payload = call(method, path, body)
    if status >= 400:
        raise AssertionError(f"{method} {path} -> HTTP {status}: {payload}")
    assert payload is not None, f"{method} {path} returned empty body"
    return payload


def psql(sql: str) -> str:
    result = subprocess.run(
        ["psql", "-d", DB, "-v", "ON_ERROR_STOP=1", "-tA", "-c", sql],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(f"psql failed: {result.stderr.strip()}")
    return result.stdout.strip()


def wait_job(job_id: str) -> dict:
    terminal = {"SUCCEEDED", "SUCCEEDED_WITH_ERRORS", "FAILED", "CANCELLED", "TIMEOUT"}
    for _ in range(240):
        job = must("GET", f"/jobs/{job_id}")
        if job["status"] in terminal:
            if job["status"] not in terminal - {"FAILED", "CANCELLED", "TIMEOUT"}:
                raise AssertionError(f"job {job_id} ended as {job['status']}: {job.get('errors')}")
            return job
        time.sleep(0.25)
    raise TimeoutError(f"job {job_id} did not finish")


def dollar(value: dict) -> str:
    return "$j$" + json.dumps(value, ensure_ascii=False) + "$j$::jsonb"


def main() -> int:
    # 受入は何度でも同じ結果になるよう、test schema だけを初期化する。
    psql(
        "TRUNCATE test.validation_rule, test.test_plan, test.test_scenario, "
        "test.test_case, test.test_step, test.test_data_set, test.test_data_row, "
        "test.test_run, test.test_execution, test.defect, test.coverage_snapshot "
        "RESTART IDENTITY CASCADE;"
    )
    psql("DELETE FROM core.job WHERE kind = 'TEST_RUN';")
    # テスト資産のグラフ投影も初期化する（revision='test' のものだけ）。
    psql("DELETE FROM graph_meta.relationship WHERE revision = 'test';")
    psql("DELETE FROM graph_meta.entity WHERE revision = 'test';")
    psql(f"DELETE FROM impact.assessment_draft WHERE id = '{IMPACT_ID}';")

    login = must("POST", "/auth/login", {"login_id": LOGIN_ID, "password": PASSWORD})
    pid = login["memberships"][0]["project_id"]

    # Q-01 の Impact Draft fixture。実コード解析をやり直さずに R-16 を測る。
    impact_report = {
        "items": [
            {
                "item_id": "b1b1b1b1-b1b1-4b1b-8b1b-b1b1b1b1b1b1",
                "label": "UserService.save(UserDto)",
                "category": "CODE",
                "why": "変更ファイル内の Method",
            },
            {
                "item_id": "b2b2b2b2-b2b2-4b2b-8b2b-b2b2b2b2b2b2",
                "label": "ユーザ編集画面",
                "category": "SCREENS",
                "why": "Action から到達",
            },
        ],
        "degraded": ["tests_not_available"],
        "approval_state": "DRAFT",
    }
    psql(
        "INSERT INTO impact.assessment_draft (id, project_id, input, status, report) "
        f"VALUES ('{IMPACT_ID}', '{pid}', '{{}}', 'SUCCEEDED', {dollar(impact_report)}) "
        "ON CONFLICT (id) DO UPDATE SET report = EXCLUDED.report;"
    )

    # ── 1. ルール抽出（競合は両方残す / 読めない記述は数える） ──
    rules_response = must(
        "POST",
        f"/projects/{pid}/test/rules:extract",
        {
            "sources": [
                {"kind": "CODE", "field_ref": "user.addr",
                 "text": "if (s.length() > 20) errors.add(...)",
                 "evidence_ids": ["c1c1c1c1-c1c1-4c1c-8c1c-c1c1c1c1c1c1"]},
                {"kind": "SPEC", "field_ref": "user.addr",
                 "text": "住所は全角30文字以内",
                 "evidence_ids": ["c2c2c2c2-c2c2-4c2c-8c2c-c2c2c2c2c2c2"]},
                {"kind": "SPEC", "field_ref": "user.name", "text": "必須項目",
                 "evidence_ids": []},
                {"kind": "SPEC", "field_ref": "user.age", "text": "min=1 max=99",
                 "evidence_ids": []},
                {"kind": "SPEC", "field_ref": "user.memo", "text": "特記事項なし",
                 "evidence_ids": []},
            ]
        },
    )
    assert rules_response["conflicts"] == 1, rules_response
    assert rules_response["unparsed"] == 1, rules_response
    # 5 入力のうち 1 件は読めず、残り 4 件がルールになる。
    assert len(rules_response["rules"]) >= 4, rules_response
    assert "rules_conflict" in rules_response["degraded"]
    REPORT["rules"] = {
        "parsed": len(rules_response["rules"]),
        "conflicts": rules_response["conflicts"],
        "unparsed": rules_response["unparsed"],
        "degraded": rules_response["degraded"],
    }

    # ── 2. Q-01 → DRAFT 計画。test_run は作らない（R-16） ──
    derived = must(
        "POST",
        f"/projects/{pid}/test/plans:derive",
        {"impact_id": IMPACT_ID, "name": "Q-01 由来の計画", "max_drafts": 20},
    )
    assert derived["test_run_created"] is False
    assert derived["draft_case_count"] > 0
    assert derived["scenario_count"] > 0
    REPORT["derive"] = derived

    overview = must("GET", f"/projects/{pid}/test/overview")
    assert not overview["runs"], "derive が test_run を作ってしまった"
    assert derived["plan_id"] in [p["id"] for p in overview["plans"]]
    assert all(c["status"] == "DRAFT" for c in overview["cases"])
    assert all(not c["validation_errors"] for c in overview["cases"])
    assert "runner_external_process" in overview["degraded"]
    REPORT["overview_initial"] = {
        "cases": len(overview["cases"]),
        "data": len(overview["data"]),
        "plans": len(overview["plans"]),
        "degraded": overview["degraded"],
    }

    # ── 3. DSL 保存時検証（R-08 / R-14 / R-15） ──
    bad_case = overview["cases"][0]
    original_dsl = bad_case["dsl"]
    illegal = json.loads(json.dumps(original_dsl))
    illegal["steps"] = [
        {"action": "goto", "target": {"path": "/x"}},
        {"action": "wait_for", "target": {}, "ms": 6000},
        {"action": "teleport", "target": {}},
        {"action": "goto", "target": {"url": "https://evil.example.com"}},
    ]
    psql(
        "UPDATE test.test_case SET dsl = "
        f"{dollar(illegal)} WHERE id = '{bad_case['id']}';"
    )
    status, rejected = call("POST", f"/projects/{pid}/test/cases/{bad_case['id']}/approve")
    assert status == 422, f"不正 DSL が承認された: {status} {rejected}"
    detail = (rejected or {}).get("detail", "")
    for expected in ["wait_for_ms_exceeds_5000", "unknown_action", "origin_not_allowed"]:
        assert expected in detail, f"{expected} が 422 に出ていない: {detail}"
    REPORT["dsl_rejection"] = {"status": status, "detail": detail}
    psql(
        "UPDATE test.test_case SET dsl = "
        f"{dollar(original_dsl)} WHERE id = '{bad_case['id']}';"
    )

    # ── 4. 承認（最後の 1 件は DRAFT のまま残し、R-03 に使う） ──
    all_cases = overview["cases"]
    approved = all_cases[:-1]
    draft_case = all_cases[-1]
    for case in approved:
        response = must("POST", f"/projects/{pid}/test/cases/{case['id']}/approve")
        assert response["status"] == "APPROVED", response
    REPORT["approved"] = len(approved)

    # ── 5. run 起票と環境ロック（R-12） ──
    env = {
        "id": "env-stg-01",
        "base_url": "http://127.0.0.1:18086/app",
        "guard_kind": "STAGING",
        "allowed_origins": ["127.0.0.1"],
        "parallel": 4,
    }
    first = must("POST", f"/projects/{pid}/test/runs",
                 {"name": "run-1", "plan_id": None, "environment": env})
    assert first["job_id"] and not first["queued_behind"], first
    second = must("POST", f"/projects/{pid}/test/runs",
                  {"name": "run-2", "plan_id": None, "environment": env})
    assert second["queued_behind"] and second["job_id"] is None, second
    REPORT["environment_lock"] = {
        "first": {"run_id": first["run_id"], "status": first["status"]},
        "second": {"run_id": second["run_id"], "queued_behind": second["queued_behind"]},
    }

    run_id = first["run_id"]
    job = wait_job(first["job_id"])
    assert job["status"] == "SUCCEEDED_WITH_ERRORS", job
    REPORT["prepare_job"] = {"status": job["status"], "errors": job.get("errors")}

    # ── 6. 被覆の再現性と「未実行は 0」（R-02 / R-03 / R-04） ──
    coverage_request = {"test_run_id": run_id}
    c1 = must("POST", f"/projects/{pid}/test/coverage:recompute", coverage_request)
    c2 = must("POST", f"/projects/{pid}/test/coverage:recompute", coverage_request)
    assert c1 == c2, "同じ test_run の被覆が再現しない"
    detail1 = c1["detail"]
    assert detail1["rule"]["num"] == 0, detail1
    assert detail1["requirement"]["num"] == 0
    assert detail1["requirement"]["den"] > 0, "Requirement が分母に入っていない"
    assert "REQUIREMENT_INFERRED_FROM_RULE" in detail1["requirement"]["degraded"]
    assert detail1["excluded"]["never_executed"] > 0
    assert detail1["excluded"]["draft_cases"] >= 1
    REPORT["coverage_before"] = {
        "rule": detail1["rule"], "requirement": detail1["requirement"],
        "excluded": detail1["excluded"], "degraded": detail1["degraded"],
    }

    # ── 7. 実行結果の取り込み（R-05 / R-06 / R-13） ──
    timeout_case = approved[0]["id"]
    assertion_case = approved[1]["id"]
    batch1 = [
        {"case_id": timeout_case, "attempt": 1, "status": "FAILED",
         "failure_kind": "TIMEOUT", "failure_step": 1, "execution_source": "IMPORTED"},
        {"case_id": assertion_case, "attempt": 1, "status": "FAILED",
         "failure_kind": "ASSERTION", "failure_step": 3,
         "expected": "３ 件", "actual": "4 件", "execution_source": "IMPORTED"},
    ]
    batch1 += [
        {"case_id": c["id"], "attempt": 1, "status": "PASSED",
         "execution_source": "IMPORTED"}
        for c in approved[2:]
    ]
    # DRAFT ケースは被覆に算入されない（R-03）。実行自体も拒否する。
    batch1.append({"case_id": draft_case["id"], "attempt": 1,
                   "status": "PASSED", "execution_source": "IMPORTED"})
    imported1 = must("POST", f"/projects/{pid}/test/runs/{run_id}/executions:import",
                     {"executions": batch1})
    assert any("case_not_approved_or_unknown" in r for r in imported1["rejected"]), imported1
    assert imported1["accepted"] == len(approved), imported1

    batch2 = [
        # TIMEOUT は 1 回だけ再試行可（R-06）。
        {"case_id": timeout_case, "attempt": 2, "status": "PASSED",
         "execution_source": "IMPORTED"},
        # ASSERTION は再試行しない（R-05）。
        {"case_id": assertion_case, "attempt": 2, "status": "PASSED",
         "execution_source": "IMPORTED"},
        # 同一 fingerprint の再発。欠陥候補は 1 件のまま recurrence が増える（R-13）。
        {"case_id": assertion_case, "attempt": 1, "status": "FAILED",
         "failure_kind": "ASSERTION", "failure_step": 3,
         "expected": "3 件", "actual": "4 件", "execution_source": "IMPORTED"},
    ]
    imported2 = must("POST", f"/projects/{pid}/test/runs/{run_id}/executions:import",
                     {"executions": batch2})
    assert any("retry_not_allowed:ASSERTION" in r for r in imported2["rejected"]), imported2
    REPORT["imports"] = {
        "first": imported1, "second": imported2,
    }

    final_overview = must("GET", f"/projects/{pid}/test/overview")
    fingerprints = [
        d for d in final_overview["defects"]
        if d["fingerprint"] and d["execution_id"] is not None
    ]
    assert fingerprints, "欠陥候補が 1 件も無い"
    assert max(d["recurrence"] for d in fingerprints) >= 2, fingerprints
    REPORT["defects"] = {
        "count": len(final_overview["defects"]),
        "max_recurrence": max(d["recurrence"] for d in fingerprints),
    }

    c3 = must("POST", f"/projects/{pid}/test/coverage:recompute", coverage_request)
    assert c3["detail"]["rule"]["num"] > 0, c3["detail"]
    assert c3["detail"]["regression"]["num"] > 0, c3["detail"]
    assert c3["detail"]["requirement"]["num"] > 0, "VERIFIES 辺から要件被覆が算入されていない"
    REPORT["coverage_after"] = {
        "rule": c3["detail"]["rule"],
        "screen": c3["detail"]["screen"],
        "impact": c3["detail"]["impact"],
        "regression": c3["detail"]["regression"],
        "requirement": c3["detail"]["requirement"],
        "excluded": c3["detail"]["excluded"],
    }

    # ── 8. PRODUCTION は API が受け付けず、既存 run の取込も SKIPPED（R-07） ──
    # 8-a. 実行権限だけでは本番に流せない（承認フローが未接続なので拒否）。
    prod_env = dict(env, id="env-prod-01", guard_kind="PRODUCTION")
    status, refused = call(
        "POST",
        f"/projects/{pid}/test/runs",
        {"name": "run-prod", "plan_id": None, "environment": prod_env},
    )
    assert status == 400, (status, refused)
    assert "PRODUCTION" in refused["detail"], refused
    # 未知の環境名も同じく拒否（fail-closed の入口）。
    status, unknown = call(
        "POST",
        f"/projects/{pid}/test/runs",
        {
            "name": "run-prod-typo",
            "plan_id": None,
            "environment": dict(env, id="env-prod-02", guard_kind="prod"),
        },
    )
    assert status == 400, (status, unknown)

    # 8-b. 既に在る（あるいは外から持ち込まれた）PRODUCTION の run でも、
    # 破壊的ケースの取込結果は SKIPPED に落とす。入口だけに頼らない。
    prod_run_id = str(uuid.uuid4())
    prod_env_json = json.dumps({**prod_env, "guard_kind": "PRODUCTION"})
    psql(
        "INSERT INTO test.test_run "
        "(id, project_id, plan_id, name, environment, environment_id, status, "
        " case_count, lock_acquired) VALUES "
        f"('{prod_run_id}', '{pid}', NULL, 'run-prod-direct', "
        f"'{prod_env_json}'::jsonb, 'env-prod-01', 'READY', 0, false);"
    )
    psql("UPDATE test.test_case SET destructive = true "
         f"WHERE id = '{approved[0]['id']}';")
    prod_import = must(
        "POST",
        f"/projects/{pid}/test/runs/{prod_run_id}/executions:import",
        {"executions": [{"case_id": approved[0]["id"], "attempt": 1,
                         "status": "PASSED", "execution_source": "IMPORTED"}]},
    )
    assert prod_import["accepted"] == 1, prod_import
    prod_overview = must("GET", f"/projects/{pid}/test/overview")
    prod_exec = [
        e for e in prod_overview["executions"] if e["test_run_id"] == prod_run_id
    ]
    assert prod_exec and prod_exec[0]["status"] == "SKIPPED", prod_exec
    REPORT["production_guard"] = {
        "run_create": status,
        "import": prod_exec[0]["status"],
    }
    psql("UPDATE test.test_case SET destructive = false "
         f"WHERE id = '{approved[0]['id']}';")

    # ── 9. 環境ロック解放後の後続 run ──
    after = must("GET", f"/projects/{pid}/test/overview")
    second_run = next(r for r in after["runs"] if r["id"] == second["run_id"])
    REPORT["queued_run_after_release"] = {
        "status": second_run["status"],
        "lock_acquired": second_run["lock_acquired"],
        "job_id": second_run["job_id"],
    }

    out = ROOT / "data" / "phase6-eval" / "report.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(REPORT, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(REPORT, ensure_ascii=False, indent=2))
    print(f"\nwritten: {out}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as error:  # noqa: BLE001 - 受入スクリプトは失敗内容をそのまま出す
        print(f"FAILED: {error}", file=sys.stderr)
        raise
