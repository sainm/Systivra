#!/usr/bin/env python3
"""Phase 4 Git 同期の API E2E。

ローカルの bare リポジトリを作り、GIT リポジトリとして登録して
clone/fetch → revision SHA → tree digest → 解析 → 1 ファイル変更の増分を測る。
API と worker が起動していること。
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
BASE_URL = os.environ.get("SYSTIVRA_BASE_URL", "http://127.0.0.1:18093")
LOGIN_ID = os.environ.get("SYSTIVRA_LOGIN_ID", "alice")
PASSWORD = os.environ.get("SYSTIVRA_PASSWORD", "SystivraDemo1!")
PROJECT_ID = os.environ.get("SYSTIVRA_PROJECT_ID")
DATA_DIR = pathlib.Path(os.environ.get("SYSTIVRA_DATA_DIR", ROOT / "data"))
TAG = os.environ.get("PHASE4_GIT_TAG", f"git-{int(time.time())}")
SOURCE = DATA_DIR / f"git-eval-src-{TAG}"
BARE = DATA_DIR / "repositories" / f"git-eval-{TAG}.git"
REPORT = DATA_DIR / "phase4-git" / "report.json"
CHECKS: list[dict] = []

cookies = http.cookiejar.CookieJar()
opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(cookies))


def check(name: str, ok: bool, detail: str = "") -> None:
    CHECKS.append({"name": name, "ok": bool(ok), "detail": detail})
    print(f"  {'ok  ' if ok else 'FAIL'} {name}" + (f" ({detail})" if detail else ""), flush=True)


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
        with opener.open(request, timeout=60) as response:
            payload = response.read()
            return json.loads(payload) if payload else None
    except urllib.error.HTTPError as error:
        detail = error.read().decode("utf-8", "replace")
        raise RuntimeError(f"{method} {path}: HTTP {error.code}: {detail}") from error


def git(*args: str, cwd: pathlib.Path | None = None) -> str:
    proc = subprocess.run(
        ["git", *args],
        cwd=cwd,
        capture_output=True,
        text=True,
        check=True,
    )
    return proc.stdout.strip()


def wait_job(job_id: str) -> dict:
    terminal = {"SUCCEEDED", "SUCCEEDED_WITH_ERRORS", "FAILED", "CANCELLED", "TIMEOUT"}
    for _ in range(240):
        job = call("GET", f"/jobs/{job_id}")
        if job["status"] in terminal:
            if job["status"] not in {"SUCCEEDED", "SUCCEEDED_WITH_ERRORS"}:
                raise RuntimeError(f"job {job_id}: {job['status']} {job.get('errors')}")
            return job
        time.sleep(0.5)
    raise TimeoutError(f"job {job_id} did not finish")


def wait_job_failure(job_id: str) -> dict:
    terminal = {"SUCCEEDED", "SUCCEEDED_WITH_ERRORS", "FAILED", "CANCELLED", "TIMEOUT"}
    for _ in range(240):
        job = call("GET", f"/jobs/{job_id}")
        if job["status"] in terminal:
            return job
        time.sleep(0.5)
    raise TimeoutError(f"job {job_id} did not finish")


def write_java(path: pathlib.Path, name: str, body: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        f"package com.example;\n\npublic class {name} {{\n{body}\n}}\n",
        encoding="utf-8",
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

    SOURCE.mkdir(parents=True, exist_ok=True)
    git("init", "-q", "-b", "main", str(SOURCE))
    write_java(
        SOURCE / "src/main/java/com/example/Hello.java",
        "Hello",
        '    public String greet() {\n        return "hello";\n    }',
    )
    write_java(
        SOURCE / "src/main/java/com/example/World.java",
        "World",
        '    public String name() {\n        return "world";\n    }',
    )
    git("add", ".", cwd=SOURCE)
    git("-c", "user.name=eval", "-c", "user.email=eval@example.test", "commit", "-qm", "init", cwd=SOURCE)
    BARE.parent.mkdir(parents=True, exist_ok=True)
    git("clone", "-q", "--bare", "--branch", "main", str(SOURCE), str(BARE))
    first_sha = git("rev-parse", "HEAD", cwd=SOURCE)

    repository = call(
        "POST",
        f"/projects/{project_id}/code/repositories",
        {
            "kind": "GIT",
            "path": str(BARE.relative_to(DATA_DIR)),
            "default_branch": "main",
        },
    )
    check("GIT リポジトリを登録できる", repository["kind"] == "GIT", repository["path"])
    file_repository = call(
        "POST",
        f"/projects/{project_id}/code/repositories",
        {
            "kind": "GIT",
            "path": BARE.as_uri(),
            "default_branch": "main",
        },
    )
    check(
        "file:// URL をローカル Git パスとして登録できる",
        file_repository["id"] == repository["id"] and file_repository["kind"] == "GIT",
        file_repository["path"],
    )

    sync1 = call(
        "POST",
        f"/projects/{project_id}/code/revisions:sync",
        {"repository_id": repository["id"], "revision": "main"},
    )
    wait_job(sync1["job_id"])
    revisions = call(
        "GET",
        f"/projects/{project_id}/code/revisions?repository_id={repository['id']}&limit=10",
    )["items"]
    check("branch 同期が commit SHA に解決される", revisions[0]["revision"] == first_sha, revisions[0]["revision"])
    check("tree digest が付く", len(revisions[0]["tree_digest"]) > 0)
    check("ファイル数が 2", revisions[0]["file_count"] == 2, str(revisions[0]["file_count"]))

    analysis1 = call(
        "POST",
        f"/projects/{project_id}/code/analysis-runs",
        {"revision_id": revisions[0]["id"]},
    )
    wait_job(analysis1["job_id"])
    run1 = call("GET", f"/code/analysis-runs/{analysis1['run_id']}")
    check("解析が成功する", run1["status"] == "SUCCEEDED", run1["status"])
    check("Symbol が生成される", run1["symbols"] >= 2, str(run1["symbols"]))
    check("Fact が生成される", run1["facts"] > 0, str(run1["facts"]))

    git("tag", "v1", cwd=SOURCE)
    git("push", "-q", str(BARE), "v1", cwd=SOURCE)
    sync_tag = call(
        "POST",
        f"/projects/{project_id}/code/revisions:sync",
        {"repository_id": repository["id"], "revision": "v1"},
    )
    wait_job(sync_tag["job_id"])
    revisions_tag = call(
        "GET",
        f"/projects/{project_id}/code/revisions?repository_id={repository['id']}&limit=10",
    )["items"]
    check(
        "fetch 後の tag が commit SHA に解決される",
        len(revisions_tag) == 1 and revisions_tag[0]["revision"] == first_sha,
        str(revisions_tag[0]["revision"]),
    )

    write_java(
        SOURCE / "src/main/java/com/example/Hello.java",
        "Hello",
        '    public String greet() {\n        return "hello git";\n    }',
    )
    git("add", ".", cwd=SOURCE)
    git("-c", "user.name=eval", "-c", "user.email=eval@example.test", "commit", "-qm", "change", cwd=SOURCE)
    git("push", "-q", str(BARE), "main", cwd=SOURCE)
    second_sha = git("rev-parse", "HEAD", cwd=SOURCE)

    sync2 = call(
        "POST",
        f"/projects/{project_id}/code/revisions:sync",
        {"repository_id": repository["id"], "revision": "main"},
    )
    wait_job(sync2["job_id"])
    revisions2 = call(
        "GET",
        f"/projects/{project_id}/code/revisions?repository_id={repository['id']}&limit=10",
    )["items"]
    check("fetch 後に新しい revision が作られる", revisions2[0]["revision"] == second_sha, revisions2[0]["revision"])
    check("revision が 2 件", len(revisions2) == 2, str(len(revisions2)))
    check("parent が前の revision", revisions2[0]["parent_id"] == revisions[0]["id"])

    sync_sha = call(
        "POST",
        f"/projects/{project_id}/code/revisions:sync",
        {"repository_id": repository["id"], "revision": second_sha},
    )
    wait_job(sync_sha["job_id"])
    revisions_sha = call(
        "GET",
        f"/projects/{project_id}/code/revisions?repository_id={repository['id']}&limit=10",
    )["items"]
    check("commit SHA を直接同期できる", len(revisions_sha) == 2, str(len(revisions_sha)))
    check(
        "同じ commit SHA は revision を重複作成しない",
        revisions_sha[0]["revision"] == second_sha and revisions_sha[0]["tree_digest"] == revisions2[0]["tree_digest"],
    )

    analysis2 = call(
        "POST",
        f"/projects/{project_id}/code/analysis-runs",
        {"revision_id": revisions2[0]["id"]},
    )
    wait_job(analysis2["job_id"])
    run2 = call("GET", f"/code/analysis-runs/{analysis2['run_id']}")
    analyzed = int(run2["stats"]["files_analyzed"])
    check("変更ファイルだけ再解析される", analyzed == 1, f"files_analyzed={analyzed}")

    bad_sync = call(
        "POST",
        f"/projects/{project_id}/code/revisions:sync",
        {"repository_id": repository["id"], "revision": "branch-does-not-exist"},
    )
    failed_job = wait_job_failure(bad_sync["job_id"])
    check("同期失敗は FAILED で終端する", failed_job["status"] == "FAILED", failed_job["status"])
    check("同期失敗理由がジョブに残る", bool(failed_job.get("errors")), str(failed_job.get("errors")))
    revisions_failed = call(
        "GET",
        f"/projects/{project_id}/code/revisions?repository_id={repository['id']}&limit=10",
    )["items"]
    check("同期失敗では revision を追加しない", len(revisions_failed) == 2, str(len(revisions_failed)))

    report = {
        "tag": TAG,
        "repository_id": repository["id"],
        "first_revision": first_sha,
        "second_revision": second_sha,
        "symbols": run1["symbols"],
        "facts": run1["facts"],
        "incremental_files_analyzed": analyzed,
        "failed_sync_status": failed_job["status"],
        "checks": CHECKS,
        "passed": sum(1 for item in CHECKS if item["ok"]),
        "failed": sum(1 for item in CHECKS if not item["ok"]),
    }
    REPORT.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"\n{report['passed']}/{len(CHECKS)} checks passed", flush=True)
    return 0 if report["failed"] == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
