#!/usr/bin/env python3
"""Phase 6: SAMPLE 24 画面 / 300 ルールの生成と受入実測（R-01 / R-17）。

前提: 001〜027 適用済み DB（既定 systivra_phase6_verify）、API（既定 18088）、
worker が起動済み。

  PGHOST=localhost PGUSER=sainm PGDATABASE=systivra_phase6_verify \
  SYSTIVRA_BASE_URL=http://127.0.0.1:18088 python3 scripts/phase6-sample-eval.py

実測:
  - 24 画面の静的 SAMPLE と 300 ルールを生成（fixture は samples/ に書き出す）
  - 24 計画 × 50 草稿 = 1,200 組のデータ派生（受入 900 組以上）
  - 承認 120 ケース（各画面 5 = ルール 4 + 画面 1）を 100 ケース / 4 並列と
    120 ケース / 4 並列で実行し、初回合格率 60% 以上と 60 分以内を実測
  - 正解集（全ケース PASSED 期待）との一致を検査
  - 被覆: 画面 24 / 24、ルールは DRAFT 上限のため一部（分子分母を残す）
"""

from __future__ import annotations

import importlib.util
import json
import os
import pathlib
import shutil
import sys
import uuid

ROOT = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location(
    "p6failure", ROOT / "scripts" / "phase6-failure-eval.py"
)
p6 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(p6)

SAMPLE_ROOT = ROOT / "samples" / "sample-web"
GOLDEN_DIR = ROOT / "samples" / "phase6"
REPORT_DIR = ROOT / "data" / "phase6-sample"
SCREEN_COUNT = 24
RULE_TARGET = 300
MAX_DRAFTS = 50
APPROVE_PER_SCREEN = 5
TIMING_LIMIT = 100

SCREEN_NAMES = [
    "ユーザ編集", "ユーザ一覧", "部署編集", "部署一覧",
    "権限設定", "パスワード変更", "ログイン履歴", "マスタ編集",
    "商品編集", "商品一覧", "在庫照会", "受注登録",
    "受注一覧", "請求書発行", "入金照会", "顧客編集",
    "顧客一覧", "問い合わせ登録", "問い合わせ一覧", "帳票出力",
    "バッチ監視", "メール配信", "ファイル取込", "監査ログ照会",
]

RULE_TEXTS = [
    ("SPEC", "必須項目"),
    ("SPEC", "全角20文字以内"),
    ("SPEC", "min=1 max=99"),
    ("SPEC", 'pattern="^[ぁ-ん]+$" 正規表現'),
    ("SPEC", "開始日 < 終了日"),
    ("CODE", "if (s.length() > 20) errors.add(...)"),
]

PAGE_TEMPLATE = """<!doctype html>
<html lang="ja">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>__TITLE__ - SAMPLE</title>
  <style>
    body { font-family: system-ui, sans-serif; margin: 2rem; line-height: 1.6; }
    .sample-mark { display: inline-block; border: 1px solid #888; padding: 0 6px; font-size: 12px; }
    label { display: block; margin: 12px 0 4px; font-weight: 600; }
    input { padding: 6px; width: 28rem; max-width: 90%; }
    button { padding: 6px 18px; margin-top: 12px; }
    .field-error { color: #b00020; margin-top: 12px; }
    .ok { color: #0a7a3b; margin-top: 12px; }
    [hidden] { display: none !important; }
  </style>
</head>
<body>
  <span class="sample-mark">SAMPLE</span>
  <h1>__TITLE__</h1>

  <form id="form" novalidate>
    <label for="value" id="field-label">項目</label>
    <input id="value" name="value" data-testid="value-input" autocomplete="off" />
    <div>
      <button type="submit" data-testid="submit">登録</button>
    </div>
  </form>

  <div id="field-error" class="field-error" role="alert" hidden>入力値を確認してください</div>
  <div id="ok" class="ok" hidden>登録しました</div>

  <section>
    <h2>影響項目</h2>
    <div id="impact-label"></div>
  </section>

  <script>
__SCRIPT__
  </script>
</body>
</html>
"""

SHARED_JS = """const params = new URLSearchParams(location.search);
const field = params.get("field") || "項目";
document.getElementById("field-label").textContent = field;
document.getElementById("value").setAttribute("aria-label", field);
const impactLabel = params.get("label");
if (impactLabel) {
  document.getElementById("impact-label").textContent = impactLabel;
}

function isInvalid(value) {
  if (value.trim() === "") return true;
  const chars = [...value];
  if (chars.length > 20) return true;
  if (/[A-Za-z]/.test(value)) return true;
  for (const ch of chars) {
    const cp = ch.codePointAt(0);
    if (cp >= 0xe000 && cp <= 0xf8ff) return true;
    if (cp > 0xffff) return true;
    if (cp === 0x3000) return true;
    if (cp === 0x3099 || cp === 0x309a) return true;
  }
  if (/^-?\\d+$/.test(value)) {
    const n = Number(value);
    if (n < 1 || n > 99) return true;
  }
  if (value.includes("|")) {
    const [start, end] = value.split("|");
    if (start >= end) return true;
  }
  return false;
}

document.getElementById("form").addEventListener("submit", (event) => {
  event.preventDefault();
  const value = document.getElementById("value").value;
  const bad = isInvalid(value);
  document.getElementById("field-error").hidden = !bad;
  document.getElementById("ok").hidden = bad;
});
"""


def screen_id(index: int) -> str:
    return f"S{index:02}"


def generate_screens() -> list[dict]:
    screens = []
    for index in range(1, SCREEN_COUNT + 1):
        sid = screen_id(index)
        title = f"画面 {sid} {SCREEN_NAMES[index - 1]}"
        directory = SAMPLE_ROOT / "sample" / "s" / f"{index:02}" / "edit"
        directory.mkdir(parents=True, exist_ok=True)
        (directory / "index.html").write_text(
            PAGE_TEMPLATE.replace("__TITLE__", title).replace("__SCRIPT__", SHARED_JS),
            encoding="utf-8",
        )
        screens.append({"id": sid, "title": title, "base_path": f"/sample/s/{index:02}/edit"})
    return screens


def rule_sources() -> list[dict]:
    sources = []
    for index in range(1, SCREEN_COUNT + 1):
        for field_index in range(12):
            kind, text = RULE_TEXTS[field_index % len(RULE_TEXTS)]
            sources.append({
                "kind": kind,
                "field_ref": f"s{index:02}.f{field_index + 1:02}",
                "text": text,
                "evidence_ids": [],
            })
    for field_index in range(RULE_TARGET - SCREEN_COUNT * 12):
        sources.append({
            "kind": "SPEC",
            "field_ref": f"shared.f{field_index + 1:02}",
            "text": "必須項目",
            "evidence_ids": [],
        })
    assert len(sources) == RULE_TARGET
    return sources


def item_uuid(sid: str, kind: str) -> str:
    return str(uuid.uuid5(uuid.NAMESPACE_URL, f"systivra:sample:{sid}:{kind}"))


def prepare(pid: str, screens: list[dict]) -> None:
    for screen in screens:
        impact_id = item_uuid(screen["id"], "impact")
        report = {
            "items": [
                {"item_id": item_uuid(screen["id"], "screen"),
                 "label": screen["title"], "category": "SCREENS", "why": "変更画面"},
                {"item_id": item_uuid(screen["id"], "code"),
                 "label": f"{screen['id']}Service.save", "category": "CODE", "why": "変更 Method"},
            ],
            "degraded": [],
            "approval_state": "DRAFT",
        }
        p6.psql(
            "INSERT INTO impact.assessment_draft (id, project_id, input, status, report) "
            f"VALUES ('{impact_id}', '{pid}', '{{}}', 'SUCCEEDED', {p6.dollar(report)}) "
            "ON CONFLICT (id) DO UPDATE SET report = EXCLUDED.report;"
        )
        p6.call(
            "POST",
            f"/projects/{pid}/test/plans:derive",
            {
                "impact_id": impact_id,
                "name": f"{screen['id']} 画面確認",
                "max_drafts": 10,
                "base_path": screen["base_path"],
            },
        )
    response = p6.call("POST", f"/projects/{pid}/test/rules:extract", {"sources": rule_sources()})
    rules = p6.call("GET", f"/projects/{pid}/test/overview")["rules"]
    assert len(rules) == RULE_TARGET, (len(rules), response)
    assert response.get("unparsed", 0) == 0, response
    for screen in screens:
        p6.call(
            "POST",
            f"/projects/{pid}/test/plans:derive",
            {
                "impact_id": item_uuid(screen["id"], "impact"),
                "name": f"{screen['id']} 境界",
                "max_drafts": MAX_DRAFTS,
                "base_path": screen["base_path"],
            },
        )


def approve(pid: str) -> tuple[list[dict], dict]:
    overview = p6.call("GET", f"/projects/{pid}/test/overview")
    drafts = [case for case in overview["cases"] if case["status"] == "DRAFT"]
    assert len(drafts) == SCREEN_COUNT * 2 + SCREEN_COUNT * MAX_DRAFTS, len(drafts)
    screen_cases = [case for case in drafts if case["covers_screens"]]
    assert len(screen_cases) == SCREEN_COUNT, len(screen_cases)
    by_plan: dict[str, list[dict]] = {}
    for case in drafts:
        if case["covers_rules"]:
            prefix = case["code"].split("-TC-")[0]
            by_plan.setdefault(prefix, []).append(case)
    assert len(by_plan) == SCREEN_COUNT, len(by_plan)
    approved: list[dict] = []
    golden: dict[str, dict] = {}
    for prefix, cases in by_plan.items():
        rule_cases = cases[: APPROVE_PER_SCREEN - 1]
        assert len(rule_cases) == APPROVE_PER_SCREEN - 1, prefix
        for case in rule_cases:
            if case["validation_errors"]:
                raise AssertionError(f"不正 DSL: {case['code']} {case['validation_errors']}")
            p6.call("POST", f"/projects/{pid}/test/cases/{case['id']}/approve")
            approved.append(case)
            golden[case["code"]] = {
                "expected": "PASSED",
                "plan": prefix,
                "covers_rules": case["covers_rules"],
                "covers_screens": case["covers_screens"],
            }
    for case in screen_cases:
        if case["validation_errors"]:
            raise AssertionError(f"不正 DSL: {case['code']} {case['validation_errors']}")
        p6.call("POST", f"/projects/{pid}/test/cases/{case['id']}/approve")
        approved.append(case)
        golden[case["code"]] = {
            "expected": "PASSED",
            "plan": case["code"].split("-TC-")[0],
            "covers_rules": case["covers_rules"],
            "covers_screens": case["covers_screens"],
        }
    return approved, golden


def final_statuses(pid: str, run_id: str) -> dict[str, str]:
    overview = p6.call("GET", f"/projects/{pid}/test/overview")
    cases = {c["id"]: c["code"] for c in overview["cases"]}
    latest: dict[str, tuple[int, str]] = {}
    for execution in overview["executions"]:
        if execution["test_run_id"] != run_id:
            continue
        code = cases.get(execution["test_case_id"])
        if code is None:
            continue
        attempt, status = latest.get(code, (0, "PENDING"))
        if execution["attempt"] >= attempt:
            latest[code] = (execution["attempt"], execution["status"])
    return {code: status for code, (_, status) in latest.items()}


def main() -> int:
    p6.reset()
    p6.psql("DELETE FROM core.audit_log WHERE action LIKE 'test.%';")
    for path in (REPORT_DIR,):
        if path.exists():
            shutil.rmtree(path)
    REPORT_DIR.mkdir(parents=True)
    GOLDEN_DIR.mkdir(parents=True, exist_ok=True)
    artifact_root = p6.ARTIFACT_ROOT
    artifact_root.mkdir(parents=True, exist_ok=True)
    for child in artifact_root.iterdir():
        if child.is_dir():
            shutil.rmtree(child)

    screens = generate_screens()
    login = p6.call("POST", "/auth/login", {"login_id": p6.LOGIN_ID, "password": p6.PASSWORD})
    pid = login["memberships"][0]["project_id"]
    prepare(pid, screens)
    approved, golden = approve(pid)
    (GOLDEN_DIR / "golden.json").write_text(
        json.dumps(golden, ensure_ascii=False, indent=2), encoding="utf-8"
    )

    overview = p6.call("GET", f"/projects/{pid}/test/overview")
    data_pairs = len(overview["data"])
    assert data_pairs >= 900, data_pairs

    sample = p6.start_server(
        p6.SampleHandler, int(os.environ.get("SAMPLE_PORT", "18099")), SAMPLE_ROOT
    )
    p6.wait_http(f"{p6.WEB_BASE}/")
    try:
        code, timing = p6.run_runner({
            "LIMIT": str(TIMING_LIMIT),
            "PARALLEL": "4",
            "ENVIRONMENT_ID": "env-24screens-timing",
        })
        assert code == 0, timing
        assert timing["executed_case_count"] == TIMING_LIMIT, timing
        assert timing["wall_seconds"] < 3600, timing

        code, report = p6.run_runner({
            "LIMIT": "0",
            "PARALLEL": "4",
            "ENVIRONMENT_ID": "env-24screens-full",
        })
        assert code == 0, report
        assert report["executed_case_count"] == len(approved), report
        assert report["wall_seconds"] < 3600, report

        statuses = final_statuses(pid, report["run_id"])
        mismatches = [
            {"code": case_code, "expected": golden[case_code]["expected"], "actual": status}
            for case_code, status in statuses.items()
            if golden.get(case_code, {}).get("expected") != status
        ]
        matched = len(statuses) - len(mismatches)
        first_pass_rate = matched / len(golden)
        assert len(statuses) == len(golden), (len(statuses), len(golden))
        assert first_pass_rate >= 0.6, mismatches

        overview = p6.call("GET", f"/projects/{pid}/test/overview")
        coverage = overview["coverage"]["detail"]
        assert coverage["screen"]["num"] == SCREEN_COUNT, coverage["screen"]
        assert coverage["screen"]["den"] == SCREEN_COUNT, coverage["screen"]

        summary = {
            "screens": SCREEN_COUNT,
            "rules": RULE_TARGET,
            "data_pairs": data_pairs,
            "approved_cases": len(approved),
            "first_attempt_pass_rate": report["first_attempt_pass_rate"],
            "golden_match_rate": first_pass_rate,
            "golden_mismatches": mismatches,
            "timing_run": {
                "limit": TIMING_LIMIT,
                "executed": timing["executed_case_count"],
                "parallel": timing["parallel"],
                "wall_seconds": timing["wall_seconds"],
            },
            "full_run": {
                "executed": report["executed_case_count"],
                "parallel": report["parallel"],
                "wall_seconds": report["wall_seconds"],
                "failure_kinds": report["failure_kinds"],
            },
            "coverage": {
                "screen": coverage["screen"],
                "rule": coverage["rule"],
                "impact": coverage["impact"],
                "regression": coverage["regression"],
            },
        }
        out = REPORT_DIR / "report.json"
        out.write_text(json.dumps(summary, ensure_ascii=False, indent=2), encoding="utf-8")
        print(json.dumps(summary, ensure_ascii=False, indent=2))
        print(f"\nwritten: {out}")
    finally:
        sample.shutdown()
        sample.server_close()
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as error:  # noqa: BLE001
        print(f"FAILED: {error}", file=sys.stderr)
        raise