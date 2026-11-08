#!/usr/bin/env python3
"""Phase 2 改版管理の E2E（改版登録 / 切替 / 版指定検索 / graph tombstone）。

前提: 001〜029 適用済み DB（既定 systivra_phase6_verify）、API（既定 18088）、
Neo4j（bolt://localhost:7687）。worker は本スクリプトが起動する。

  PGHOST=localhost PGUSER=sainm PGDATABASE=systivra_phase6_verify \
  SYSTIVRA_BASE_URL=http://127.0.0.1:18088 python3 scripts/phase2-revision-eval.py
"""

from __future__ import annotations

import importlib.util
import json
import pathlib
import re
import subprocess
import sys
import time
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location(
    "p6failure", ROOT / "scripts" / "phase6-failure-eval.py"
)
p6 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(p6)

BASE = p6.BASE_URL
DB = p6.DB
FIXTURE = ROOT / "tests" / "fixtures" / "walking_skeleton_100p.pdf"
REPORT_DIR = ROOT / "data" / "phase2-revision"
NEO4J = [
    "cypher-shell", "-a", "bolt://localhost:7687", "-u", "neo4j", "-p", "neo4j",
    "--format", "plain",
]


def cypher(query: str) -> list[str]:
    result = subprocess.run([*NEO4J, query], capture_output=True, text=True, timeout=60)
    if result.returncode != 0:
        raise RuntimeError(f"cypher failed: {result.stderr.strip()}")
    return [line for line in result.stdout.strip().splitlines() if line.strip()]


def neo_revision_count(pid: str, revision: str) -> int:
    rows = cypher(
        "MATCH (e:Entity {project_id: '" + pid + "', revision: '" + revision + "'}) "
        "RETURN count(e)"
    )
    return int(rows[-1])


def upload(pid: str, data: bytes, filename: str, mime: str) -> str:
    created = p6.call(
        "POST",
        f"/projects/{pid}/documents:upload-url",
        {"filename": filename, "size_bytes": len(data), "mime_type": mime},
    )
    url = created["upload_url"]
    if url.startswith("/"):
        url = BASE + url
    request = urllib.request.Request(
        url,
        data=data,
        method="PUT",
        headers={"Content-Type": "application/octet-stream"},
    )
    p6.opener.open(request, timeout=120).read()
    return created["object_key"]


def parse(did: str, rid: str) -> dict:
    request = urllib.request.Request(
        f"{BASE}/api/v1/documents/{did}/revisions/{rid}:parse",
        data=b"",
        method="POST",
        headers={"idempotency-key": f"rev-eval-{rid}"},
    )
    with p6.opener.open(request, timeout=60) as response:
        raw = response.read()
        return json.loads(raw) if raw else {}


def wait_parse(rid: str, timeout: float = 240) -> str:
    deadline = time.time() + timeout
    while time.time() < deadline:
        status = p6.psql(
            "SELECT status FROM corpus.parse_run "
            f"WHERE revision_id='{rid}' ORDER BY started_at DESC LIMIT 1;"
        )
        if status in ("SUCCEEDED", "FAILED", "CANCELLED", "TIMEOUT"):
            return status
        time.sleep(0.5)
    raise RuntimeError("parse timeout")


def search(pid: str, query: str, revision: str | None = None) -> list[dict]:
    body = {
        "query": query,
        "mode": "hybrid",
        "top_k": 20,
        "filters": {"revision_id": revision} if revision else {},
    }
    return p6.call("POST", f"/projects/{pid}/knowledge/search", body)["results"]


def pending(pid: str) -> int:
    return int(p6.psql(
        "SELECT count(*) FROM core.outbox_event WHERE published_at IS NULL "
        f"AND NOT dead_letter AND payload->>'project_id' = '{pid}';"
    ))


def start_worker() -> subprocess.Popen:
    env = dict(p6.os.environ)
    env.update({
        "DATABASE_URL": f"postgres://systivra_scheduler@localhost:5432/{DB}",
        "WORKER_TICK_SEC": "1",
        "SYSTIVRA_DATA_DIR": str(ROOT / "data"),
        "NEO4J_URI": "bolt://localhost:7687",
        "NEO4J_USER": "neo4j",
        "NEO4J_PASSWORD": "neo4j",
        "WORKER_ID": "revision-eval",
    })
    env.pop("NEO4J_DISABLED", None)
    return subprocess.Popen(
        [str(ROOT / "target" / "debug" / "systivra-worker")],
        cwd=str(ROOT), env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
    )


def cleanup(pid: str) -> None:
    docs = p6.psql("SELECT id FROM corpus.document WHERE title LIKE 'REV-EVAL-%';").splitlines()
    p6.psql("DELETE FROM core.job WHERE kind = 'PARSE' AND status IN ('PENDING','QUEUED');")
    for did in docs:
        p6.psql(
            "DELETE FROM corpus.evidence WHERE revision_id IN "
            f"(SELECT id FROM corpus.document_revision WHERE document_id='{did}');"
        )
        p6.psql(f"DELETE FROM corpus.chunk WHERE document_id='{did}';")
        p6.psql(
            "DELETE FROM corpus.doc_node WHERE revision_id IN "
            f"(SELECT id FROM corpus.document_revision WHERE document_id='{did}');"
        )
        p6.psql(
            "DELETE FROM corpus.parse_run WHERE revision_id IN "
            f"(SELECT id FROM corpus.document_revision WHERE document_id='{did}');"
        )
        p6.psql(f"DELETE FROM corpus.document_revision WHERE document_id='{did}';")
        p6.psql(f"DELETE FROM corpus.document WHERE id='{did}';")
        p6.psql(
            "DELETE FROM graph_meta.relationship WHERE project_id = "
            f"'{pid}' AND (src_id IN (SELECT id FROM graph_meta.entity "
            f"WHERE properties->>'document_id' = '{did}') OR dst_id IN "
            f"(SELECT id FROM graph_meta.entity WHERE properties->>'document_id' = '{did}'));"
        )
        p6.psql(
            f"DELETE FROM graph_meta.entity WHERE properties->>'document_id' = '{did}';"
        )
        cypher(
            "MATCH (e:Entity) WHERE e.properties_document_id = '" + did + "' DETACH DELETE e"
        )


def wait_for(condition, timeout: float, what: str):
    deadline = time.time() + timeout
    while time.time() < deadline:
        value = condition()
        if value:
            return value
        time.sleep(0.3)
    raise RuntimeError(f"timeout: {what}")


def main() -> int:
    REPORT_DIR.mkdir(parents=True, exist_ok=True)
    login = p6.call("POST", "/auth/login", {"login_id": p6.LOGIN_ID, "password": p6.PASSWORD})
    pid = login["memberships"][0]["project_id"]
    cleanup(pid)
    title = f"REV-EVAL-{int(time.time())}"
    data1 = FIXTURE.read_bytes()
    data2 = data1 + b"\n%REV-EVAL-2\n"

    worker = start_worker()
    results: dict = {}
    try:
        object1 = upload(pid, data1, "rev-eval-v1.pdf", "application/pdf")
        registered = p6.call(
            "POST",
            f"/projects/{pid}/documents",
            {
                "object_key": object1,
                "doc_type": "BASIC_DESIGN",
                "title": title,
                "mime_type": "application/pdf",
                "source_revision": "v1",
            },
        )
        did = registered["document"]["id"]
        rid1 = registered["document"]["current_revision"]["id"]
        parse(did, rid1)
        status = wait_parse(rid1)
        assert status == "SUCCEEDED", status

        object2 = upload(pid, data2, "rev-eval-v2.pdf", "application/pdf")
        second = p6.call(
            "POST",
            f"/projects/{pid}/documents/{did}/revisions",
            {
                "object_key": object2,
                "doc_type": "BASIC_DESIGN",
                "title": title,
                "mime_type": "application/pdf",
                "source_revision": "v2",
            },
        )
        rid2 = second["revision"]["id"]
        parse(did, rid2)
        status = wait_parse(rid2)
        assert status == "SUCCEEDED", status

        history = p6.call("GET", f"/documents/{did}/revisions")["items"]
        statuses = {item["id"]: item["status"] for item in history}
        assert statuses == {rid1: "SUPERSEDED", rid2: "CURRENT"}, statuses
        current_flags = p6.psql(
            "SELECT revision_id::text || ':' || is_current FROM corpus.chunk "
            f"WHERE document_id='{did}' ORDER BY revision_id, ordinal;"
        ).splitlines()
        assert all(
            (rid2 + ":true") in line or (rid1 + ":false") in line
            for line in current_flags
        ), current_flags[:5]

        texts = p6.psql(
            "SELECT replace(text, E'\\n', ' ') FROM corpus.chunk "
            f"WHERE revision_id='{rid1}' ORDER BY char_length(text) DESC LIMIT 5;"
        ).splitlines()
        query = ""
        for line in texts:
            candidate = re.sub(r"[^\wぁ-んァ-ヶ一-龠々ー]", "", line)[:10]
            if candidate:
                query = candidate
                break
        assert query, texts[:1]
        default_hits = search(pid, query)
        assert default_hits and all(hit["revision"]["id"] == rid2 for hit in default_hits), [
            hit["revision"]["id"] for hit in default_hits
        ]
        old_hits = search(pid, query, rid1)
        assert old_hits and all(hit["revision"]["id"] == rid1 for hit in old_hits), [
            hit["revision"]["id"] for hit in old_hits
        ]
        results["before_switch"] = {
            "statuses": statuses,
            "default_revision": rid2,
            "default_hits": len(default_hits),
            "revision_filter_hits": len(old_hits),
        }

        wait_for(lambda: pending(pid) == 0, 60, "projection drain (v2)")
        assert neo_revision_count(pid, rid2) > 0

        activated = p6.call(
            "POST", f"/projects/{pid}/documents/{did}/revisions/{rid1}:activate"
        )["items"]
        flipped = {item["id"]: item["status"] for item in activated}
        assert flipped == {rid1: "CURRENT", rid2: "SUPERSEDED"}, flipped
        default_hits = search(pid, query)
        assert default_hits and all(hit["revision"]["id"] == rid1 for hit in default_hits)
        flags = p6.psql(
            "SELECT revision_id::text || ':' || is_current FROM corpus.chunk "
            f"WHERE document_id='{did}';"
        ).splitlines()
        assert all(
            (rid1 + ":true") in line or (rid2 + ":false") in line for line in flags
        ), flags[:5]

        wait_for(lambda: pending(pid) == 0, 60, "projection drain (activate)")
        neo_old = neo_revision_count(pid, rid2)
        neo_new = neo_revision_count(pid, rid1)
        tombstoned = int(p6.psql(
            "SELECT count(*) FROM graph_meta.entity "
            f"WHERE revision='{rid2}' AND deleted_at IS NOT NULL;"
        ))
        superseded_events = int(p6.psql(
            "SELECT count(*) FROM core.outbox_event "
            "WHERE event_type='DocumentRevisionSuperseded' AND published_at IS NOT NULL;"
        ))
        audit = int(p6.psql(
            "SELECT count(*) FROM core.audit_log "
            f"WHERE action='document.revision.activate' AND target_id='{did}';"
        ))
        assert neo_old == 0, neo_old
        assert neo_new > 0, neo_new
        assert tombstoned > 0, tombstoned
        assert superseded_events >= 1, superseded_events
        assert audit == 1, audit

        content = p6.opener.open(
            f"{BASE}/api/v1/documents/{did}/revisions/{rid1}/content", timeout=60
        ).read()
        assert content == data1

        results["after_switch"] = {
            "statuses": flipped,
            "default_revision": rid1,
            "neo4j_old_revision_nodes": neo_old,
            "neo4j_new_revision_nodes": neo_new,
            "pg_tombstoned_entities": tombstoned,
            "superseded_events_published": superseded_events,
            "audit_rows": audit,
            "old_content_bytes": len(content),
        }
        results["document"] = {"id": did, "title": title, "revision_current": rid1}
    finally:
        worker.terminate()
        try:
            worker.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            worker.kill()
            worker.communicate()

    out = REPORT_DIR / "report.json"
    out.write_text(json.dumps(results, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(results, ensure_ascii=False, indent=2))
    print(f"\nwritten: {out}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as error:  # noqa: BLE001
        print(f"FAILED: {error}", file=sys.stderr)
        raise