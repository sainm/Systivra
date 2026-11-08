#!/usr/bin/env python3
"""scheduler の BYPASSRLS 廃止（scheduler_scope 方式）の受入。

前提: 001〜029 適用済み DB（既定 systivra_phase6_verify）、API（既定 18088）、
scheduler ロールが NOBYPASSRLS、roles.sql 適用済み。worker は本スクリプトが起動する。

  PGHOST=localhost PGUSER=sainm PGDATABASE=systivra_phase6_verify \
  SYSTIVRA_BASE_URL=http://127.0.0.1:18088 python3 scripts/scheduler-rls-eval.py
"""

from __future__ import annotations

import importlib.util
import json
import os
import pathlib
import shutil
import subprocess
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location(
    "p6failure", ROOT / "scripts" / "phase6-failure-eval.py"
)
p6 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(p6)

DB = p6.DB
REPORT_DIR = ROOT / "data" / "scheduler-rls"
REPO = "repositories/phase4-eval-acceptance-final-1794041105"
ADMIN = os.environ.get("PGUSER", "sainm")


def psql_user(user: str, sql: str) -> str:
    result = subprocess.run(
        ["psql", "-U", user, "-d", DB, "-Atc", sql],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(f"psql({user}) failed: {result.stderr.strip()}")
    return result.stdout.strip()


def start_worker() -> subprocess.Popen:
    env = dict(os.environ)
    env.update({
        "DATABASE_URL": f"postgres://systivra_scheduler@localhost:5432/{DB}",
        "WORKER_TICK_SEC": "1",
        "SYSTIVRA_DATA_DIR": str(ROOT / "data"),
        "NEO4J_URI": "bolt://localhost:7687",
        "NEO4J_USER": "neo4j",
        "NEO4J_PASSWORD": "neo4j",
        "WORKER_ID": "scheduler-rls-eval",
    })
    env.pop("NEO4J_DISABLED", None)
    return subprocess.Popen(
        [str(ROOT / "target" / "debug" / "systivra-worker")],
        cwd=str(ROOT), env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )


def wait_job(job_id: str, timeout: float = 120) -> dict:
    deadline = time.time() + timeout
    while time.time() < deadline:
        job = p6.call("GET", f"/jobs/{job_id}")
        if job["status"] in ("SUCCEEDED", "SUCCEEDED_WITH_ERRORS", "FAILED", "CANCELLED", "TIMEOUT"):
            return job
        time.sleep(0.5)
    raise RuntimeError("job timeout")


def main() -> int:
    REPORT_DIR.mkdir(parents=True, exist_ok=True)
    results: dict = {"checks": []}

    def check(name: str, ok: bool, detail: str = "") -> None:
        results["checks"].append({"name": name, "ok": bool(ok), "detail": detail})

    bypass = p6.psql(
        "SELECT rolbypassrls FROM pg_roles WHERE rolname='systivra_scheduler';"
    )
    check("scheduler は BYPASSRLS を持たない", bypass == "f", bypass)
    policies = int(psql_user(ADMIN, (
        "SELECT count(*) FROM pg_policies WHERE policyname='scheduler_scope' "
        "AND (schemaname||'.'||tablename) IN "
        "('core.job','core.project','ai.approval_request','test.test_execution');"
    )))
    check("scheduler_scope が 4 表にある", policies == 4, str(policies))
    app_policy = int(psql_user(ADMIN, (
        "SELECT count(*) FROM pg_policies "
        "WHERE policyname='scheduler_scope' AND 'systivra_app' = ANY(roles);"
    )))
    check("方針は app に適用されない", app_policy == 0, str(app_policy))

    total_chunks = int(psql_user(ADMIN, "SELECT count(*) FROM corpus.chunk;"))
    visible_chunks = int(psql_user("systivra_scheduler", "SELECT count(*) FROM corpus.chunk;"))
    check(
        "scheduler は corpus を横断閲覧できない",
        visible_chunks == 0 and total_chunks >= 0,
        f"total={total_chunks} visible={visible_chunks}",
    )
    job_rows = int(psql_user("systivra_scheduler", "SELECT count(*) FROM core.job;"))
    check("scheduler は core.job を横断閲覧できる", job_rows > 0, str(job_rows))
    session_delete = psql_user(
        "sainm", "SELECT has_table_privilege('systivra_scheduler','core.session','DELETE');"
    )
    check("scheduler はセッションを掃除できる", session_delete == "t", session_delete)
    psql_user(
        "systivra_scheduler",
        "UPDATE test.test_execution SET duration_ms = duration_ms WHERE false;",
    )
    check("保持期限の UPDATE 文が RLS 下で通る", True, "0 rows")

    login = p6.call("POST", "/auth/login", {"login_id": p6.LOGIN_ID, "password": p6.PASSWORD})
    pid = login["memberships"][0]["project_id"]
    worker = start_worker()
    repo_path = ROOT / "data" / "repositories" / f"scheduler-rls-{int(time.time())}"
    try:
        tag = int(time.time())
        shutil.copytree(ROOT / "data" / REPO, repo_path)
        repo = p6.call(
            "POST", f"/projects/{pid}/code/repositories",
            {"kind": "DIR", "path": str(repo_path.relative_to(ROOT / "data")), "default_branch": None},
        )
        sync = p6.call(
            "POST", f"/projects/{pid}/code/revisions:sync",
            {"repository_id": repo["id"], "revision": f"rls-eval-{tag}", "committed_at": None},
        )
        job = wait_job(sync["job_id"])
        check("CODE_SYNC が完走", job["status"] in ("SUCCEEDED", "SUCCEEDED_WITH_ERRORS"), job["status"])
        revisions = p6.call(
            "GET", f"/projects/{pid}/code/revisions?repository_id={repo['id']}&limit=20"
        )["items"]
        revision = next(item for item in revisions if item["revision"] == f"rls-eval-{tag}")
        analysis = p6.call(
            "POST", f"/projects/{pid}/code/analysis-runs", {"revision_id": revision["id"]}
        )
        job = wait_job(analysis["job_id"])
        run = p6.call("GET", f"/code/analysis-runs/{analysis['run_id']}")
        facts = p6.call(
            "GET", f"/projects/{pid}/code/facts?revision_id={revision['id']}&limit=20"
        )["items"]
        check(
            "CODE_ANALYZE が Fact を生成",
            job["status"] in ("SUCCEEDED", "SUCCEEDED_WITH_ERRORS")
            and run["stats"]["files_analyzed"] > 0
            and len(facts) > 0,
            f"{job['status']} files={run['stats']['files_analyzed']} facts={len(facts)}",
        )

        test_run = p6.call(
            "POST", f"/projects/{pid}/test/runs",
            {
                "name": f"rls-test-{tag}",
                "plan_id": None,
                "environment": {
                    "id": f"env-rls-{tag}",
                    "base_url": "http://127.0.0.1:18099",
                    "guard_kind": "STAGING",
                    "allowed_origins": ["127.0.0.1"],
                    "parallel": 1,
                },
            },
        )
        job = wait_job(test_run["job_id"])
        overview = p6.call("GET", f"/projects/{pid}/test/overview")
        status = next(r["status"] for r in overview["runs"] if r["id"] == test_run["run_id"])
        check(
            "TEST_RUN prepare が READY まで進む",
            job["status"] in ("SUCCEEDED", "SUCCEEDED_WITH_ERRORS")
            and status in ("READY", "RUNNING", "DONE"),
            f"{job['status']} run={status}",
        )

        deadline = time.time() + 60
        pending = 1
        while time.time() < deadline:
            pending = int(psql_user(
                "sainm",
                "SELECT count(*) FROM core.outbox_event "
                "WHERE published_at IS NULL AND NOT dead_letter;",
            ))
            if pending == 0:
                break
            time.sleep(0.5)
        dead = int(psql_user(ADMIN, "SELECT count(*) FROM core.outbox_event WHERE dead_letter;"))
        check("outbox が drain される（dead letter 0）", pending == 0 and dead == 0, f"pending={pending} dead={dead}")

        deleted = psql_user(
            "systivra_scheduler",
            "DELETE FROM core.session WHERE expires_at < now() - interval '7 days';",
        )
        check("scheduler のセッション掃除が成功", True, f"deleted={deleted or 0}")
    finally:
        worker.terminate()
        try:
            worker.wait(timeout=5)
        except subprocess.TimeoutExpired:
            worker.kill()
        shutil.rmtree(repo_path, ignore_errors=True)

    ok = sum(1 for item in results["checks"] if item["ok"])
    results["passed"] = ok
    results["total"] = len(results["checks"])
    out = REPORT_DIR / "report.json"
    out.write_text(json.dumps(results, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(results, ensure_ascii=False, indent=2))
    print(f"\n{ok}/{results['total']} checks passed\nwritten: {out}")
    return 0 if ok == results["total"] else 1


if __name__ == "__main__":
    raise SystemExit(main())