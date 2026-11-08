#!/usr/bin/env python3
"""Phase 5 source-only 20-change golden-set evaluation."""

from __future__ import annotations

import http.cookiejar
import json
import os
import pathlib
import time
import urllib.error
import urllib.parse
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[1]
BASE_URL = os.environ.get("SYSTIVRA_BASE_URL", "http://127.0.0.1:18096")
LOGIN_ID = os.environ.get("SYSTIVRA_LOGIN_ID", "alice")
PASSWORD = os.environ.get("SYSTIVRA_PASSWORD", "SystivraDemo1!")
PROJECT_ID = os.environ.get("SYSTIVRA_PROJECT_ID")
MAX_ITEMS = int(os.environ.get("PHASE5_GOLDEN_MAX_ITEMS", "200"))
FIXTURE = ROOT / "tests/fixtures/impact/golden-phase5.json"
REPORT = ROOT / "data/phase5-golden/report.json"

cookies = http.cookiejar.CookieJar()
opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(cookies))


def call(method: str, path: str, body: dict | None = None):
    data = None
    headers = {}
    if body is not None:
        data = json.dumps(body, ensure_ascii=False).encode("utf-8")
        headers["content-type"] = "application/json"
    request = urllib.request.Request(
        f"{BASE_URL}/api/v1{path}", data=data, headers=headers, method=method
    )
    try:
        with opener.open(request, timeout=120) as response:
            payload = response.read()
            return json.loads(payload) if payload else None
    except urllib.error.HTTPError as error:
        detail = error.read().decode("utf-8", "replace")
        raise RuntimeError(f"{method} {path}: HTTP {error.code}: {detail}") from error


def wait_job(job_id: str) -> dict:
    terminal = {"SUCCEEDED", "SUCCEEDED_WITH_ERRORS", "FAILED", "CANCELLED", "TIMEOUT"}
    for _ in range(480):
        job = call("GET", f"/jobs/{job_id}")
        if job["status"] in terminal:
            if job["status"] not in {"SUCCEEDED", "SUCCEEDED_WITH_ERRORS"}:
                raise RuntimeError(f"job {job_id}: {job['status']} {job.get('errors')}")
            return job
        time.sleep(0.25)
    raise TimeoutError(f"job {job_id} did not finish")


def resolve_entity(project_id: str, expected: dict) -> dict:
    query = urllib.parse.urlencode(
        {"q": expected["natural_key"], "entity_type": expected["kind"], "limit": 20}
    )
    items = call("GET", f"/projects/{project_id}/graph/entities?{query}")["items"]
    match = next(
        (
            item
            for item in items
            if item["entity_type"] == expected["kind"]
            and (item.get("qualified_name") or item["name"]) == expected["natural_key"]
        ),
        None,
    )
    if match is None:
        raise AssertionError(f"golden entity missing: {expected}")
    return match


def item_key(item: dict) -> tuple[str, str]:
    return item["entity_kind"], item["natural_key"]


def main() -> int:
    fixture = json.loads(FIXTURE.read_text(encoding="utf-8"))
    login = call("POST", "/auth/login", {"login_id": LOGIN_ID, "password": PASSWORD})
    project_id = PROJECT_ID or next(
        (
            item["project_id"]
            for item in login["memberships"]
            if item.get("project_code") == fixture["project_code"]
        ),
        login["memberships"][0]["project_id"],
    )

    rows = []
    true_positive = 0
    expected_total = 0
    actual_total = 0
    evidence_failures = []
    for change in fixture["changes"]:
        seed = resolve_entity(project_id, change["seed"])
        accepted = call(
            "POST",
            f"/projects/{project_id}/impacts",
            {
                "seed": {"kind": "ENTITY", "entity_ids": [seed["id"]]},
                "options": {
                    "max_depth": 1,
                    "hub_threshold": 200,
                    "max_items": MAX_ITEMS,
                    "include_documents": False,
                },
            },
        )
        wait_job(accepted["job_id"])
        response = call("GET", f"/impacts/{accepted['impact_id']}")
        report = response["report"]
        if report is None:
            raise AssertionError(f"{change['id']}: report is missing")
        if report["approval_state"] != "DRAFT":
            raise AssertionError(f"{change['id']}: report is not DRAFT")
        if report["code_modified"] or report["tests_executed"]:
            raise AssertionError(f"{change['id']}: non-destructive invariant failed")

        expected = {
            (item["kind"], item["natural_key"]) for item in change["expected"]
        }
        actual = {item_key(item) for item in report["items"]}
        matched = expected & actual
        missing = sorted(expected - actual)
        extra = sorted(actual - expected)
        for item in report["items"]:
            if not (
                item.get("why")
                and item.get("path")
                and item.get("evidence")
                and any(
                    evidence.get("quote") and evidence.get("locator")
                    for evidence in item["evidence"]
                )
            ):
                evidence_failures.append(
                    {
                        "change_id": change["id"],
                        "natural_key": item["natural_key"],
                    }
                )

        true_positive += len(matched)
        expected_total += len(expected)
        actual_total += len(actual)
        rows.append(
            {
                "id": change["id"],
                "impact_id": accepted["impact_id"],
                "expected": len(expected),
                "actual": len(actual),
                "matched": len(matched),
                "missing": missing,
                "extra": extra,
                "rationale": change["rationale"],
            }
        )

    recall = true_positive / expected_total if expected_total else 0.0
    precision = true_positive / actual_total if actual_total else 0.0
    passed = (
        recall >= fixture["min_recall"]
        and precision >= fixture["min_precision"]
        and not evidence_failures
        and len(rows) == 20
    )
    summary = {
        "changes": len(rows),
        "true_positive": true_positive,
        "expected_total": expected_total,
        "actual_total": actual_total,
        "recall": round(recall, 6),
        "precision": round(precision, 6),
        "min_recall": fixture["min_recall"],
        "min_precision": fixture["min_precision"],
        "evidence_failures": evidence_failures,
        "passed": passed,
        "source_scope": "synthetic SAMPLE; source-only Facts; no OpenRewrite/javac/classpath",
        "rows": rows,
    }
    REPORT.parent.mkdir(parents=True, exist_ok=True)
    REPORT.write_text(json.dumps(summary, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(summary, ensure_ascii=False, indent=2))
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
