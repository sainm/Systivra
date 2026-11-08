#!/usr/bin/env python3
"""Phase 6: TestCase -[:VERIFIES]-> Requirement の outbox → Neo4j 投影（detail/07 / detail/09 §9.2）。

前提: 001〜027 適用済み DB（既定 systivra_phase6_verify）、API（既定 18088）、
Neo4j（bolt://localhost:7687）が起動済み。worker は本スクリプトが一時起動する。

  PGHOST=localhost PGUSER=sainm PGDATABASE=systivra_phase6_verify \
  SYSTIVRA_BASE_URL=http://127.0.0.1:18088 python3 scripts/phase6-graph-eval.py

実測:
  - derive が同じ transaction で PG graph_meta と outbox へ書く
  - worker の projector が EntityUpserted / RelationshipUpserted を Neo4j へ MERGE
  - Neo4j の VERIFIES 辺の件数が PG と一致し、resolution=INFERRED
  - Requirement ノードがルール由来（natural_key=rule:<uuid>）である
"""

from __future__ import annotations

import importlib.util
import json
import os
import pathlib
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location(
    "p6failure", ROOT / "scripts" / "phase6-failure-eval.py"
)
p6 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(p6)

REPORT_DIR = ROOT / "data" / "phase6-graph"
NEO4J = os.environ.get("NEO4J_SHELL", "cypher-shell")
NEO4J_ARGS = [
    "-a", os.environ.get("NEO4J_URI", "bolt://localhost:7687"),
    "-u", os.environ.get("NEO4J_USER", "neo4j"),
    "-p", os.environ.get("NEO4J_PASSWORD", "neo4j"),
    "--format", "plain",
]

RULES = [
    {"kind": "SPEC", "field_ref": "user.name", "text": "必須項目", "evidence_ids": []},
    {"kind": "SPEC", "field_ref": "user.memo", "text": "全角20文字以内", "evidence_ids": []},
    {"kind": "SPEC", "field_ref": "user.code",
     "text": "pattern=\"^[ぁ-ん]+$\" 正規表現", "evidence_ids": []},
]


def cypher(query: str) -> list[str]:
    result = subprocess.run(
        [NEO4J, *NEO4J_ARGS, query], capture_output=True, text=True, timeout=60
    )
    if result.returncode != 0:
        raise RuntimeError(f"cypher-shell failed: {result.stderr.strip()}")
    return [line for line in result.stdout.strip().splitlines() if line]


def main() -> int:
    p6.reset()
    p6.psql("DELETE FROM core.outbox_dead_letter;")
    p6.psql("DELETE FROM core.outbox_event;")
    REPORT_DIR.mkdir(parents=True, exist_ok=True)

    login = p6.call("POST", "/auth/login", {"login_id": p6.LOGIN_ID, "password": p6.PASSWORD})
    pid = login["memberships"][0]["project_id"]
    p6.call("POST", f"/projects/{pid}/test/rules:extract", {"sources": RULES})
    impact_id = "d0000000-0000-4000-8000-0000000000aa"
    report = {
        "items": [
            {"item_id": "e0000000-0000-4000-8000-0000000000b1",
             "label": "ユーザ編集画面", "category": "SCREENS", "why": "変更画面"},
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
        {"impact_id": impact_id, "name": "graph-eval", "max_drafts": 50},
    )

    pg_requirements = int(p6.psql(
        "SELECT count(*) FROM graph_meta.entity "
        f"WHERE project_id = '{pid}' AND revision = 'test' AND entity_type = 'Requirement';"
    ))
    pg_cases = int(p6.psql(
        "SELECT count(*) FROM graph_meta.entity "
        f"WHERE project_id = '{pid}' AND revision = 'test' AND entity_type = 'TestCase';"
    ))
    pg_verifies = int(p6.psql(
        "SELECT count(*) FROM graph_meta.relationship "
        f"WHERE project_id = '{pid}' AND revision = 'test' "
        "AND rel_type = 'VERIFIES' AND valid_to IS NULL;"
    ))
    pg_verified_cases = int(p6.psql(
        "SELECT count(DISTINCT src_id) FROM graph_meta.relationship "
        f"WHERE project_id = '{pid}' AND revision = 'test' "
        "AND rel_type = 'VERIFIES' AND valid_to IS NULL;"
    ))
    pending = int(p6.psql("SELECT count(*) FROM core.outbox_event WHERE published_at IS NULL AND NOT dead_letter;"))
    assert pg_requirements == len(RULES), pg_requirements
    assert pg_verifies > 0 and pending == pg_requirements + pg_cases + pg_verifies, (pending, pg_verifies)

    cypher(
        "MATCH (e:Entity) WHERE e.project_id = "
        f"'{pid}' AND e.revision = 'test' DETACH DELETE e"
    )

    env = dict(os.environ)
    env.update({
        "DATABASE_URL": f"postgres://systivra_scheduler@localhost:5432/{p6.DB}",
        "WORKER_TICK_SEC": "1",
        "SYSTIVRA_DATA_DIR": str(ROOT / "data"),
        "NEO4J_URI": os.environ.get("NEO4J_URI", "bolt://localhost:7687"),
        "NEO4J_USER": os.environ.get("NEO4J_USER", "neo4j"),
        "NEO4J_PASSWORD": os.environ.get("NEO4J_PASSWORD", "neo4j"),
    })
    env.pop("NEO4J_DISABLED", None)
    worker = subprocess.Popen(
        [str(ROOT / "target" / "debug" / "systivra-worker")],
        cwd=str(ROOT), env=env,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
    )
    try:
        deadline = time.time() + 60
        drained = False
        while time.time() < deadline:
            left = int(p6.psql(
                "SELECT count(*) FROM core.outbox_event WHERE published_at IS NULL AND NOT dead_letter;"
            ))
            dead = int(p6.psql(
                "SELECT count(*) FROM core.outbox_event WHERE dead_letter;"
            ))
            assert dead == 0, "outbox に dead letter がある"
            if left == 0:
                drained = True
                break
            time.sleep(1)
        assert drained, "outbox が drain されない"
    finally:
        worker.terminate()
        try:
            out, err = worker.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            worker.kill()
            out, err = worker.communicate()
    assert "graph outbox projected" in out + err, (out + err)[-2000:]

    rows = cypher(
        "MATCH (tc:Entity {entity_type: 'TestCase'})-[r:VERIFIES]->"
        "(req:Entity {entity_type: 'Requirement'}) "
        f"WHERE tc.project_id = '{pid}' AND req.project_id = '{pid}' "
        "RETURN count(r) AS rels, count(DISTINCT tc) AS cases, "
        "count(DISTINCT req) AS reqs, collect(DISTINCT r.resolution) AS resolutions"
    )
    assert len(rows) >= 1, rows
    rels, cases, reqs, resolutions = [part.strip() for part in rows[-1].split(",")]
    assert int(rels) == pg_verifies, (rels, pg_verifies)
    assert int(cases) == pg_verified_cases, (cases, pg_verified_cases)
    assert int(reqs) == pg_requirements, (reqs, pg_requirements)
    assert "INFERRED" in resolutions, resolutions

    all_cases = [
        line for line in cypher(
            "MATCH (tc:Entity {entity_type: 'TestCase'}) "
            f"WHERE tc.project_id = '{pid}' RETURN count(tc)"
        )
        if line.isdigit()
    ]
    assert all_cases and int(all_cases[-1]) == pg_cases, (all_cases, pg_cases)

    natural_keys = [
        line.strip().strip('"')
        for line in cypher(
            "MATCH (req:Entity {entity_type: 'Requirement'}) "
            f"WHERE req.project_id = '{pid}' RETURN req.natural_key ORDER BY req.natural_key"
        )
        if line.strip().strip('"').startswith("rule:")
    ]
    assert len(natural_keys) == len(RULES), natural_keys
    assert all(key.startswith("rule:") for key in natural_keys), natural_keys

    summary = {
        "pg": {
            "requirements": pg_requirements,
            "test_cases": pg_cases,
            "verifies": pg_verifies,
            "outbox_events": pending,
        },
        "neo4j": {
            "verifies": int(rels),
            "cases_with_verifies": int(cases),
            "test_cases": int(all_cases[-1]),
            "requirements": int(reqs),
            "resolutions": resolutions,
            "requirement_natural_keys": natural_keys,
        },
    }
    out_path = REPORT_DIR / "report.json"
    out_path.write_text(json.dumps(summary, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(summary, ensure_ascii=False, indent=2))
    print(f"\nwritten: {out_path}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as error:  # noqa: BLE001
        print(f"FAILED: {error}", file=sys.stderr)
        raise