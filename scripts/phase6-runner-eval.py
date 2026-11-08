#!/usr/bin/env python3
"""Phase 6 Playwright Runner の受入（100 ケース / 4 並列 / 成果物 / HAR マスキング）。

前提: 001〜015 適用済み DB、API（既定 18086）、worker が起動済み。
SAMPLE 静的サイトを自動で立て、Runner を 4 並列で走らせ、結果を API へ戻す。

  PGHOST=localhost PGUSER=<admin> PGDATABASE=systivra_phase6_verify \
  SYSTIVRA_BASE_URL=http://127.0.0.1:18086 python3 scripts/phase6-runner-eval.py
"""

from __future__ import annotations

import http.cookiejar
import json
import os
import pathlib
import shutil
import subprocess
import sys
import time
import urllib.error
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[1]
BASE_URL = os.environ.get("SYSTIVRA_BASE_URL", "http://127.0.0.1:18086")
WEB_PORT = int(os.environ.get("SAMPLE_PORT", "18099"))
WEB_BASE = f"http://127.0.0.1:{WEB_PORT}"
DB = os.environ.get("PGDATABASE", "systivra")
LOGIN_ID = os.environ.get("SYSTIVRA_LOGIN_ID", "alice")
PASSWORD = os.environ.get("SYSTIVRA_PASSWORD", "SystivraDemo1!")
TARGET_CASES = int(os.environ.get("TARGET_CASES", "100"))
PARALLEL = int(os.environ.get("PARALLEL", "4"))
PLAN_COUNT = 6

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


def prepare(pid: str) -> int:
    call("POST", f"/projects/{pid}/test/rules:extract", {"sources": RULE_SOURCES})
    for index in range(PLAN_COUNT):
        impact_id = f"d{index:03d}0000-0000-4000-8000-000000000000"
        report = {
            "items": [
                {"item_id": f"e{index:03d}0000-0000-4000-8000-000000000001",
                 "label": "UserService.save(UserDto)", "category": "CODE", "why": "変更 Method"},
                {"item_id": f"e{index:03d}0000-0000-4000-8000-000000000002",
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
            {"impact_id": impact_id, "name": f"Runner 用計画 {index}", "max_drafts": 100},
        )

    overview = call("GET", f"/projects/{pid}/test/overview")
    drafts = [case for case in overview["cases"] if case["status"] == "DRAFT"]
    assert len(drafts) >= TARGET_CASES, f"生成されたケースが足りない: {len(drafts)}"
    for case in drafts:
        if case["validation_errors"]:
            raise AssertionError(f"不正 DSL: {case['code']} {case['validation_errors']}")
        call("POST", f"/projects/{pid}/test/cases/{case['id']}/approve")
    approved = [
        case for case in call("GET", f"/projects/{pid}/test/overview")["cases"]
        if case["status"] == "APPROVED"
    ]
    assert len(approved) >= TARGET_CASES
    return len(approved)


def inject_stale_expectation(pid: str) -> str:
    """期待値が古いケースを 1 件作り、ASSERTION 失敗と 4 点成果物を実測する。"""
    overview = call("GET", f"/projects/{pid}/test/overview")
    target = next(
        (
            case
            for case in overview["cases"]
            if case["status"] == "APPROVED"
            and any(step["action"] == "field_error" for step in case["dsl"]["steps"])
        ),
        None,
    )
    assert target, "field_error を持つケースが必要"
    dsl = json.loads(json.dumps(target["dsl"]))
    for step in dsl["steps"]:
        if step["action"] == "field_error":
            step["value"] = "期待はずれ-STALE-EXPECTATION"
    psql(
        "UPDATE test.test_case SET dsl = "
        f"{dollar(dsl)} WHERE id = '{target['id']}';"
    )
    return target["id"]


def main() -> int:
    reset()
    # 前回の成果物を混ぜない。report.json / acceptance.json は残す。
    artifact_root = ROOT / "data" / "phase6-runner"
    artifact_root.mkdir(parents=True, exist_ok=True)
    for child in artifact_root.iterdir():
        if child.is_dir():
            shutil.rmtree(child)
    login = call("POST", "/auth/login", {"login_id": LOGIN_ID, "password": PASSWORD})
    pid = login["memberships"][0]["project_id"]
    approved = prepare(pid)
    stale_case = inject_stale_expectation(pid)

    server = subprocess.Popen(
        [
            sys.executable, "-m", "http.server", str(WEB_PORT),
            "--bind", "127.0.0.1", "--directory", str(ROOT / "samples" / "sample-web"),
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    try:
        for _ in range(50):
            try:
                urllib.request.urlopen(f"{WEB_BASE}/", timeout=1).close()
                break
            except Exception:  # noqa: BLE001 - 起動待ち
                time.sleep(0.1)
        else:
            raise RuntimeError("SAMPLE サイトが起動しない")

        env = dict(os.environ)
        env.update(
            {
                "SYSTIVRA_BASE_URL": BASE_URL,
                "WEB_BASE": WEB_BASE,
                "PROJECT_ID": pid,
                "PARALLEL": str(PARALLEL),
                "LIMIT": str(TARGET_CASES),
            }
        )
        run = subprocess.run(
            ["node", str(ROOT / "workers" / "runner" / "runner.mjs")],
            cwd=str(ROOT),
            env=env,
            capture_output=True,
            text=True,
            timeout=1800,
        )
        if run.returncode != 0:
            raise RuntimeError(f"runner failed:\n{run.stdout[-3000:]}\n{run.stderr[-3000:]}")
    finally:
        server.terminate()
        try:
            server.wait(timeout=5)
        except subprocess.TimeoutExpired:
            server.kill()

    report = json.loads((ROOT / "data" / "phase6-runner" / "report.json").read_text(encoding="utf-8"))
    overview = call("GET", f"/projects/{pid}/test/overview")
    executions = [e for e in overview["executions"] if e["test_run_id"] == report["run_id"]]
    playwright = [e for e in executions if e["execution_source"] == "PLAYWRIGHT"]

    assert report["case_count"] >= TARGET_CASES, report
    assert report["parallel"] == PARALLEL
    assert report["wall_seconds"] < 3600, "100 ケース / 4 並列が 60 分を超えた"
    assert report["artifacts"]["pass_with_trace"] == 0, "成功時に trace が残っている"
    assert report["artifacts"]["har_leaks"] == 0, "HAR に秘密が残っている"
    assert report["artifacts"]["failed_with_four"] >= 1, "失敗時の 4 点成果物が無い"
    assert report["failure_kinds"].get("ASSERTION", 0) >= 1, "古い期待値が検出されていない"
    assert len(playwright) >= TARGET_CASES, f"API に戻った実行が足りない: {len(playwright)}"
    assert any(e["status"] == "FAILED" for e in playwright)
    assert any(d["recurrence"] >= 1 for d in overview["defects"])

    summary = {
        "approved_cases": approved,
        "stale_case": stale_case,
        "runner": report,
        "api": {
            "playwright_executions": len(playwright),
            "failed": sum(1 for e in playwright if e["status"] == "FAILED"),
            "defect_candidates": len(overview["defects"]),
        },
    }
    out = ROOT / "data" / "phase6-runner" / "acceptance.json"
    out.write_text(json.dumps(summary, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(summary, ensure_ascii=False, indent=2))
    print(f"\nwritten: {out}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as error:  # noqa: BLE001
        print(f"FAILED: {error}", file=sys.stderr)
        raise
