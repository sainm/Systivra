#!/usr/bin/env python3
"""Phase 4 Fact persistence / supersede / Neo4j reprojection benchmark.

This does not run the source analyzer. It generates a deterministic 2.5M-row
Fact-equivalent data set, persists it with PostgreSQL COPY in 5,000-row
statements, applies a supersede_then_insert increment, and rebuilds a matching
5,500-node / 2.5M-edge graph through the production graph rebuild API.

Required:
  - PostgreSQL database with all migrations and scripts/seed-dev.sh applied.
  - systivra-api and Neo4j reachable.

Environment:
  PHASE4_FACT_DB          database name (default: systivra)
  PHASE4_FACT_COUNT       default: 2500000
  PHASE4_SUPERSEDE_COUNT  default: 25000 (approximately 1%)
  SYSTIVRA_BASE_URL       default: http://127.0.0.1:18093
"""

from __future__ import annotations

import http.cookiejar
import json
import os
import pathlib
import re
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[1]
DATABASE = os.environ.get("PHASE4_FACT_DB", "systivra")
BASE_URL = os.environ.get("SYSTIVRA_BASE_URL", "http://127.0.0.1:18093")
LOGIN_ID = os.environ.get("SYSTIVRA_LOGIN_ID", "alice")
PASSWORD = os.environ.get("SYSTIVRA_PASSWORD", "SystivraDemo1!")
FACT_COUNT = int(os.environ.get("PHASE4_FACT_COUNT", "2500000"))
SUPERSEDE_COUNT = int(os.environ.get("PHASE4_SUPERSEDE_COUNT", "25000"))
COPY_BATCH = int(os.environ.get("PHASE4_FACT_COPY_BATCH", "5000"))
KEEP_DATABASE = os.environ.get("PHASE4_FACT_KEEP_DB", "0") == "1"
REPORT = pathlib.Path(
    os.environ.get("PHASE4_FACT_REPORT", ROOT / "data" / "phase4-scale" / "fact-report.json")
)

WORKSPACE_ID = "44444444-4444-4444-8444-444444444441"
PROJECT_ID = "44444444-4444-4444-8444-444444444442"
REPOSITORY_ID = "44444444-4444-4444-8444-444444444443"
REVISION_ID = "44444444-4444-4444-8444-444444444444"
ALICE_ID = "33333333-3333-4333-8333-333333333333"

cookies = http.cookiejar.CookieJar()
opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(cookies))


def psql(sql: str) -> str:
    result = subprocess.run(
        [
            "psql",
            "-X",
            "-q",
            "-v",
            "ON_ERROR_STOP=1",
            "-d",
            DATABASE,
            "-Atc",
            sql,
        ],
        check=True,
        capture_output=True,
        text=True,
    )
    return result.stdout.strip()


def cypher(query: str) -> str:
    user = os.environ.get("NEO4J_USER", "neo4j")
    password = os.environ.get("NEO4J_PASSWORD", "neo4j")
    address = os.environ.get("NEO4J_URI", "bolt://127.0.0.1:7687")
    database = os.environ.get("NEO4J_DATABASE", "neo4j")
    result = subprocess.run(
        [
            "cypher-shell",
            "-a",
            address,
            "-u",
            user,
            "-p",
            password,
            "-d",
            database,
            "--format",
            "plain",
            query,
        ],
        check=True,
        capture_output=True,
        text=True,
    )
    return result.stdout.strip()


def call(method: str, path: str, body: dict | None = None, timeout: int = 7200):
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
        with opener.open(request, timeout=timeout) as response:
            payload = response.read()
            return json.loads(payload) if payload else None
    except urllib.error.HTTPError as error:
        detail = error.read().decode("utf-8", "replace")
        raise RuntimeError(f"{method} {path}: HTTP {error.code}: {detail}") from error


def cleanup() -> None:
    try:
        cypher(
            f"MATCH (e:Entity {{project_id: '{PROJECT_ID}'}}) DETACH DELETE e"
        )
        cypher(
            f"MATCH (s:ProjectionState {{project_id: '{PROJECT_ID}'}}) DELETE s"
        )
    except Exception as error:
        print(f"warning: Neo4j cleanup failed: {error}", file=sys.stderr)
    if KEEP_DATABASE:
        print("keeping benchmark database as requested", file=sys.stderr)
        return
    try:
        psql(
            f"""
            BEGIN;
            SET LOCAL session_replication_role = replica;
            DELETE FROM graph_meta.relationship WHERE project_id = '{PROJECT_ID}';
            DELETE FROM graph_meta.entity WHERE project_id = '{PROJECT_ID}';
            DELETE FROM graph_meta.projection_state WHERE project_id = '{PROJECT_ID}';
            DELETE FROM graph_meta.projection_build WHERE project_id = '{PROJECT_ID}';
            DELETE FROM core.outbox_event
             WHERE payload->>'project_id' = '{PROJECT_ID}';
            DELETE FROM code.fact_evidence WHERE project_id = '{PROJECT_ID}';
            DELETE FROM code.fact WHERE project_id = '{PROJECT_ID}';
            DELETE FROM corpus.evidence WHERE project_id = '{PROJECT_ID}';
            DELETE FROM code.symbol WHERE project_id = '{PROJECT_ID}';
            DELETE FROM code.analysis_run WHERE project_id = '{PROJECT_ID}';
            DELETE FROM code.source_file WHERE project_id = '{PROJECT_ID}';
            DELETE FROM code.code_revision WHERE project_id = '{PROJECT_ID}';
            DELETE FROM code.repository WHERE project_id = '{PROJECT_ID}';
            DELETE FROM core.project_member WHERE project_id = '{PROJECT_ID}';
            DELETE FROM core.project WHERE id = '{PROJECT_ID}';
            DELETE FROM core.workspace WHERE id = '{WORKSPACE_ID}';
            COMMIT;
            """
        )
    except Exception as error:
        print(f"warning: PostgreSQL cleanup failed: {error}", file=sys.stderr)


def seed() -> None:
    cleanup()
    psql(
        f"""
        INSERT INTO core.workspace (id, code, name)
        VALUES ('{WORKSPACE_ID}', 'PHASE4-FACT-SCALE', 'Phase 4 Fact scale SAMPLE');
        INSERT INTO core.project (id, workspace_id, code, name, is_sample, source_kind)
        VALUES (
          '{PROJECT_ID}', '{WORKSPACE_ID}', 'PHASE4-FACT-SCALE',
          'Phase 4 2.5M Fact SAMPLE', true, 'SNAPSHOT'
        );
        INSERT INTO core.project_member (project_id, user_id, role)
        VALUES ('{PROJECT_ID}', '{ALICE_ID}', 'OWNER');
        INSERT INTO code.repository (id, project_id, kind, url)
        VALUES ('{REPOSITORY_ID}', '{PROJECT_ID}', 'SNAPSHOT', 'phase4-fact-scale');
        INSERT INTO code.code_revision
          (id, project_id, repository_id, revision, tree_digest, snapshot_key)
        VALUES (
          '{REVISION_ID}', '{PROJECT_ID}', '{REPOSITORY_ID}',
          'scale-facts-v1', decode('00', 'hex'), 'code/phase4-fact-scale/v1'
        );
        """
    )


def copy_facts() -> float:
    if FACT_COUNT <= 0 or FACT_COUNT % COPY_BATCH != 0:
        raise ValueError("PHASE4_FACT_COUNT must be a positive multiple of the COPY batch")
    process = subprocess.Popen(
        [
            "psql",
            "-X",
            "-q",
            "-v",
            "ON_ERROR_STOP=1",
            "-d",
            DATABASE,
        ],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    assert process.stdin is not None
    statement = (
        "COPY code.fact "
        "(project_id,revision_id,subject_kind,subject_key,relation,"
        "object_kind,object_key,resolution,confidence,analysis_engine,"
        "engine_version,candidates,fact_hash) "
        "FROM STDIN WITH (FORMAT csv);\n"
    ).encode("utf-8")
    started = time.monotonic()
    index = 0
    try:
        while index < FACT_COUNT:
            process.stdin.write(statement)
            for _ in range(COPY_BATCH):
                subject = index // 5000
                object_index = index % 5000
                process.stdin.write(
                    (
                        f"{PROJECT_ID},{REVISION_ID},Method,s{subject:04d},CALLS,"
                        f"Method,o{object_index:04d},RESOLVED,0.98,phase4-scale,"
                        f"1,\"[]\",\\x00\n"
                    ).encode("utf-8")
                )
                index += 1
            process.stdin.write(b"\\.\n")
            if index % 100_000 == 0:
                print(f"  COPY {index:,} / {FACT_COUNT:,}", flush=True)
        stdout, stderr = process.communicate(timeout=7200)
        if process.returncode != 0:
            raise RuntimeError(
                f"psql COPY failed ({process.returncode}): "
                f"{stderr.decode('utf-8', 'replace')} {stdout.decode('utf-8', 'replace')}"
            )
    except Exception:
        process.kill()
        raise
    return time.monotonic() - started


def supersede_increment() -> float:
    started = time.monotonic()
    psql(
        f"""
        BEGIN;
        CREATE TEMP TABLE staged_facts ON COMMIT DROP AS
        SELECT gen_random_uuid() AS new_id,
               project_id, revision_id, subject_kind, subject_key, relation,
               object_kind, object_key, 'INFERRED'::text AS resolution,
               0.65::real AS confidence, analysis_engine, engine_version,
               NULL::uuid AS conflict_group,
               'phase4 scale supersede increment'::text AS unresolved_reason,
               candidates, fact_hash
          FROM code.fact
         WHERE project_id = '{PROJECT_ID}' AND revision_id = '{REVISION_ID}'
           AND valid_to IS NULL
         ORDER BY id
         LIMIT {SUPERSEDE_COUNT};

        UPDATE code.fact AS old
           SET valid_to = now(), superseded_by = staged.new_id
          FROM staged_facts AS staged
         WHERE old.project_id = staged.project_id
           AND old.revision_id = staged.revision_id
           AND old.subject_key = staged.subject_key
           AND old.relation = staged.relation
           AND old.object_key = staged.object_key
           AND old.analysis_engine = staged.analysis_engine
           AND old.valid_to IS NULL;

        INSERT INTO code.fact
          (id, project_id, revision_id, subject_kind, subject_key, relation,
           object_kind, object_key, resolution, confidence, analysis_engine,
           engine_version, conflict_group, unresolved_reason, candidates, fact_hash)
        SELECT new_id, project_id, revision_id, subject_kind, subject_key, relation,
               object_kind, object_key, resolution, confidence, analysis_engine,
               engine_version, conflict_group, unresolved_reason, candidates, fact_hash
          FROM staged_facts;
        COMMIT;
        """
    )
    return time.monotonic() - started


def build_pg_graph() -> float:
    started = time.monotonic()
    psql(
        f"""
        INSERT INTO graph_meta.entity
          (project_id, entity_type, natural_key, name, qualified_name,
           properties, ontology_version, revision, resolution,
           first_seen_rev, last_seen_rev, projection_seq)
        SELECT '{PROJECT_ID}', 'Method', key, key, key,
               jsonb_build_object('source', 'phase4-fact-scale'),
               'v1', 'scale-facts-v1', 'RESOLVED',
               'scale-facts-v1', 'scale-facts-v1', 1
          FROM (
            SELECT 's' || lpad(g::text, 4, '0') AS key
              FROM generate_series(0, 499) AS g
            UNION ALL
            SELECT 'o' || lpad(g::text, 4, '0')
              FROM generate_series(0, 4999) AS g
          ) AS entities;

        INSERT INTO graph_meta.relationship
          (project_id, src_id, dst_id, rel_type, resolution, confidence,
           fact_id, revision, projection_seq)
        SELECT f.project_id, src.id, dst.id, 'CALLS', f.resolution, f.confidence,
               f.id, f.revision_id::text, 1
          FROM code.fact AS f
          JOIN graph_meta.entity AS src
            ON src.project_id = f.project_id
           AND src.entity_type = 'Method'
           AND src.natural_key = f.subject_key
          JOIN graph_meta.entity AS dst
            ON dst.project_id = f.project_id
           AND dst.entity_type = 'Method'
           AND dst.natural_key = f.object_key
         WHERE f.project_id = '{PROJECT_ID}'
           AND f.revision_id = '{REVISION_ID}'
           AND f.valid_to IS NULL;

        INSERT INTO graph_meta.entity
          (id, project_id, entity_type, natural_key, name, qualified_name,
           properties, ontology_version, revision, resolution,
           first_seen_rev, last_seen_rev, projection_seq)
        SELECT id, '{PROJECT_ID}', 'Method', natural_key, natural_key, natural_key,
               jsonb_build_object('source', 'phase5-performance-chain'),
               'v1', 'scale-facts-v1', 'RESOLVED',
               'scale-facts-v1', 'scale-facts-v1', 1
          FROM (VALUES
            ('45555555-5555-4555-8555-555555555550'::uuid, 'scale.impact.hop0'),
            ('45555555-5555-4555-8555-555555555551'::uuid, 'scale.impact.hop1'),
            ('45555555-5555-4555-8555-555555555552'::uuid, 'scale.impact.hop2'),
            ('45555555-5555-4555-8555-555555555553'::uuid, 'scale.impact.hop3'),
            ('45555555-5555-4555-8555-555555555554'::uuid, 'scale.impact.hop4'),
            ('45555555-5555-4555-8555-555555555555'::uuid, 'scale.impact.hop5')
          ) AS hops(id, natural_key)
        ON CONFLICT (project_id, entity_type, natural_key) DO NOTHING;

        INSERT INTO graph_meta.relationship
          (id, project_id, src_id, dst_id, rel_type, resolution, confidence,
           revision, projection_seq)
        VALUES
          ('46666666-6666-4666-8666-666666666660', '{PROJECT_ID}',
           '45555555-5555-4555-8555-555555555550',
           '45555555-5555-4555-8555-555555555551',
           'CALLS', 'RESOLVED', 0.98, 'scale-facts-v1', 1),
          ('46666666-6666-4666-8666-666666666661', '{PROJECT_ID}',
           '45555555-5555-4555-8555-555555555551',
           '45555555-5555-4555-8555-555555555552',
           'CALLS', 'RESOLVED', 0.98, 'scale-facts-v1', 1),
          ('46666666-6666-4666-8666-666666666662', '{PROJECT_ID}',
           '45555555-5555-4555-8555-555555555552',
           '45555555-5555-4555-8555-555555555553',
           'CALLS', 'RESOLVED', 0.98, 'scale-facts-v1', 1),
          ('46666666-6666-4666-8666-666666666663', '{PROJECT_ID}',
           '45555555-5555-4555-8555-555555555553',
           '45555555-5555-4555-8555-555555555554',
           'CALLS', 'RESOLVED', 0.98, 'scale-facts-v1', 1),
          ('46666666-6666-4666-8666-666666666664', '{PROJECT_ID}',
           '45555555-5555-4555-8555-555555555554',
           '45555555-5555-4555-8555-555555555555',
           'CALLS', 'RESOLVED', 0.98, 'scale-facts-v1', 1);

        INSERT INTO corpus.evidence
          (id, project_id, source_type, locator, quote, quote_hash, resolution, confidence)
        SELECT id, '{PROJECT_ID}', 'CODE',
               jsonb_build_object(
                 'kind', 'code',
                 'path', 'scale/impact-hop-' || hop || '.java',
                 'byte_start', 0,
                 'byte_end', 32,
                 'line', 1
               ),
               'scale impact hop ' || hop,
               sha256(('scale impact hop ' || hop)::bytea),
               'RESOLVED',
               0.98
          FROM generate_series(0, 5) AS hop
          CROSS JOIN LATERAL (
            SELECT ('47777777-7777-4777-8777-77777777777' || hop)::uuid AS id
          ) AS evidence_ids;

        INSERT INTO code.fact
          (id, project_id, revision_id, subject_kind, subject_key, relation,
           object_kind, object_key, resolution, confidence, analysis_engine,
           engine_version, fact_hash)
        SELECT ('48888888-8888-4888-8888-88888888888' || hop)::uuid,
               '{PROJECT_ID}', '{REVISION_ID}', 'Method',
               'scale.impact.hop' || (hop - 1), 'CALLS', 'Method',
               'scale.impact.hop' || hop, 'RESOLVED', 0.98,
               'phase5-performance', '1', decode('00', 'hex')
          FROM generate_series(1, 5) AS hop;

        INSERT INTO code.fact_evidence (project_id, fact_id, evidence_id, role)
        SELECT '{PROJECT_ID}',
               ('48888888-8888-4888-8888-88888888888' || hop)::uuid,
               ('47777777-7777-4777-8777-77777777777' || (hop - 1))::uuid,
               'PRIMARY'
          FROM generate_series(1, 5) AS hop;

        ANALYZE code.fact;
        ANALYZE graph_meta.entity;
        ANALYZE graph_meta.relationship;
        """
    )
    return time.monotonic() - started


def neo4j_counts() -> tuple[int, int]:
    output = cypher(
        f"MATCH (e:Entity {{project_id: '{PROJECT_ID}'}}) "
        f"OPTIONAL MATCH (e)-[r]->(:Entity {{project_id: '{PROJECT_ID}'}}) "
        "RETURN count(DISTINCT e) AS nodes, count(DISTINCT r) AS rels"
    )
    numbers = re.findall(r"\d+", output.splitlines()[-1])
    if len(numbers) < 2:
        raise RuntimeError(f"cannot parse Neo4j counts: {output}")
    return int(numbers[-2]), int(numbers[-1])


def impact_six_hop() -> dict:
    query = urllib.parse.urlencode(
        {
            "q": "scale.impact.hop0",
            "entity_type": "Method",
            "limit": 20,
        }
    )
    entities = call(
        "GET",
        f"/projects/{PROJECT_ID}/graph/entities?{query}",
        timeout=120,
    )["items"]
    seed = next(
        (
            entity
            for entity in entities
            if (entity.get("qualified_name") or entity["name"]) == "scale.impact.hop0"
        ),
        None,
    )
    if seed is None:
        raise RuntimeError("6-hop performance seed entity is missing")

    started = time.monotonic()
    accepted = call(
        "POST",
        f"/projects/{PROJECT_ID}/impacts",
        {
            "seed": {"kind": "ENTITY", "entity_ids": [seed["id"]]},
            "options": {
                "max_depth": 6,
                "hub_threshold": 200,
                "max_items": 500,
                "include_documents": False,
            },
        },
        timeout=120,
    )
    terminal = {"SUCCEEDED", "SUCCEEDED_WITH_ERRORS", "FAILED", "CANCELLED", "TIMEOUT"}
    for _ in range(480):
        job = call("GET", f"/jobs/{accepted['job_id']}", timeout=120)
        if job["status"] in terminal:
            if job["status"] not in {"SUCCEEDED", "SUCCEEDED_WITH_ERRORS"}:
                raise RuntimeError(f"impact job failed: {job}")
            break
        time.sleep(0.25)
    else:
        raise TimeoutError("impact job did not finish")
    response = call(
        "GET",
        f"/impacts/{accepted['impact_id']}",
        timeout=120,
    )
    seconds = time.monotonic() - started
    report = response.get("report")
    if report is None:
        raise RuntimeError(f"impact report missing: {response}")
    keys = {item["natural_key"] for item in report["items"]}
    required = {f"scale.impact.hop{index}" for index in range(6)}
    if not required.issubset(keys):
        raise RuntimeError(f"6-hop traversal missed nodes: {sorted(required - keys)}")
    if any(not item.get("evidence") for item in report["items"]):
        raise RuntimeError("6-hop performance chain contains evidence-free items")
    return {
        "impact_id": accepted["impact_id"],
        "seconds": round(seconds, 3),
        "items": report["stats"]["total"],
        "max_depth": max(item["depth"] for item in report["items"]),
        "passed": seconds < 120.0,
    }


def main() -> int:
    report: dict = {}
    passed = False
    try:
        seed()
        call("POST", "/auth/login", {"login_id": LOGIN_ID, "password": PASSWORD})

        print("COPY Fact-equivalent rows", flush=True)
        copy_seconds = copy_facts()
        persisted = int(
            psql(
                f"SELECT count(*) FROM code.fact "
                f"WHERE project_id = '{PROJECT_ID}' AND revision_id = '{REVISION_ID}';"
            )
        )

        print("Apply supersede_then_insert increment", flush=True)
        supersede_seconds = supersede_increment()
        totals = psql(
            f"""
            SELECT
              count(*) FILTER (WHERE valid_to IS NULL),
              count(*) FILTER (WHERE valid_to IS NOT NULL),
              count(*) FILTER (WHERE superseded_by IS NOT NULL),
              count(*)
              FROM code.fact
             WHERE project_id = '{PROJECT_ID}' AND revision_id = '{REVISION_ID}';
            """
        )
        live, superseded, linked, total = [int(value) for value in totals.split("|")]

        print("Build PostgreSQL graph projection", flush=True)
        graph_pg_seconds = build_pg_graph()
        pg_graph = psql(
            f"""
            SELECT
              (SELECT count(*) FROM graph_meta.entity
                WHERE project_id = '{PROJECT_ID}' AND deleted_at IS NULL),
              (SELECT count(*) FROM graph_meta.relationship
                WHERE project_id = '{PROJECT_ID}' AND valid_to IS NULL);
            """
        )
        pg_entities, pg_relationships = [int(value) for value in pg_graph.split("|")]

        print("Rebuild Neo4j projection", flush=True)
        started = time.monotonic()
        rebuilt = call(
            "POST",
            f"/projects/{PROJECT_ID}/graph/rebuild",
            timeout=7200,
        )
        neo4j_seconds = time.monotonic() - started
        neo_entities, neo_relationships = neo4j_counts()
        impact_performance = impact_six_hop()

        passed = (
            persisted == FACT_COUNT
            and live == FACT_COUNT
            and superseded == SUPERSEDE_COUNT
            and linked == SUPERSEDE_COUNT
            and total == FACT_COUNT + SUPERSEDE_COUNT
            and pg_entities == neo_entities == rebuilt["entities"]
            and pg_relationships == neo_relationships == rebuilt["relationships"]
            and impact_performance["passed"]
            and impact_performance["max_depth"] == 5
        )
        report = {
            "project_id": PROJECT_ID,
            "fact_count_requested": FACT_COUNT,
            "fact_count_persisted": persisted,
            "copy_batch_rows": COPY_BATCH,
            "copy_seconds": round(copy_seconds, 3),
            "copy_rows_per_second": round(persisted / copy_seconds, 2),
            "copy_statements": FACT_COUNT // COPY_BATCH,
            "supersede_requested": SUPERSEDE_COUNT,
            "supersede_seconds": round(supersede_seconds, 3),
            "live_facts": live,
            "historical_facts": superseded,
            "superseded_by_links": linked,
            "total_fact_rows": total,
            "pg_graph_build_seconds": round(graph_pg_seconds, 3),
            "pg_graph_entities": pg_entities,
            "pg_graph_relationships": pg_relationships,
            "neo4j_reprojection_seconds": round(neo4j_seconds, 3),
            "neo4j_entities": neo_entities,
            "neo4j_relationships": neo_relationships,
            "impact_six_hop": impact_performance,
            "graph_rebuild_response": rebuilt,
            "passed": passed,
            "limitations": [
                "This benchmark persists a Fact-equivalent synthetic data set with COPY; it does not generate 2.5M Facts through source analysis.",
                "The synthetic rows intentionally omit corpus evidence rows so the COPY number isolates code.fact persistence.",
                "OpenRewrite, Java 21 semantic analysis, and a customer classpath are external dependencies and are not exercised.",
                "Neo4j reprojection uses the production batched /graph/rebuild API after constructing the matching PostgreSQL graph projection.",
                "The 6-hop timing uses a low-degree six-node chain embedded in the 2.5M-edge graph; unrelated high-degree nodes exercise hub truncation rather than full graph loading.",
            ],
        }
        REPORT.parent.mkdir(parents=True, exist_ok=True)
        REPORT.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        print(json.dumps(report, ensure_ascii=False, indent=2), flush=True)
    finally:
        cleanup()
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
