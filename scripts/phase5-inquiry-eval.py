#!/usr/bin/env python3
"""Phase 5 残: Q-01 変更影響問合せの受入。

前提: 001〜017 適用済み DB、API（既定 18086）、worker 起動済み。
グラフのシードは最小 fixture を psql で投入する（実コード解析は Phase 4 の受入で別途）。

  PGHOST=localhost PGUSER=<admin> PGDATABASE=systivra_phase6_verify \
  SYSTIVRA_BASE_URL=http://127.0.0.1:18086 python3 scripts/phase5-inquiry-eval.py
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
BASE_URL = os.environ.get("SYSTIVRA_BASE_URL", "http://127.0.0.1:18086")
LOGIN_ID = os.environ.get("SYSTIVRA_LOGIN_ID", "alice")
PASSWORD = os.environ.get("SYSTIVRA_PASSWORD", "SystivraDemo1!")
DB = os.environ.get("PGDATABASE", "systivra")
PROJECT_ID = os.environ.get("SYSTIVRA_PROJECT_ID")
REPORT: dict = {}

SEED = "e3e3e3e3-e3e3-4e3e-8e3e-e3e3e3e3e3e3"
TARGET = "e4e4e4e4-e4e4-4e4e-8e4e-e4e4e4e4e4e4"
DUP1 = "e1e1e1e1-e1e1-4e1e-8e1e-e1e1e1e1e1e1"
DUP2 = "e2e2e2e2-e2e2-4e2e-8e2e-e2e2e2e2e2e2"
REPO = "e5e5e5e5-e5e5-4e5e-8e5e-e5e5e5e5e5e5"
REVISION = "e6e6e6e6-e6e6-4e6e-8e6e-e6e6e6e6e6e6"
FACT = "e7e7e7e7-e7e7-4e7e-8e7e-e7e7e7e7e7e7"
EVIDENCE = "e8e8e8e8-e8e8-4e8e-8e8e-e8e8e8e8e8e8"


class Client:
    def __init__(self) -> None:
        self.jar = http.cookiejar.CookieJar()
        self.opener = urllib.request.build_opener(
            urllib.request.HTTPCookieProcessor(self.jar)
        )

    def call(self, method: str, path: str, body: dict | None = None):
        data = None
        headers = {}
        if body is not None:
            data = json.dumps(body, ensure_ascii=False).encode("utf-8")
            headers["content-type"] = "application/json"
        request = urllib.request.Request(
            f"{BASE_URL}/api/v1{path}", data=data, headers=headers, method=method
        )
        try:
            with self.opener.open(request, timeout=30) as response:
                raw = response.read()
                return response.status, json.loads(raw) if raw else None
        except urllib.error.HTTPError as error:
            raw = error.read()
            try:
                parsed = json.loads(raw) if raw else None
            except json.JSONDecodeError:
                parsed = {"raw": raw.decode("utf-8", "replace")}
            return error.code, parsed

    def must(self, method: str, path: str, body: dict | None = None) -> dict:
        status, payload = self.call(method, path, body)
        if status >= 400:
            raise AssertionError(f"{method} {path} -> HTTP {status}: {payload}")
        return payload or {}

    def raw(
        self, path: str, headers: dict[str, str] | None = None
    ) -> tuple[int, str, str]:
        request = urllib.request.Request(
            f"{BASE_URL}/api/v1{path}", headers=headers or {}
        )
        with self.opener.open(request, timeout=30) as response:
            return (
                response.status,
                response.read().decode("utf-8"),
                response.headers.get("content-type", ""),
            )

    def raw_bytes(self, path: str) -> tuple[int, bytes, str]:
        request = urllib.request.Request(f"{BASE_URL}/api/v1{path}")
        with self.opener.open(request, timeout=60) as response:
            return (
                response.status,
                response.read(),
                response.headers.get("content-type", ""),
            )

    def sse(
        self, path: str, last_event_id: int | None = None
    ) -> tuple[str, list[dict]]:
        headers = {}
        if last_event_id is not None:
            headers["Last-Event-ID"] = str(last_event_id)
        request = urllib.request.Request(
            f"{BASE_URL}/api/v1{path}", headers=headers
        )
        with self.opener.open(request, timeout=30) as response:
            content_type = response.headers.get("content-type", "")
            text = response.read().decode("utf-8")
        events = []
        for block in text.split("\n\n"):
            event = "message"
            event_id = None
            data = []
            for line in block.splitlines():
                if not line or line.startswith(":"):
                    continue
                if line.startswith("event:"):
                    event = line[6:].strip()
                elif line.startswith("id:"):
                    event_id = int(line[3:].strip())
                elif line.startswith("data:"):
                    data.append(line[5:].lstrip())
            if data:
                events.append(
                    {"id": event_id, "event": event, "data": json.loads("\n".join(data))}
                )
        return content_type, events


def psql(sql: str) -> str:
    result = subprocess.run(
        ["psql", "-d", DB, "-v", "ON_ERROR_STOP=1", "-tA", "-c", sql],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(f"psql failed: {result.stderr.strip()}")
    return result.stdout.strip()


def wait_job(client: Client, job_id: str) -> dict:
    terminal = {"SUCCEEDED", "SUCCEEDED_WITH_ERRORS", "FAILED", "CANCELLED", "TIMEOUT"}
    for _ in range(240):
        job = client.must("GET", f"/jobs/{job_id}")
        if job["status"] in terminal:
            if job["status"] in {"FAILED", "CANCELLED", "TIMEOUT"}:
                raise AssertionError(f"job {job_id} ended as {job['status']}: {job.get('errors')}")
            return job
        time.sleep(0.25)
    raise TimeoutError(f"job {job_id} did not finish")


def seed_fixture(pid: str) -> None:
    psql(
        "DELETE FROM graph_meta.relationship WHERE revision = 'inquiry';"
        "DELETE FROM graph_meta.entity WHERE revision = 'inquiry';"
        "DELETE FROM impact.change_inquiry;"
    )
    psql(
        f"""
        INSERT INTO graph_meta.entity
          (id, project_id, entity_type, natural_key, name, properties, ontology_version,
           revision, resolution, first_seen_rev, last_seen_rev, projection_seq)
        VALUES
          ('{SEED}', '{pid}', 'Method', 'jp.example.web.UserEditAction', 'UserEditAction',
           '{{}}', 'v1', 'inquiry', 'RESOLVED', 'inquiry', 'inquiry', 1),
          ('{TARGET}', '{pid}', 'Class', 'jp.example.service.UserService', 'UserService',
           '{{}}', 'v1', 'inquiry', 'RESOLVED', 'inquiry', 'inquiry', 1),
          ('{DUP1}', '{pid}', 'Method', 'jp.example.a.m', 'A.m',
           '{{}}', 'v1', 'inquiry', 'INFERRED', 'inquiry', 'inquiry', 1),
          ('{DUP2}', '{pid}', 'Method', 'jp.example.c.m', 'A.m',
           '{{}}', 'v1', 'inquiry', 'INFERRED', 'inquiry', 'inquiry', 1)
        ON CONFLICT (project_id, entity_type, natural_key) DO UPDATE
          SET name = EXCLUDED.name, deleted_at = NULL;
        """
    )
    psql(
        f"""
        INSERT INTO graph_meta.relationship
          (id, project_id, src_id, dst_id, rel_type, resolution, confidence, revision, projection_seq)
        SELECT
          'e9e9e9e9-e9e9-4e9e-8e9e-e9e9e9e9e9e9', '{pid}',
          src.id,
          dst.id,
          'CALLS', 'RESOLVED', 0.9, 'inquiry', 1
          FROM graph_meta.entity src
          JOIN graph_meta.entity dst
            ON dst.project_id = '{pid}'
           AND dst.entity_type = 'Class'
           AND dst.natural_key = 'jp.example.service.UserService'
         WHERE src.project_id = '{pid}'
           AND src.entity_type = 'Method'
           AND src.natural_key = 'jp.example.web.UserEditAction'
        ON CONFLICT (project_id, src_id, dst_id, rel_type, revision) DO NOTHING;
        """
    )
    psql(
        f"""
        INSERT INTO code.repository (id, project_id, kind, url)
        VALUES ('{REPO}', '{pid}', 'DIR', 'repositories/inquiry')
        ON CONFLICT DO NOTHING;
        INSERT INTO code.code_revision
          (id, project_id, repository_id, revision, tree_digest, snapshot_key)
        VALUES ('{REVISION}', '{pid}', '{REPO}', 'r1', '\\x01', 'code/inquiry/r1')
        ON CONFLICT DO NOTHING;
        """
    )
    psql(
        f"""
        INSERT INTO corpus.evidence
          (id, project_id, source_type, locator, quote, quote_hash, resolution, confidence)
        VALUES ('{EVIDENCE}', '{pid}', 'CODE',
                '{{"path":"src/UserEditAction.java","byte_start":0,"byte_end":24}}',
                'class UserEditAction {{}}', sha256('class UserEditAction {{}}'::bytea),
                'RESOLVED', 0.9)
        ON CONFLICT DO NOTHING;
        INSERT INTO code.fact
          (id, project_id, revision_id, subject_kind, subject_key, relation,
           object_kind, object_key, resolution, confidence, analysis_engine,
           engine_version, fact_hash)
        VALUES ('{FACT}', '{pid}', '{REVISION}', 'Method', 'jp.example.web.UserEditAction',
                'CALLS', 'Class', 'jp.example.service.UserService', 'RESOLVED', 0.95,
                'tree-sitter', 'inquiry', '\\x03')
        ON CONFLICT DO NOTHING;
        INSERT INTO code.fact_evidence (project_id, fact_id, evidence_id, role)
        VALUES ('{pid}', '{FACT}', '{EVIDENCE}', 'PRIMARY')
        ON CONFLICT DO NOTHING;
        """
    )


def run_inquiry(client: Client, pid: str, body: dict) -> tuple[dict, float]:
    started = time.monotonic()
    accepted = client.must("POST", f"/projects/{pid}/change-inquiries", body)
    if accepted.get("job_id"):
        wait_job(client, accepted["job_id"])
    view = client.must(
        "GET", f"/projects/{pid}/change-inquiries/{accepted['inquiry_id']}"
    )
    return view, time.monotonic() - started


def main() -> int:
    login = Client()
    me = login.must("POST", "/auth/login", {"login_id": LOGIN_ID, "password": PASSWORD})
    pid = PROJECT_ID or next(
        (
            item["project_id"]
            for item in me["memberships"]
            if item.get("project_code") == "PRJ-DEMO"
        ),
        me["memberships"][0]["project_id"],
    )
    seed_fixture(pid)

    # ── 1. ENTITY 入力で 5 セクション ──────────────────────
    entity, entity_seconds = run_inquiry(
        login,
        pid,
        {
            "question": "UserEditAction の変更影響",
            "input": {"kind": "ENTITY", "entity_ids": [SEED]},
            "mode": "FULL",
        },
    )
    assert entity_seconds < 8.0, entity_seconds
    assert entity["status"] == "SUCCEEDED", entity
    assert entity["summary"], entity
    assert entity["summary"]["approval_state"] == "DRAFT"
    assert entity["summary"]["code_modified"] is False
    assert entity["summary"]["tests_executed"] is False
    counts = entity["summary"]["counts"]
    assert counts["code"] == entity["code"]["total"], (counts, entity["code"]["total"])
    assert counts["screens"] == entity["screens"]["total"]
    assert counts["documents"] == entity["documents"]["total"]
    assert counts["tests"] == (
        len(entity["tests"]["existing"]) + len(entity["tests"]["drafts"])
    )
    assert entity["code"]["total"] >= 1, entity["code"]
    for section_name in ["screens", "documents", "code"]:
        for item in entity[section_name]["items"]:
            assert item["why"], (section_name, item)
            assert item["resolution"], (section_name, item)
            assert item["path"], (section_name, item)
            assert item["quote"], (section_name, item)
            assert item["locator"], (section_name, item)
    first = entity["code"]["items"][0]
    assert first["why"], "why が空"
    assert first["resolution"], "resolution が空"
    assert first["path"], "path が空"
    assert first["quote"], "quote が空"
    assert first["locator"], "locator が空"
    assert entity["tests"] is not None
    REPORT["entity"] = {
        "status": entity["status"],
        "headline": entity["summary"]["headline"],
        "counts": counts,
        "code_first": {
            "label": first["label"],
            "why": first["why"],
            "path": first["path"],
            "quote": first["quote"],
        },
        "existing_tests": len(entity["tests"]["existing"]),
        "draft_tests": len(entity["tests"]["drafts"]),
        "degraded": entity["summary"]["degraded"],
        "summary_seconds": round(entity_seconds, 3),
    }

    # ── 1.5 Q-01 から計画へ（test_run は作らない） ─────────
    derived = login.must(
        "POST",
        f"/projects/{pid}/change-inquiries/{entity['inquiry_id']}/test-plans:derive",
        {"name": "Q-01 から派生した計画", "max_drafts": 20},
    )
    assert derived["test_run_created"] is False, derived
    assert derived["plan_id"], derived
    assert derived["draft_case_count"] > 0, derived
    plans = login.must("GET", f"/projects/{pid}/test/overview")["plans"]
    assert any(plan["id"] == derived["plan_id"] and plan["status"] == "DRAFT" for plan in plans)
    REPORT["derive_plan"] = {
        "plan_id": derived["plan_id"],
        "draft_case_count": derived["draft_case_count"],
        "test_run_created": derived["test_run_created"],
    }

    # ── 1.6 計画に紐づく run は、その計画のケースだけを見る ──
    scoped = login.must(
        "POST",
        f"/projects/{pid}/test/runs",
        {
            "name": "q01-plan-run",
            "plan_id": derived["plan_id"],
            "environment": {
                "id": "env-q01-01",
                "base_url": "http://127.0.0.1:1/app",
                "guard_kind": "STAGING",
                "allowed_origins": ["127.0.0.1"],
                "parallel": 1,
            },
        },
    )
    # 計画のケースはすべて DRAFT。承認前なので 0 件が正しい。
    assert scoped["case_count"] == 0, scoped
    REPORT["plan_scoped_run"] = {
        "case_count": scoped["case_count"],
        "reason": "plan cases are DRAFT until approved",
    }

    # ── 1.7 Markdown 导出 ─────────────────────────────────
    md_status, md_text, md_type = login.raw(
        f"/projects/{pid}/change-inquiries/{entity['inquiry_id']}/report.md"
    )
    assert md_status == 200 and "text/markdown" in md_type, (md_status, md_type)
    for marker in ["## どんな影響か", "## 影響するコード", "| 名前 |", "追加候補"]:
        assert marker in md_text, marker
    # AI 未設定なので叙述セクションは出ない。
    assert "AI の要約" not in md_text
    impact_status, impact_md, impact_type = login.raw(
        f"/impacts/{entity['impact_id']}/report.md"
    )
    assert impact_status == 200 and "text/markdown" in impact_type
    for marker in ["# 変更影響レポート", "## 影響項目", "### 証拠"]:
        assert marker in impact_md, marker
    impact_narrate = login.must("POST", f"/impacts/{entity['impact_id']}/narrate")
    assert impact_narrate["narrative"] is None, impact_narrate
    assert any("ai_unavailable" in reason for reason in impact_narrate["degraded"])
    impact_after = login.must("GET", f"/impacts/{entity['impact_id']}")
    assert impact_after["report"]["narrative"] is None, impact_after["report"].get("narrative")
    REPORT["impact_narrate"] = {
        "narrative": impact_narrate["narrative"],
        "degraded": impact_narrate["degraded"],
        "items_unchanged": impact_after["report"]["stats"]["total"],
    }

    REPORT["markdown"] = {
        "inquiry_bytes": len(md_text.encode("utf-8")),
        "impact_bytes": len(impact_md.encode("utf-8")),
    }
    pdf_status, inquiry_pdf, inquiry_pdf_type = login.raw_bytes(
        f"/projects/{pid}/change-inquiries/{entity['inquiry_id']}/report.pdf"
    )
    impact_pdf_status, impact_pdf, impact_pdf_type = login.raw_bytes(
        f"/impacts/{entity['impact_id']}/report.pdf"
    )
    assert pdf_status == 200 and "application/pdf" in inquiry_pdf_type
    assert impact_pdf_status == 200 and "application/pdf" in impact_pdf_type
    assert inquiry_pdf.startswith(b"%PDF-") and len(inquiry_pdf) > 10_000
    assert impact_pdf.startswith(b"%PDF-") and len(impact_pdf) > 10_000
    REPORT["pdf"] = {
        "inquiry_bytes": len(inquiry_pdf),
        "impact_bytes": len(impact_pdf),
        "font": os.environ.get(
            "SYSTIVRA_PDF_FONT",
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        ),
    }

    # ── 1.8 AI 未設定なら叙述を作らない（5 段は残る） ──────
    authoritative_before = {
        "summary": {
            "counts": entity["summary"]["counts"],
            "risk": entity["summary"]["risk"],
            "approval_state": entity["summary"]["approval_state"],
            "code_modified": entity["summary"]["code_modified"],
            "tests_executed": entity["summary"]["tests_executed"],
        },
        "screens": entity["screens"],
        "documents": entity["documents"],
        "code": entity["code"],
        "tests": entity["tests"],
    }
    narrate = login.must(
        "POST",
        f"/projects/{pid}/change-inquiries/{entity['inquiry_id']}/narrate",
    )
    assert narrate["narrative"] is None, narrate
    assert narrate["hypotheses"] == [], narrate
    assert any("ai_unavailable" in reason for reason in narrate["degraded"]), narrate
    reloaded = login.must(
        "GET", f"/projects/{pid}/change-inquiries/{entity['inquiry_id']}"
    )
    assert "ai_unavailable" in reloaded["summary"]["degraded"], reloaded["summary"]
    assert reloaded["summary"].get("narrative") is None, reloaded["summary"]
    assert reloaded["summary"]["hypotheses"] == [], reloaded["summary"]
    authoritative_after = {
        "summary": {
            "counts": reloaded["summary"]["counts"],
            "risk": reloaded["summary"]["risk"],
            "approval_state": reloaded["summary"]["approval_state"],
            "code_modified": reloaded["summary"]["code_modified"],
            "tests_executed": reloaded["summary"]["tests_executed"],
        },
        "screens": reloaded["screens"],
        "documents": reloaded["documents"],
        "code": reloaded["code"],
        "tests": reloaded["tests"],
    }
    assert authoritative_after == authoritative_before, (
        authoritative_before,
        authoritative_after,
    )
    REPORT["narrate"] = {
        "narrative": narrate["narrative"],
        "hypotheses": narrate["hypotheses"],
        "degraded": narrate["degraded"],
        "sections_still_present": bool(reloaded["code"]["items"]),
        "authoritative_unchanged": True,
    }

    # ── 2. SSE の順序と Last-Event-ID 再開 ─────────────────
    content_type, events = login.sse(
        f"/projects/{pid}/change-inquiries/{entity['inquiry_id']}/events"
    )
    assert content_type.startswith("text/event-stream"), content_type
    names = [event["event"] for event in events]
    ids = [event["id"] for event in events]
    assert ids == list(range(1, len(ids) + 1)), ids
    assert names[:-1] == ["partial"] * 5, names
    assert names[-1] == "done", names
    sections = [event["data"]["section"] for event in events[:-1]]
    assert sections == ["summary", "screens", "documents", "code", "tests"], sections
    _, resumed = login.sse(
        f"/projects/{pid}/change-inquiries/{entity['inquiry_id']}/events",
        last_event_id=3,
    )
    assert [event["id"] for event in resumed] == ids[3:]
    assert [event["event"] for event in resumed] == names[3:]
    REPORT["events"] = {
        "content_type": content_type,
        "names": names,
        "sections": sections,
        "ids": ids,
        "resume_from_3": [event["event"] for event in resumed],
    }

    # ── 3. 自然言語: 一意に決まる ─────────────────────────
    natural, natural_seconds = run_inquiry(
        login,
        pid,
        {
            "question": "UserEditAction を変更したらどこに影響する？",
            "input": {"kind": "NATURAL_LANGUAGE"},
        },
    )
    assert natural_seconds < 8.0, natural_seconds
    assert natural["status"] == "SUCCEEDED", natural
    REPORT["natural_language"] = {
        "status": natural["status"],
        "impact_id": natural["impact_id"],
        "summary_seconds": round(natural_seconds, 3),
    }

    # ── 4. 曖昧なら止まる（NEEDS_CLARIFICATION） ──────────
    ambiguous = login.must(
        "POST",
        f"/projects/{pid}/change-inquiries",
        {
            "question": "A.m を変更したい",
            "input": {"kind": "NATURAL_LANGUAGE"},
        },
    )
    assert ambiguous["status"] == "NEEDS_CLARIFICATION", ambiguous
    assert ambiguous["impact_id"] is None
    detail = login.must(
        "GET", f"/projects/{pid}/change-inquiries/{ambiguous['inquiry_id']}"
    )
    assert len(detail["candidates"]) >= 2, detail["candidates"]
    assert all(candidate["reason"] for candidate in detail["candidates"])
    REPORT["needs_clarification"] = {
        "status": ambiguous["status"],
        "candidates": len(detail["candidates"]),
    }

    # ── 5. 見つからなければ FAILED / NOT_FOUND_SEED ────────
    missing = login.must(
        "POST",
        f"/projects/{pid}/change-inquiries",
        {"question": "存在しない画面Zzzを変更したい", "input": {"kind": "NATURAL_LANGUAGE"}},
    )
    assert missing["status"] == "FAILED", missing
    detail = login.must(
        "GET", f"/projects/{pid}/change-inquiries/{missing['inquiry_id']}"
    )
    assert detail["error"]["code"] == "NOT_FOUND_SEED", detail["error"]
    REPORT["not_found_seed"] = detail["error"]["code"]

    # ── 6. 冪等: 同じ質問は同じ inquiry_id ─────────────────
    again = login.must(
        "POST",
        f"/projects/{pid}/change-inquiries",
        {
            "question": "UserEditAction を変更したらどこに影響する？",
            "input": {"kind": "NATURAL_LANGUAGE"},
        },
    )
    assert again["inquiry_id"] == natural["inquiry_id"], (again, natural["inquiry_id"])
    REPORT["idempotent"] = True

    # ── 7. Git unified diff 入力 ───────────────────────────
    revisions = login.must(
        "GET", f"/projects/{pid}/code/revisions?limit=20"
    )["items"]
    head = next(item for item in revisions if item["file_count"] == 12)
    field_patch = """\
diff --git a/src/main/java/jp/example/action/UserEditAction.java b/src/main/java/jp/example/action/UserEditAction.java
--- a/src/main/java/jp/example/action/UserEditAction.java
+++ b/src/main/java/jp/example/action/UserEditAction.java
@@ -6 +6 @@
-    private UserService userService;
+    private UserService userService; // inquiry changed field
"""
    diff_view, diff_seconds = run_inquiry(
        login,
        pid,
        {
            "question": "UserEditAction.userService の変更影響",
            "input": {
                "kind": "DIFF",
                "head_revision_id": head["id"],
                "diff_unified": field_patch,
            },
            "mode": "FULL",
        },
    )
    assert diff_view["status"] == "SUCCEEDED", diff_view
    assert diff_view["code"]["total"] > 0
    assert all(
        item["why"] and item["quote"] and item["locator"]
        for item in diff_view["code"]["items"]
    )
    REPORT["diff"] = {
        "status": diff_view["status"],
        "summary_seconds": round(diff_seconds, 3),
        "code_items": diff_view["code"]["total"],
    }

    out = ROOT / "data" / "phase5-inquiry" / "report.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(REPORT, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(REPORT, ensure_ascii=False, indent=2))
    print(f"\nwritten: {out}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as error:  # noqa: BLE001
        print(f"FAILED: {error}", file=sys.stderr)
        raise
