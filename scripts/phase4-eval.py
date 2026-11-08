#!/usr/bin/env python3
"""Phase 4 API acceptance against a running API + worker.

Prerequisites:
  ./scripts/seed-phase4.sh
  app / worker / PostgreSQL are running and alice can log in.
"""

from __future__ import annotations

import http.cookiejar
import json
import os
import pathlib
import shutil
import sys
import time
import unicodedata
import urllib.error
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[1]
BASE_URL = os.environ.get("SYSTIVRA_BASE_URL", "http://127.0.0.1:18080")
TAG = os.environ.get("PHASE4_TAG", f"phase4-{int(time.time())}")
LOGIN_ID = os.environ.get("SYSTIVRA_LOGIN_ID", "alice")
PASSWORD = os.environ.get("SYSTIVRA_PASSWORD", "SystivraDemo1!")
PROJECT_ID = os.environ.get("SYSTIVRA_PROJECT_ID")
SOURCE_REPO = ROOT / "data" / "repositories" / "phase4-sample"
REPO = f"repositories/phase4-eval-{TAG}"
REPO_DIR = ROOT / "data" / REPO

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


def call_bytes(path: str) -> bytes:
    request = urllib.request.Request(f"{BASE_URL}/api/v1{path}")
    try:
        with opener.open(request, timeout=30) as response:
            return response.read()
    except urllib.error.HTTPError as error:
        detail = error.read().decode("utf-8", "replace")
        raise RuntimeError(f"GET {path}: HTTP {error.code}: {detail}") from error


def wait_job(job_id: str) -> dict:
    terminal = {"SUCCEEDED", "SUCCEEDED_WITH_ERRORS", "FAILED", "CANCELLED", "TIMEOUT"}
    for _ in range(180):
        job = call("GET", f"/jobs/{job_id}")
        if job["status"] in terminal:
            if job["status"] not in {"SUCCEEDED", "SUCCEEDED_WITH_ERRORS"}:
                raise RuntimeError(f"job {job_id} ended as {job['status']}: {job.get('errors')}")
            return job
        time.sleep(0.5)
    raise TimeoutError(f"job {job_id} did not finish")


def append_one_file_change() -> None:
    path = REPO_DIR / "src/main/java/jp/example/service/UserService.java"
    marker = f"// phase4 incremental change {TAG}"
    text = path.read_text(encoding="utf-8")
    if marker not in text:
        method = "    public void save(UserDto user) {"
        if method not in text:
            raise RuntimeError("incremental fixture method was not found")
        path.write_text(
            text.replace(method, f"{method}\n        {marker}", 1),
            encoding="utf-8",
        )


def main() -> int:
    if not SOURCE_REPO.is_dir():
        raise RuntimeError("phase4 source fixture is missing; run ./scripts/seed-phase4.sh")
    shutil.copytree(SOURCE_REPO, REPO_DIR)
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

    repository = call(
        "POST",
        f"/projects/{project_id}/code/repositories",
        {"kind": "DIR", "path": REPO, "default_branch": None},
    )
    repository_id = repository["id"]

    sync1 = call(
        "POST",
        f"/projects/{project_id}/code/revisions:sync",
        {
            "repository_id": repository_id,
            "revision": f"{TAG}-r1",
            "committed_at": None,
        },
    )
    wait_job(sync1["job_id"])
    revisions1 = call(
        "GET",
        f"/projects/{project_id}/code/revisions?repository_id={repository_id}&limit=20",
    )["items"]
    revision1 = next(item for item in revisions1 if item["revision"] == f"{TAG}-r1")

    analysis1 = call(
        "POST",
        f"/projects/{project_id}/code/analysis-runs",
        {"revision_id": revision1["id"]},
    )
    wait_job(analysis1["job_id"])
    run1 = call("GET", f"/code/analysis-runs/{analysis1['run_id']}")
    symbols1 = call(
        "GET",
        f"/projects/{project_id}/code/symbols?revision_id={revision1['id']}&limit=200",
    )["items"]
    facts1 = call(
        "GET",
        f"/projects/{project_id}/code/facts?revision_id={revision1['id']}&limit=200",
    )["items"]
    files1 = call(
        "GET",
        f"/projects/{project_id}/code/revisions/{revision1['id']}/files",
    )["items"]

    resolved_calls = [
        fact
        for fact in facts1
        if fact["relation"] == "CALLS" and fact["resolution"] == "RESOLVED"
    ]
    reflection_calls = [
        fact
        for fact in facts1
        if fact["relation"] == "CALLS"
        and fact["subject_key"].startswith("jp.example.reflection.ReflectiveGateway")
    ]
    proxy_calls = [
        fact for fact in reflection_calls if "newProxyInstance" in fact["object_key"]
    ]
    assert reflection_calls, "reflection fixture was not detected"
    assert all(fact["resolution"] != "RESOLVED" for fact in reflection_calls)
    assert len(proxy_calls) >= 15, "dynamic proxy fixture was not detected"
    assert any(fact["relation"] == "HANDLES" for fact in facts1)
    assert any(fact["relation"] == "QUERIES" for fact in facts1)
    assert any(
        symbol["simple_name"] == "LegacyNameService" for symbol in symbols1
    ), "Shift_JIS fixture did not produce a class symbol"
    assert run1["classpath_complete"] is False
    assert run1["degraded_reasons"], "degradation reasons must be visible"

    evidence_count = 0
    byte_evidence_verified = 0
    file_by_path = {item["path"]: item for item in files1}
    raw_by_file: dict[str, bytes] = {}
    for fact in facts1:
        assert fact["evidence"], "every fact needs evidence"
        for evidence in fact["evidence"]:
            locator = evidence["locator"]
            assert locator["path"] and evidence["quote"]
            assert int(locator["byte_end"]) > int(locator["byte_start"])
            evidence_count += 1
            file = file_by_path.get(locator["path"])
            if file:
                if file["id"] not in raw_by_file:
                    raw_by_file[file["id"]] = call_bytes(
                        f"/code/files/{file['id']}/content"
                    )
                raw = raw_by_file[file["id"]]
                encoding = {
                    "UTF-8": "utf-8",
                    "Shift_JIS": "cp932",
                    "EUC-JP": "euc_jp",
                }.get(file["encoding"], "utf-8")
                quote = unicodedata.normalize("NFC", evidence["quote"])
                decoded = raw[
                    int(locator["byte_start"]) : int(locator["byte_end"])
                ].decode(encoding)
                assert quote == unicodedata.normalize("NFC", decoded), (
                    locator["path"],
                    locator["byte_start"],
                    locator["byte_end"],
                )
                byte_evidence_verified += 1

    append_one_file_change()
    sync2 = call(
        "POST",
        f"/projects/{project_id}/code/revisions:sync",
        {
            "repository_id": repository_id,
            "revision": f"{TAG}-r2",
            "committed_at": None,
        },
    )
    wait_job(sync2["job_id"])
    revisions2 = call(
        "GET",
        f"/projects/{project_id}/code/revisions?repository_id={repository_id}&limit=20",
    )["items"]
    revision2 = next(item for item in revisions2 if item["revision"] == f"{TAG}-r2")
    analysis2 = call(
        "POST",
        f"/projects/{project_id}/code/analysis-runs",
        {"revision_id": revision2["id"]},
    )
    wait_job(analysis2["job_id"])
    run2 = call("GET", f"/code/analysis-runs/{analysis2['run_id']}")
    total = int(run2["stats"]["files_total"])
    analyzed = int(run2["stats"]["files_analyzed"])
    incremental_fraction = analyzed / total
    assert 0 < incremental_fraction < 0.1, (analyzed, total)

    report = {
        "project_id": project_id,
        "repository_id": repository_id,
        "revision_r1": revision1,
        "revision_r2": revision2,
        "run_r1": run1,
        "run_r2": run2,
        "symbols": len(symbols1),
        "facts": len(facts1),
        "resolved_calls": len(resolved_calls),
        "reflection_calls": len(reflection_calls),
        "proxy_calls": len(proxy_calls),
        "facts_with_evidence": evidence_count,
        "byte_evidence_verified": byte_evidence_verified,
        "incremental_files_analyzed": analyzed,
        "incremental_files_total": total,
        "incremental_fraction": incremental_fraction,
    }
    out_dir = ROOT / "data" / "phase4-eval"
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "report.json").write_text(
        json.dumps(report, ensure_ascii=False, indent=2),
        encoding="utf-8",
    )
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:  # noqa: BLE001 - CLI should show evidence and exit 1
        print(f"phase4 evaluation failed: {error}", file=sys.stderr)
        raise
