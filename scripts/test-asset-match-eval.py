#!/usr/bin/env python3
"""既存テスト資産の照合（`test_coverage`）を実測する。

前提: 001〜030 適用済み DB と API + worker。
  SYSTIVRA_BASE_URL=http://127.0.0.1:18086 PGDATABASE=systivra_phase6_verify \
    python3 scripts/test-asset-match-eval.py

流れ:
  1. 図の実体を 1 つ選び、ENTITY seed で影響評価を回す（既存テストは 0 件）。
  2. `test_coverage.covering_cases` が空で `tests_not_available` が立つことを確認。
  3. その影響項目を覆う既存ケースを 1 件作り、同じ seed で再評価。
  4. 照合され、`tests_not_available` が消えることを確認。最後に片付ける。
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
BASE = os.environ.get("SYSTIVRA_BASE_URL", "http://127.0.0.1:18086")
DB = os.environ.get("PGDATABASE", "systivra")
PROJECT = os.environ.get("SYSTIVRA_PROJECT_ID", "22222222-2222-4222-8222-222222222222")
PLAN_ID = "c1c1c1c1-c1c1-4c1c-8c1c-c1c1c1c1c1c1"
SCENARIO_ID = "c2c2c2c2-c2c2-4c2c-8c2c-c2c2c2c2c2c2"
CASE_ID = "c3c3c3c3-c3c3-4c3c-8c3c-c3c3c3c3c3c3"

COOKIES: dict[str, str] = {}
OPENER = urllib.request.build_opener(
    urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar())
)


def call(method: str, path: str, body: dict | None = None) -> tuple[int, dict]:
    data = json.dumps(body, ensure_ascii=False).encode("utf-8") if body is not None else None
    request = urllib.request.Request(
        f"{BASE}/api/v1{path}", data=data, method=method
    )
    if body is not None:
        request.add_header("content-type", "application/json")
    try:
        with OPENER.open(request, timeout=30) as response:
            raw = response.read()
            return response.status, (json.loads(raw) if raw else {})
    except urllib.error.HTTPError as error:
        raw = error.read()
        try:
            return error.code, json.loads(raw) if raw else {}
        except json.JSONDecodeError:
            return error.code, {"raw": raw.decode("utf-8", "replace")}


def psql(sql: str) -> str:
    result = subprocess.run(
        ["psql", "-d", DB, "-v", "ON_ERROR_STOP=1", "-tA", "-c", sql],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(f"psql failed: {result.stderr.strip()}")
    return result.stdout.strip()


def wait_job(job_id: str) -> None:
    terminal = {"SUCCEEDED", "SUCCEEDED_WITH_ERRORS", "FAILED", "CANCELLED", "TIMEOUT"}
    for _ in range(480):
        _, job = call("GET", f"/jobs/{job_id}")
        if job.get("status") in terminal:
            if job["status"] not in {"SUCCEEDED", "SUCCEEDED_WITH_ERRORS"}:
                raise RuntimeError(f"job {job_id} ended as {job['status']}: {job.get('errors')}")
            return
        time.sleep(0.25)
    raise TimeoutError(f"job {job_id} did not finish")


def cleanup() -> None:
    psql(
        f"DELETE FROM test.test_case WHERE id='{CASE_ID}';"
        f"DELETE FROM test.test_scenario WHERE id='{SCENARIO_ID}';"
        f"DELETE FROM test.test_plan WHERE id='{PLAN_ID}';"
    )


def main() -> int:
    status, _ = call(
        "POST", "/auth/login", {"login_id": "alice", "password": "SystivraDemo1!"}
    )
    assert status == 200, status

    # コード系の実体を選ぶ。TestCase / Requirement は証跡を持たず items が空になる。
    _, entities = call("GET", f"/projects/{PROJECT}/graph/entities?limit=200")
    items = entities.get("items", [])
    assert items, "図の実体が無い（先にコード解析と投影が必要）"
    preferred = ("Method", "Action", "Class", "Screen", "Table", "Module")
    seed = next(
        (item for kind in preferred for item in items if item["entity_type"] == kind),
        None,
    )
    assert seed, f"コード系の実体が無い（型: {sorted({i['entity_type'] for i in items})}）"
    entity_id = seed["id"]

    request = {
        "seed": {"kind": "ENTITY", "entity_ids": [entity_id]},
        "options": {
            "max_depth": 3,
            "hub_threshold": 200,
            "max_items": 50,
            "include_documents": False,
        },
    }
    status, accepted = call("POST", f"/projects/{PROJECT}/impacts", request)
    assert status in (200, 202), (status, accepted)
    wait_job(accepted["job_id"])
    _, first = call("GET", f"/impacts/{accepted['impact_id']}")
    report = first.get("report") or {}
    assert report.get("items"), report
    coverage = report["test_coverage"]
    assert coverage["covering_cases"] == [], coverage
    assert "tests_not_available" in report["degraded"], report["degraded"]

    item_id = report["items"][0]["item_id"]
    psql(
        "INSERT INTO test.test_plan (id, project_id, code, name, status) "
        f"VALUES ('{PLAN_ID}', '{PROJECT}', 'TP-ASSET', 'asset match probe', 'APPROVED') "
        "ON CONFLICT (id) DO NOTHING;"
        "INSERT INTO test.test_scenario (id, project_id, plan_id, code, title, kind) "
        f"VALUES ('{SCENARIO_ID}', '{PROJECT}', '{PLAN_ID}', 'TS-ASSET', 'asset match', 'NORMAL') "
        "ON CONFLICT (id) DO NOTHING;"
        "INSERT INTO test.test_case "
        "(id, project_id, scenario_id, code, title, status, covers_impacts) "
        f"VALUES ('{CASE_ID}', '{PROJECT}', '{SCENARIO_ID}', 'TC-ASSET', 'asset match', 'DRAFT', "
        f"ARRAY['{item_id}']::uuid[]) ON CONFLICT (id) DO NOTHING;"
    )
    try:
        status, accepted2 = call("POST", f"/projects/{PROJECT}/impacts", request)
        assert status in (200, 202), (status, accepted2)
        assert accepted2["impact_id"] == accepted["impact_id"], "seed から id が決まらない"
        wait_job(accepted2["job_id"])
        _, second = call("GET", f"/impacts/{accepted['impact_id']}")
        matched = second["report"]
        covering = matched["test_coverage"]["covering_cases"]
        assert covering, "既存テストが照合されない"
        assert covering[0]["case_id"] == CASE_ID, covering
        assert item_id in matched["test_coverage"]["covered_item_ids"]
        assert "tests_not_available" not in matched["degraded"], matched["degraded"]

        result = {
            "before": {"covering": 0, "degraded": report["degraded"]},
            "after": {
                "covering": len(covering),
                "covered_items": len(matched["test_coverage"]["covered_item_ids"]),
                "uncovered_items": len(matched["test_coverage"]["uncovered_item_ids"]),
                "degraded": matched["degraded"],
            },
        }
        out = ROOT / "data" / "phase6-eval" / "asset-match.json"
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8")
        print(json.dumps(result, ensure_ascii=False, indent=2))
        print(f"\nwritten: {out}")
        return 0
    finally:
        cleanup()


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as error:  # noqa: BLE001 - 受入スクリプトは失敗内容をそのまま出す
        print(f"FAILED: {error}", file=sys.stderr)
        raise
