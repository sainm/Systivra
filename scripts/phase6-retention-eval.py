#!/usr/bin/env python3
"""Phase 6 成果物保持の実測（detail/09 §8.3）。

前提: 001〜027 適用済み DB（既定 systivra_phase6_verify）、API（既定 18088）、
worker が起動済み。Runner と SAMPLE サイトは本スクリプトが起動・停止する。

  PGHOST=localhost PGUSER=sainm PGDATABASE=systivra_phase6_verify \
  SYSTIVRA_BASE_URL=http://127.0.0.1:18088 python3 scripts/phase6-retention-eval.py

実測項目:
  - trace 20 MB 上限（評価は 2 KB に縮小）→ TRUNCATED 記録
  - run 合計 20 GB 上限（評価は 4 KB に縮小）→ 以降の trace 無効
  - 保持期限 90 日 / trace 30 日 → 実体削除 + EXPIRED（リンク切れにしない）
  - 保持適用の冪等性（2 回目は 0 件）
  - run 単位の成果物削除 API と監査ログ
  - worker の日次保持ジョブ（起動時適用）が EXPIRED を立てる
"""

from __future__ import annotations

import importlib.util
import json
import os
import pathlib
import shutil
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location(
    "p6failure", ROOT / "scripts" / "phase6-failure-eval.py"
)
p6 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(p6)

RETENTION_DIR = ROOT / "data" / "phase6-retention"
REPORT_OUT = ROOT / "data" / "phase6-retention" / "report.json"
DB = p6.DB


def make_file(name: str, age_days: float, size: int = 128) -> dict:
    path = RETENTION_DIR / name
    path.write_bytes(b"x" * size)
    stamp = time.time() - age_days * 86400
    os.utime(path, (stamp, stamp))
    return {
        "kind": "TRACE" if name.endswith(".zip") else "SCREENSHOT" if name.endswith(".png") else "DOM",
        "uri": str(path.relative_to(ROOT)),
        "bytes": size,
    }


def import_execution(pid: str, run_id: str, case_id: str, artifacts: list[dict]) -> None:
    response = p6.call(
        "POST",
        f"/projects/{pid}/test/runs/{run_id}/executions:import",
        {
            "executions": [
                {
                    "case_id": case_id,
                    "attempt": 1,
                    "status": "FAILED",
                    "execution_source": "IMPORTED",
                    "failure_kind": "ASSERTION",
                    "failure_step": 3,
                    "expected": "期待値",
                    "actual": "実際値",
                    "duration_ms": 100,
                    "artifacts": artifacts,
                }
            ]
        },
    )
    assert response["accepted"] == 1, response


def execution_artifacts(pid: str, run_id: str, case_id: str) -> list[dict]:
    overview = p6.call("GET", f"/projects/{pid}/test/overview")
    execution = next(
        e for e in overview["executions"]
        if e["test_run_id"] == run_id and e["test_case_id"] == case_id
    )
    return execution["artifacts"]


def main() -> int:
    p6.reset()
    p6.psql("DELETE FROM core.audit_log WHERE action LIKE 'test.%';")
    if RETENTION_DIR.exists():
        shutil.rmtree(RETENTION_DIR)
    RETENTION_DIR.mkdir(parents=True)
    artifact_root = p6.ARTIFACT_ROOT
    artifact_root.mkdir(parents=True, exist_ok=True)
    for child in artifact_root.iterdir():
        if child.is_dir():
            shutil.rmtree(child)

    login = p6.call("POST", "/auth/login", {"login_id": p6.LOGIN_ID, "password": p6.PASSWORD})
    pid = login["memberships"][0]["project_id"]
    approved = p6.prepare(pid)
    results: dict = {}

    sample = p6.start_server(p6.SampleHandler, int(os.environ.get("SAMPLE_PORT", "18099")), ROOT / "samples" / "sample-web")
    p6.wait_http(f"{p6.WEB_BASE}/")
    try:
        originals = []
        for case in approved[:2]:
            dsl = json.loads(json.dumps(case["dsl"]))
            originals.append((case["id"], json.loads(json.dumps(dsl))))
            for index, step in enumerate(dsl["steps"]):
                if step["action"].startswith("assert_") or step["action"] == "field_error":
                    dsl["steps"][index] = {
                        "action": "field_error",
                        "target": {"label": "user.addr"},
                        "value": f"期待はずれ-RETENTION-{case['code']}",
                    }
                    break
            p6.set_case_dsl(case["id"], dsl)
        try:
            code, report = p6.run_runner({
                "PARALLEL": "1",
                "TRACE_MAX_BYTES": "2048",
                "RUN_ARTIFACT_LIMIT_BYTES": "4096",
            })
            assert code == 0, report
            assert report["failed"] == 2, report
            assert report["artifacts"]["trace_truncated"] >= 1, report
            assert report["artifacts"]["trace_disabled"] is True, report
            first = execution_artifacts(pid, report["run_id"], approved[0]["id"])
            trace = next(a for a in first if a["kind"] == "TRACE")
            assert trace.get("status") == "TRUNCATED", trace
            assert trace["bytes"] == 2048, trace
            trace_path = ROOT / trace["uri"]
            assert trace_path.stat().st_size == 2048
            second = execution_artifacts(pid, report["run_id"], approved[1]["id"])
            assert not any(a["kind"] == "TRACE" for a in second), second
            assert any(a["kind"] == "SCREENSHOT" for a in second), second
            results["trace_cap_and_run_budget"] = {
                "trace_max_bytes": 2048,
                "run_limit_bytes": 4096,
                "trace_truncated": report["artifacts"]["trace_truncated"],
                "trace_disabled": report["artifacts"]["trace_disabled"],
                "total_bytes": report["artifacts"]["total_bytes"],
            }
            print("ok trace_cap_and_run_budget")
        finally:
            for case_id, dsl in originals:
                p6.set_case_dsl(case_id, dsl)

        created = p6.call("POST", f"/projects/{pid}/test/runs", {
            "name": "retention-eval",
            "plan_id": None,
            "environment": {
                "id": "env-retention-eval",
                "base_url": p6.WEB_BASE,
                "guard_kind": "STAGING",
                "allowed_origins": ["127.0.0.1"],
                "parallel": 1,
            },
        })
        run_id = created["run_id"]

        old_png = make_file("old.png", 100)
        old_trace = make_file("old-trace.zip", 40)
        fresh_png = make_file("fresh.png", 0)
        import_execution(pid, run_id, approved[2]["id"], [old_png, old_trace, fresh_png])

        response = p6.call("POST", f"/projects/{pid}/test/artifacts:expire", {})
        assert response["expired"] == 2, response
        assert response["deleted_files"] == 2, response
        assert response["bytes_reclaimed"] == 256, response
        assert not (RETENTION_DIR / "old.png").exists()
        assert not (RETENTION_DIR / "old-trace.zip").exists()
        assert (RETENTION_DIR / "fresh.png").exists()
        artifacts = execution_artifacts(pid, run_id, approved[2]["id"])
        by_uri = {a["uri"]: a for a in artifacts}
        assert by_uri[old_png["uri"]]["status"] == "EXPIRED", artifacts
        assert by_uri[old_png["uri"]]["bytes"] == 0, artifacts
        assert by_uri[old_trace["uri"]]["status"] == "EXPIRED", artifacts
        assert by_uri[fresh_png["uri"]].get("status") is None, artifacts

        again = p6.call("POST", f"/projects/{pid}/test/artifacts:expire", {})
        assert again["expired"] == 0, again
        results["retention_expire"] = {"first": response, "second": again}
        print("ok retention_expire")

        cleanup = p6.call("POST", f"/projects/{pid}/test/runs/{run_id}/artifacts:cleanup")
        assert cleanup["expired"] == 1, cleanup
        assert cleanup["deleted_files"] == 1, cleanup
        assert not (RETENTION_DIR / "fresh.png").exists()
        artifacts = execution_artifacts(pid, run_id, approved[2]["id"])
        assert all(a.get("status") == "EXPIRED" for a in artifacts), artifacts
        audit = p6.psql(
            "SELECT count(*) FROM core.audit_log "
            f"WHERE action = 'test.artifacts.cleanup' AND target_id = '{run_id}';"
        )
        assert audit == "1", audit
        results["run_cleanup"] = cleanup
        print("ok run_cleanup")

        worker_old = make_file("worker-old.png", 120)
        import_execution(pid, run_id, approved[3]["id"], [worker_old])
        env = dict(os.environ)
        env.update({
            "DATABASE_URL": f"postgres://systivra_scheduler@localhost:5432/{DB}",
            "WORKER_TICK_SEC": "1",
            "SYSTIVRA_DATA_DIR": str(ROOT / "data"),
            "NEO4J_DISABLED": "1",
        })
        worker = subprocess.Popen(
            [str(ROOT / "target" / "debug" / "systivra-worker")],
            cwd=str(ROOT), env=env,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
        )
        try:
            deadline = time.time() + 20
            expired = False
            while time.time() < deadline:
                artifacts = execution_artifacts(pid, run_id, approved[3]["id"])
                if artifacts and artifacts[0].get("status") == "EXPIRED":
                    expired = True
                    break
                time.sleep(0.5)
            assert expired, "worker が保持期限を適用しなかった"
            assert not (RETENTION_DIR / "worker-old.png").exists()
        finally:
            worker.terminate()
            try:
                out, err = worker.communicate(timeout=5)
            except subprocess.TimeoutExpired:
                worker.kill()
                out, err = worker.communicate()
        assert "test artifacts retention applied" in out + err, (out + err)[-2000:]
        results["worker_retention"] = {"expired": True, "log": "test artifacts retention applied"}
        print("ok worker_retention")
    finally:
        sample.shutdown()
        sample.server_close()

    REPORT_OUT.write_text(json.dumps(results, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"\nwritten: {REPORT_OUT}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as error:  # noqa: BLE001
        print(f"FAILED: {error}", file=sys.stderr)
        raise