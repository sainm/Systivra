#!/usr/bin/env python3
"""Phase 5 evidence-bounded impact vertical-slice acceptance."""

from __future__ import annotations

import http.cookiejar
import json
import os
import pathlib
import time
import urllib.error
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[1]
BASE_URL = os.environ.get("SYSTIVRA_BASE_URL", "http://127.0.0.1:18080")
LOGIN_ID = os.environ.get("SYSTIVRA_LOGIN_ID", "alice")
PASSWORD = os.environ.get("SYSTIVRA_PASSWORD", "SystivraDemo1!")
PROJECT_ID = os.environ.get("SYSTIVRA_PROJECT_ID")
MAX_ITEMS = int(os.environ.get("PHASE5_MAX_ITEMS", "500"))

cookies = http.cookiejar.CookieJar()
opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(cookies))


def call(method: str, path: str, body: dict | None = None):
    data = None
    headers = {}
    if body is not None:
        data = json.dumps(body, ensure_ascii=False).encode("utf-8")
        headers["content-type"] = "application/json"
    request = urllib.request.Request(
        f"{BASE_URL}/api/v1{path}",
        data=data,
        headers=headers,
        method=method,
    )
    try:
        with opener.open(request, timeout=30) as response:
            payload = response.read()
            return json.loads(payload) if payload else None
    except urllib.error.HTTPError as error:
        detail = error.read().decode("utf-8", "replace")
        raise RuntimeError(f"{method} {path}: HTTP {error.code}: {detail}") from error


def wait_job(job_id: str) -> dict:
    terminal = {"SUCCEEDED", "SUCCEEDED_WITH_ERRORS", "FAILED", "CANCELLED", "TIMEOUT"}
    for _ in range(240):
        job = call("GET", f"/jobs/{job_id}")
        if job["status"] in terminal:
            if job["status"] not in {"SUCCEEDED", "SUCCEEDED_WITH_ERRORS"}:
                raise RuntimeError(f"job {job_id} ended as {job['status']}: {job.get('errors')}")
            return job
        time.sleep(0.25)
    raise TimeoutError(f"job {job_id} did not finish")


def run_impact(project_id: str, body: dict) -> tuple[dict, dict]:
    accepted = call("POST", f"/projects/{project_id}/impacts", body)
    wait_job(accepted["job_id"])
    response = call("GET", f"/impacts/{accepted['impact_id']}")
    if not response.get("report"):
        raise AssertionError(f"impact report missing: {response}")
    return accepted, response


def main() -> int:
    login = call(
        "POST",
        "/auth/login",
        {"login_id": LOGIN_ID, "password": PASSWORD},
    )
    project_id = PROJECT_ID or next(
        (
            item["project_id"]
            for item in login["memberships"]
            if item.get("project_code") == "PRJ-DEMO"
        ),
        login["memberships"][0]["project_id"],
    )
    entities = call(
        "GET",
        f"/projects/{project_id}/graph/entities?q=UserEditAction&limit=100",
    )["items"]
    action = next(
        (
            item
            for item in entities
            if item["entity_type"] == "Action"
            and "UserEditAction" in (item.get("qualified_name") or item["name"])
        ),
        None,
    )
    assert action, "UserEditAction graph entity is required"

    options = {
        "max_depth": 6,
        "hub_threshold": 200,
        "max_items": MAX_ITEMS,
        "include_documents": True,
    }
    entity_request = {
        "seed": {"kind": "ENTITY", "entity_ids": [action["id"]]},
        "options": options,
    }
    accepted1, entity_response = run_impact(project_id, entity_request)
    accepted2, entity_response_again = run_impact(project_id, entity_request)
    assert accepted1["impact_id"] == accepted2["impact_id"], "idempotency failed"
    assert entity_response["report"] == entity_response_again["report"], "result is not deterministic"

    entity_report = entity_response["report"]
    assert entity_report["approval_state"] == "DRAFT"
    assert entity_report["code_modified"] is False
    assert entity_report["tests_executed"] is False
    assert entity_report["items"], "entity traversal produced no evidence-backed items"
    assert all(item["why"] for item in entity_report["items"])
    assert all(item["evidence"] for item in entity_report["items"])
    entity_evidence = sum(len(item["evidence"]) for item in entity_report["items"])
    assert "tests_not_available" in entity_report["degraded"]

    revisions = call(
        "GET",
        f"/projects/{project_id}/code/revisions?limit=20",
    )["items"]
    by_id = {item["id"]: item for item in revisions}
    head = next(
        (
            item
            for item in revisions
            if item["file_count"] == 12
            and item.get("parent_id") in by_id
            and by_id[item["parent_id"]]["file_count"] == 12
        ),
        revisions[0],
    )
    base = by_id.get(head.get("parent_id"))
    assert base, "diff assessment needs a parent revision"
    diff_request = {
        "seed": {
            "kind": "DIFF",
            "base_revision_id": base["id"],
            "head_revision_id": head["id"],
            "paths": [],
        },
        "options": options,
    }
    _, diff_response = run_impact(project_id, diff_request)
    diff_report = diff_response["report"]
    diff_evidence = sum(len(item["evidence"]) for item in diff_report["items"])
    assert diff_report["seeds"], "diff seeds are empty"
    assert diff_report["items"], "diff traversal produced no evidence-backed items"
    assert all(item["evidence"] for item in diff_report["items"])
    assert diff_report["truncated"]["no_logical_change"] is False
    assert diff_report["changed_hunks"], "line-level diff hunks are missing"
    assert any(
        seed["path"] and seed["path"].endswith("UserService.java") and seed["line_start"]
        for seed in diff_report["seeds"]
    ), diff_report["seeds"]
    assert any(item["category"] == "DOCUMENT" for item in diff_report["items"]), (
        "document evidence bridge did not produce a candidate"
    )
    assert all(item["reason"] == "evidence_missing" for item in diff_report["unverified"])

    field_seed = next(
        (
            item
            for item in entities
            if item["entity_type"] == "Field"
            and item["qualified_name"].endswith("UserEditAction.userService")
        ),
        None,
    )
    assert field_seed, "field declaration graph node is required"
    field_patch = """\
diff --git a/src/main/java/jp/example/action/UserEditAction.java b/src/main/java/jp/example/action/UserEditAction.java
--- a/src/main/java/jp/example/action/UserEditAction.java
+++ b/src/main/java/jp/example/action/UserEditAction.java
@@ -6 +6 @@
-    private UserService userService;
+    private UserService userService; // changed field
"""
    field_request = {
        "seed": {
            "kind": "DIFF",
            "base_revision_id": None,
            "head_revision_id": head["id"],
            "paths": [],
            "diff_unified": field_patch,
        },
        "options": options,
    }
    _, field_response = run_impact(project_id, field_request)
    field_report = field_response["report"]
    assert any(
        seed["kind"] == "Field"
        and seed["natural_key"].endswith("UserEditAction.userService")
        and seed["line_start"] == 6
        for seed in field_report["seeds"]
    ), field_report["seeds"]
    field_closure = next(
        (
            item
            for item in field_report["items"]
            if item["category"] == "CODE"
            and "UserEditAction.update(" in item["natural_key"]
        ),
        None,
    )
    assert field_closure, "field read/write closure did not reach UserEditAction.update"
    assert any(
        edge["relation"] == "READS" for edge in field_closure["path_edges"]
    ), field_closure["path_edges"]

    report = {
        "project_id": project_id,
        "entity_impact_id": accepted1["impact_id"],
        "entity_items": entity_report["stats"]["total"],
        "entity_evidence": entity_evidence,
        "entity_degraded": entity_report["degraded"],
        "diff_impact_id": diff_response["impact_id"],
        "diff_items": diff_report["stats"]["total"],
        "diff_evidence": diff_evidence,
        "diff_seeds": diff_report["seeds"],
        "diff_unverified": len(diff_report["unverified"]),
        "diff_changed_hunks": diff_report["changed_hunks"],
        "field_impact_id": field_response["impact_id"],
        "field_seeds": field_report["seeds"],
        "field_closure_items": field_report["stats"]["total"],
        "field_closure_method": field_closure["natural_key"],
        "diff_truncated": diff_report["truncated"],
        "risk": diff_report["risk"],
        "document_items": diff_report["stats"]["documents"],
        "deterministic": True,
        "code_modified": False,
        "tests_executed": False,
    }
    out_dir = ROOT / "data" / "phase5-eval"
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "report.json").write_text(
        json.dumps(report, ensure_ascii=False, indent=2),
        encoding="utf-8",
    )
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
