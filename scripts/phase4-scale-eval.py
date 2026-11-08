#!/usr/bin/env python3
"""Phase 4 大規模 SAMPLE（既定 13,000 Java）の解析・増分を実測する。

API と worker が起動していること。DIR リポジトリとして登録し、
同期 → 解析 → 1 ファイル変更 → 再解析の時間と件数を記録する。
"""

from __future__ import annotations

import http.cookiejar
import json
import os
import pathlib
import sys
import time
import urllib.error
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[1]
BASE_URL = os.environ.get("SYSTIVRA_BASE_URL", "http://127.0.0.1:18093")
LOGIN_ID = os.environ.get("SYSTIVRA_LOGIN_ID", "alice")
PASSWORD = os.environ.get("SYSTIVRA_PASSWORD", "SystivraDemo1!")
PROJECT_ID = os.environ.get("SYSTIVRA_PROJECT_ID")
DATA_DIR = pathlib.Path(os.environ.get("SYSTIVRA_DATA_DIR", ROOT / "data"))
FILES = int(os.environ.get("PHASE4_SCALE_FILES", "13000"))
TAG = os.environ.get("PHASE4_SCALE_TAG", f"scale-{int(time.time())}")
REPO = DATA_DIR / "repositories" / f"phase4-{TAG}"
REPORT = DATA_DIR / "phase4-scale" / "report.json"

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


def wait_job(job_id: str) -> tuple[dict, float]:
    terminal = {"SUCCEEDED", "SUCCEEDED_WITH_ERRORS", "FAILED", "CANCELLED", "TIMEOUT"}
    started = time.monotonic()
    for _ in range(1200):
        job = call("GET", f"/jobs/{job_id}")
        if job["status"] in terminal:
            if job["status"] not in {"SUCCEEDED", "SUCCEEDED_WITH_ERRORS"}:
                raise RuntimeError(f"job {job_id}: {job['status']} {job.get('errors')}")
            return job, time.monotonic() - started
        time.sleep(0.5)
    raise TimeoutError(f"job {job_id} did not finish")


def java_source(index: int, changed: bool = False) -> str:
    suffix = " // changed" if changed else ""
    methods = []
    for method in range(5):
        call_target = f"m{(method + 1) % 5}"
        methods.append(
            f"    public int m{method}(int value) {{\n"
            f"        return value + {method};{suffix}\n"
            f"    }}\n"
        )
    methods.append(
        "    public int entry(int value) {\n"
        "        return m0(value);\n"
        "    }\n"
    )
    return (
        "package com.example.scale;\n\n"
        f"public class C{index} {{\n"
        + "\n".join(methods)
        + "}\n"
    )


def main() -> int:
    REPORT.parent.mkdir(parents=True, exist_ok=True)
    login = call("POST", "/auth/login", {"login_id": LOGIN_ID, "password": PASSWORD})
    project_id = PROJECT_ID or next(
        (
            item["project_id"]
            for item in login["memberships"]
            if item.get("project_code") == "PRJ-DEMO"
        ),
        login["memberships"][0]["project_id"],
    )

    started = time.monotonic()
    for index in range(FILES):
        directory = REPO / "src/main/java/com/example/scale" / f"p{index // 500:03d}"
        directory.mkdir(parents=True, exist_ok=True)
        (directory / f"C{index}.java").write_text(java_source(index), encoding="utf-8")
    generate_seconds = time.monotonic() - started

    repository = call(
        "POST",
        f"/projects/{project_id}/code/repositories",
        {"kind": "DIR", "path": str(REPO.relative_to(DATA_DIR)), "default_branch": None},
    )
    sync1 = call(
        "POST",
        f"/projects/{project_id}/code/revisions:sync",
        {"repository_id": repository["id"], "revision": "scale-001"},
    )
    _, sync_seconds = wait_job(sync1["job_id"])
    revisions = call(
        "GET",
        f"/projects/{project_id}/code/revisions?repository_id={repository['id']}&limit=10",
    )["items"]
    file_count = int(revisions[0]["file_count"])
    analysis1 = call(
        "POST",
        f"/projects/{project_id}/code/analysis-runs",
        {"revision_id": revisions[0]["id"]},
    )
    _, analysis_seconds = wait_job(analysis1["job_id"])
    run1 = call("GET", f"/code/analysis-runs/{analysis1['run_id']}")

    changed = REPO / "src/main/java/com/example/scale/p000/C0.java"
    changed.write_text(java_source(0, changed=True), encoding="utf-8")
    sync2 = call(
        "POST",
        f"/projects/{project_id}/code/revisions:sync",
        {"repository_id": repository["id"], "revision": "scale-002"},
    )
    _, sync2_seconds = wait_job(sync2["job_id"])
    revisions2 = call(
        "GET",
        f"/projects/{project_id}/code/revisions?repository_id={repository['id']}&limit=10",
    )["items"]
    analysis2 = call(
        "POST",
        f"/projects/{project_id}/code/analysis-runs",
        {"revision_id": revisions2[0]["id"]},
    )
    _, analysis2_seconds = wait_job(analysis2["job_id"])
    run2 = call("GET", f"/code/analysis-runs/{analysis2['run_id']}")
    analyzed = int(run2["stats"]["files_analyzed"])

    report = {
        "tag": TAG,
        "files": FILES,
        "file_count": file_count,
        "generate_seconds": round(generate_seconds, 2),
        "sync_seconds": round(sync_seconds, 2),
        "analysis_seconds": round(analysis_seconds, 2),
        "analysis_files_per_second": round(FILES / analysis_seconds, 2),
        "symbols": run1["symbols"],
        "facts": run1["facts"],
        "unresolved": run1["unresolved"],
        "incremental_sync_seconds": round(sync2_seconds, 2),
        "incremental_analysis_seconds": round(analysis2_seconds, 2),
        "incremental_files_analyzed": analyzed,
        "incremental_ratio": round(analyzed / FILES, 6),
        "status": run1["status"],
    }
    REPORT.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False, indent=2))
    ok = (
        run1["status"] == "SUCCEEDED"
        and file_count == FILES
        and run1["symbols"] >= FILES
        and run1["facts"] > 0
        and analyzed == 1
    )
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
