#!/usr/bin/env python3
"""Phase 3 投影運用の受入（detail/07 §5–§7 / G-06 / G-07 / G-09 * 複数 projector）。

前提: 001〜028 適用済み DB（既定 systivra_phase6_verify）、API（既定 18088、
NEO4J 有効・GRAPH_REBUILD_BATCH / GRAPH_REBUILD_STEP_DELAY_MS 設定済み）、
Neo4j（bolt://localhost:7687）。worker は本スクリプトが起動する。

  PGHOST=localhost PGUSER=sainm PGDATABASE=systivra_phase6_verify \
  SYSTIVRA_BASE_URL=http://127.0.0.1:18088 python3 scripts/phase3-graph-ops-eval.py
"""

from __future__ import annotations

import importlib.util
import json
import os
import pathlib
import re
import subprocess
import sys
import threading
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location(
    "p6failure", ROOT / "scripts" / "phase6-failure-eval.py"
)
p6 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(p6)

REPORT_DIR = ROOT / "data" / "phase3-graph-ops"
NEO4J = ["cypher-shell", "-a", "bolt://localhost:7687", "-u", "neo4j", "-p", "neo4j", "--format", "plain"]

RULES = [
    {"kind": "SPEC", "field_ref": "user.name", "text": "必須項目", "evidence_ids": []},
    {"kind": "SPEC", "field_ref": "user.memo", "text": "全角20文字以内", "evidence_ids": []},
    {"kind": "SPEC", "field_ref": "user.code",
     "text": "pattern=\"^[ぁ-ん]+$\" 正規表現", "evidence_ids": []},
]


def cypher(query: str) -> list[str]:
    result = subprocess.run([*NEO4J, query], capture_output=True, text=True, timeout=60)
    if result.returncode != 0:
        raise RuntimeError(f"cypher failed: {result.stderr.strip()}")
    return [line for line in result.stdout.strip().splitlines() if line.strip()]


def neo4j_counts(pid: str) -> tuple[int, int]:
    rows = cypher(
        "MATCH (e:Entity {project_id: '" + pid + "'}) "
        "OPTIONAL MATCH (e)-[r]->(:Entity {project_id: '" + pid + "'}) "
        "RETURN count(DISTINCT e) AS nodes, count(DISTINCT r) AS rels"
    )
    parts = rows[-1].split(", ")
    return int(parts[0]), int(parts[1])


def shadow_nodes(pid: str) -> int:
    rows = cypher(
        "MATCH (n) WHERE n.project_id = '" + pid + "' "
        "AND any(l IN labels(n) WHERE l STARTS WITH 'Entity__') RETURN count(n)"
    )
    return int(rows[-1])


def pending() -> int:
    return int(p6.psql(
        "SELECT count(*) FROM core.outbox_event WHERE published_at IS NULL AND NOT dead_letter;"
    ))


def dead() -> int:
    return int(p6.psql("SELECT count(*) FROM core.outbox_event WHERE dead_letter;"))


def stats(pid: str) -> dict:
    return p6.call("GET", f"/projects/{pid}/graph/stats")


def reset(pid: str) -> None:
    p6.reset()
    p6.psql("DELETE FROM core.outbox_dead_letter;")
    p6.psql("DELETE FROM core.outbox_event;")
    p6.psql(f"DELETE FROM graph_meta.relationship WHERE project_id = '{pid}';")
    p6.psql(f"DELETE FROM graph_meta.entity WHERE project_id = '{pid}';")
    p6.psql(f"DELETE FROM graph_meta.projection_build WHERE project_id = '{pid}';")
    p6.psql(f"DELETE FROM graph_meta.projection_drift WHERE project_id = '{pid}';")
    p6.psql(f"DELETE FROM graph_meta.projection_state WHERE project_id = '{pid}';")
    cypher(f"MATCH (e:Entity) WHERE e.project_id = '{pid}' DETACH DELETE e")


def derive_plan(pid: str, name: str) -> None:
    impact_id = "d0000000-0000-4000-8000-0000000000bb"
    report = {
        "items": [
            {"item_id": "e0000000-0000-4000-8000-0000000000c1",
             "label": "ユーザ編集画面", "category": "SCREENS", "why": "変更画面"},
            {"item_id": "e0000000-0000-4000-8000-0000000000c2",
             "label": "UserService.save", "category": "CODE", "why": "変更 Method"},
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
        {"impact_id": impact_id, "name": name, "max_drafts": 50},
    )


def start_worker(worker_id: str, claim_limit: int = 5000) -> subprocess.Popen:
    env = dict(os.environ)
    env.update({
        "DATABASE_URL": f"postgres://systivra_scheduler@localhost:5432/{p6.DB}",
        "WORKER_TICK_SEC": "1",
        "SYSTIVRA_DATA_DIR": str(ROOT / "data"),
        "NEO4J_URI": "bolt://localhost:7687",
        "NEO4J_USER": "neo4j",
        "NEO4J_PASSWORD": "neo4j",
        "WORKER_ID": worker_id,
        "GRAPH_CLAIM_LIMIT": str(claim_limit),
    })
    env.pop("NEO4J_DISABLED", None)
    return subprocess.Popen(
        [str(ROOT / "target" / "debug" / "systivra-worker")],
        cwd=str(ROOT), env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
    )


ANSI = re.compile(r"\x1b\[[0-9;]*m")


def stop_worker(worker: subprocess.Popen) -> str:
    worker.terminate()
    try:
        out, err = worker.communicate(timeout=5)
    except subprocess.TimeoutExpired:
        worker.kill()
        out, err = worker.communicate()
    return ANSI.sub("", out + err)


def wait_for(condition, timeout: float, what: str):
    deadline = time.time() + timeout
    while time.time() < deadline:
        value = condition()
        if value:
            return value
        time.sleep(0.2)
    raise RuntimeError(f"timeout: {what}")


def main() -> int:
    REPORT_DIR.mkdir(parents=True, exist_ok=True)
    login = p6.call("POST", "/auth/login", {"login_id": p6.LOGIN_ID, "password": p6.PASSWORD})
    pid = login["memberships"][0]["project_id"]
    reset(pid)
    results: dict = {}

    p6.call("POST", f"/projects/{pid}/test/rules:extract", {"sources": RULES})
    derive_plan(pid, "graph-ops")

    worker = start_worker("w1")
    try:
        wait_for(lambda: pending() == 0, 30, "initial projection drain")
    finally:
        log = stop_worker(worker)
    assert "graph outbox projected" in log, log[-1500:]
    assert dead() == 0
    base = stats(pid)
    neo = neo4j_counts(pid)
    assert (base["pg_nodes"], base["pg_relationships"]) == neo, (base, neo)
    results["initial_projection"] = {
        "pg": [base["pg_nodes"], base["pg_relationships"]],
        "neo4j": list(neo),
        "pending_events": base["pending_events"],
    }
    print("ok initial_projection", neo)

    build_states: list[str] = []
    outcome: dict = {}

    def rebuild() -> None:
        try:
            outcome["response"] = p6.call("POST", f"/projects/{pid}/graph/rebuild")
        except Exception as error:  # noqa: BLE001
            outcome["error"] = str(error)

    thread = threading.Thread(target=rebuild)
    thread.start()
    deadline = time.time() + 15
    while time.time() < deadline and thread.is_alive():
        current = stats(pid)
        build = current.get("last_build") or {}
        if build.get("status"):
            build_states.append(build["status"])
        if build.get("status") == "BUILDING":
            assert current["neo4j_nodes"] == neo[0], current
        time.sleep(0.1)
    thread.join(timeout=30)
    assert not outcome.get("error"), outcome
    response = outcome["response"]
    assert response["status"] == "SWITCHED", response
    assert response["drift_rate"] == 0.0, response
    assert "BUILDING" in build_states, build_states
    after = stats(pid)
    assert after["last_build"]["status"] == "SWITCHED", after
    assert (after["pg_nodes"], after["pg_relationships"]) == neo4j_counts(pid)
    assert shadow_nodes(pid) == 0
    results["shadow_rebuild"] = {
        "status": response["status"],
        "drift_rate": response["drift_rate"],
        "entities": response["entities"],
        "relationships": response["relationships"],
        "observed_states": sorted(set(build_states)),
    }
    print("ok shadow_rebuild", response["status"], sorted(set(build_states)))

    outcome = {}
    thread = threading.Thread(target=rebuild)
    thread.start()
    wait_for(
        lambda: (stats(pid).get("last_build") or {}).get("status") == "BUILDING",
        15,
        "build start for abort",
    )
    abort = p6.call("POST", f"/projects/{pid}/graph/rebuild/abort")
    thread.join(timeout=30)
    assert abort["status"] == "ABORTED", abort
    assert outcome.get("response", {}).get("status") == "ABORTED", outcome
    after = stats(pid)
    assert after["last_build"]["status"] == "ABORTED", after
    assert (after["pg_nodes"], after["pg_relationships"]) == neo4j_counts(pid)
    assert after["neo4j_nodes"] == neo[0], after
    assert shadow_nodes(pid) == 0
    results["abort_rebuild"] = {
        "abort": abort,
        "response_status": outcome["response"]["status"],
        "neo4j_nodes": after["neo4j_nodes"],
        "shadow_nodes": 0,
    }
    print("ok abort_rebuild")

    derive_plan(pid, "graph-ops-multi")
    before = pending()
    assert before > 10, before
    p6.psql(
        "UPDATE core.outbox_event SET locked_by = 'dead-worker', "
        "locked_until = now() - interval '1 minute' "
        "WHERE seq = (SELECT min(seq) FROM core.outbox_event "
        "WHERE published_at IS NULL AND NOT dead_letter);"
    )
    w1 = start_worker("w1", claim_limit=25)
    w2 = start_worker("w2", claim_limit=25)
    try:
        wait_for(lambda: pending() == 0, 60, "multi projector drain")
    finally:
        log1 = stop_worker(w1)
        log2 = stop_worker(w2)
    assert dead() == 0
    published = [
        int(match.group(1))
        for log in (log1, log2)
        for match in re.finditer(r"published=(\d+)", log)
    ]
    per_worker = [
        sum(int(m.group(1)) for m in re.finditer(r"published=(\d+)", log))
        for log in (log1, log2)
    ]
    assert all("graph outbox projected" in log for log in (log1, log2)), (log1[-800:], log2[-800:])
    assert sum(per_worker) >= before, (per_worker, log1[-2500:], log2[-2500:])
    after = stats(pid)
    assert (after["pg_nodes"], after["pg_relationships"]) == neo4j_counts(pid)
    assert after["pending_events"] == 0, after
    results["multi_projector"] = {
        "events": before,
        "published_per_worker": per_worker,
        "published_lines": published,
        "dead_letters": dead(),
        "pg": [after["pg_nodes"], after["pg_relationships"]],
        "neo4j": list(neo4j_counts(pid)),
        "stale_lock_reclaimed": True,
    }
    print("ok multi_projector", per_worker)

    worker = start_worker("w-drift")
    failed: Exception | None = None
    try:
        def drift_row():
            return p6.psql(
                "SELECT count(*) FROM graph_meta.projection_drift "
                f"WHERE project_id = '{pid}';"
            )

        wait_for(lambda: int(drift_row()) >= 1, 30, "startup drift check")
    except RuntimeError as error:
        failed = error
    finally:
        log = stop_worker(worker)
    assert failed is None, (str(failed), log[-2500:])
    assert "graph drift check ok" in log, log[-1500:]
    entry = p6.psql(
        "SELECT degraded, drift_rate FROM graph_meta.projection_drift "
        f"WHERE project_id = '{pid}' ORDER BY checked_at DESC LIMIT 1;"
    )
    assert entry.startswith("f|0.00000"), entry
    current = stats(pid)
    assert current["last_drift_checked_at"] is not None, current
    assert current["drift_degraded"] is False, current

    cypher(
        "MATCH (e:Entity {project_id: '" + pid + "'}) "
        "WITH e LIMIT 1 DETACH DELETE e"
    )
    worker = start_worker("w-drift-2")
    try:
        def degraded_row():
            return p6.psql(
                "SELECT count(*) FROM graph_meta.projection_drift "
                f"WHERE project_id = '{pid}' AND degraded;"
            )

        wait_for(lambda: int(degraded_row()) >= 1, 30, "degraded drift check")
    finally:
        log = stop_worker(worker)
    assert "graph drift detected" in log, log[-1500:]
    current = stats(pid)
    assert current["drift_degraded"] is True, current
    audit = p6.psql(
        "SELECT count(*) FROM core.audit_log "
        f"WHERE action = 'graph.drift.detected' AND project_id = '{pid}';"
    )
    assert audit != "0", audit
    results["daily_drift"] = {
        "checks": int(p6.psql(
            "SELECT count(*) FROM graph_meta.projection_drift "
            f"WHERE project_id = '{pid}';"
        )),
        "degraded": True,
        "audit_rows": int(audit),
    }
    print("ok daily_drift")

    p6.call("POST", f"/projects/{pid}/graph/rebuild")
    restored = stats(pid)
    assert restored["drift_degraded"] is False or restored["last_build"]["status"] == "SWITCHED"
    assert (restored["pg_nodes"], restored["pg_relationships"]) == neo4j_counts(pid)
    print("ok rebuild_after_drift")

    out = REPORT_DIR / "report.json"
    out.write_text(json.dumps(results, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"\nwritten: {out}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as error:  # noqa: BLE001
        print(f"FAILED: {error}", file=sys.stderr)
        raise