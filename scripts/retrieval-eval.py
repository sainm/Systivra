#!/usr/bin/env python3
"""Retrieval Engine（Phase 2）の受入。

  SYSTIVRA_BASE_URL=http://127.0.0.1:18086 PGDATABASE=systivra_phase6_verify \
    python3 scripts/retrieval-eval.py

確かめること:
  1. **同じ並び**: `/knowledge/search` と `/retrieval:query`（GLOBAL）が同じ chunk 順を返す
     — 新しい入口が検索の意味論を変えていないことの唯一の証拠
  2. 戦略の自動選択（規則）と、その理由が返ること
  3. 未実装の戦略は **別物を返さず** `degraded` に明示されること
  4. DOCUMENT_LEVEL は種別で実際に絞ること（黙って全件返さない）
  5. trace（run + stage）が残ること
"""

from __future__ import annotations

import http.cookiejar
import json
import os
import pathlib
import sys
import urllib.error
import urllib.request

BASE = os.environ.get("SYSTIVRA_BASE_URL", "http://127.0.0.1:18086")
PROJECT = os.environ.get("SYSTIVRA_PROJECT_ID", "22222222-2222-4222-8222-222222222222")
ROOT = pathlib.Path(__file__).resolve().parents[1]
OPENER = urllib.request.build_opener(
    urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar())
)


def call(method: str, path: str, body: dict | None = None) -> tuple[int, dict]:
    data = json.dumps(body, ensure_ascii=False).encode("utf-8") if body is not None else None
    request = urllib.request.Request(f"{BASE}/api/v1{path}", data=data, method=method)
    if body is not None:
        request.add_header("content-type", "application/json")
    try:
        with OPENER.open(request, timeout=60) as response:
            raw = response.read()
            return response.status, (json.loads(raw) if raw else {})
    except urllib.error.HTTPError as error:
        raw = error.read()
        try:
            return error.code, (json.loads(raw) if raw else {})
        except json.JSONDecodeError:
            return error.code, {"raw": raw.decode("utf-8", "replace")}



def psql(database: str, sql: str) -> str:
    """受入で使う最小の psql 呼び出し（関係の投入と後始末だけ）。"""
    import subprocess

    result = subprocess.run(
        ["psql", "-d", database, "-tAc", sql], capture_output=True, text=True
    )
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip())
    return result.stdout.strip()


def phase3(database: str) -> dict:
    """跨文档扩展: 明示関係 → 信頼度フィルタ → 上限での打ち切り。"""
    rows = psql(
        database,
        "SELECT id, node_path[2] FROM corpus.chunk WHERE is_current "
        "AND node_path[2] IN ('SECTION:20','SECTION:53','SECTION:83') "
        "ORDER BY node_path[2] LIMIT 3",
    ).splitlines()
    assert len(rows) == 3, rows
    src = rows[0].split("|")[0]
    mid = rows[1].split("|")[0]
    far = rows[2].split("|")[0]

    base = (
        "INSERT INTO corpus.chunk_relation "
        "(project_id, src_chunk_id, dst_chunk_id, relation, confidence, method, note) VALUES "
        f"('{PROJECT}','{src}','{far}','IMPLEMENTS',0.9,'MANUAL','eval'),"
        f"('{PROJECT}','{src}','{mid}','RELATED_TO',0.3,'RULE','eval') "
        "ON CONFLICT DO NOTHING"
    )
    psql(database, base)
    try:
        query = "検証キー-20-顧客管理"

        def run(min_conf: float, max_related: int) -> dict:
            status, body = call(
                "POST",
                f"/projects/{PROJECT}/retrieval:query",
                {
                    "query": query,
                    "strategy": "CROSS_DOCUMENT",
                    "top_k": 3,
                    "graph": {
                        "depth": 1,
                        "direction": "BOTH",
                        "relationship_types": [],
                        "max_related": max_related,
                        "min_confidence": min_conf,
                    },
                },
            )
            assert status == 200, (status, body)
            return body

        strict = run(0.5, 20)
        relations = {item["relation"] for item in strict.get("relationships", [])}
        assert relations == {"IMPLEMENTS"}, relations
        vias = {chunk["chunk_id"]: chunk.get("via") for chunk in strict["chunks"]}
        assert vias.get(far, {}) and vias[far]["relation"] == "IMPLEMENTS", vias
        assert mid not in vias, "信頼度 0.3 の関係が 0.5 の下限を通過している"

        loose = run(0.2, 20)
        loose_relations = {item["relation"] for item in loose.get("relationships", [])}
        assert loose_relations == {"IMPLEMENTS", "RELATED_TO"}, loose_relations

        truncated = run(0.2, 1)
        graph_stage = next(s for s in truncated["stages"] if s["stage"] == "graph")
        assert graph_stage["candidates"] == 2 and graph_stage["kept"] == 1, graph_stage
        assert any(item.startswith("graph_truncated:") for item in truncated["degraded"]), (
            truncated["degraded"]
        )
        return {
            "strict_relations": sorted(relations),
            "loose_relations": sorted(loose_relations),
            "truncation_marked": True,
        }
    finally:
        psql(
            database,
            f"DELETE FROM corpus.chunk_relation WHERE project_id = '{PROJECT}' AND note = 'eval'",
        )


def main() -> int:
    status, _ = call("POST", "/auth/login", {"login_id": "alice", "password": "SystivraDemo1!"})
    assert status == 200, status

    report: dict = {}
    query = "顧客管理の入力チェック"

    # ── 1. 並びが同じであること
    _, search = call(
        "POST",
        f"/projects/{PROJECT}/knowledge/search",
        {"query": query, "mode": "hybrid", "top_k": 5, "rerank": True, "include": ["quote"]},
    )
    status, retrieval = call(
        "POST", f"/projects/{PROJECT}/retrieval:query", {"query": query, "top_k": 5}
    )
    assert status == 200, (status, retrieval)
    search_ids = [result["chunk_id"] for result in search.get("results", [])]
    retrieval_ids = [chunk["chunk_id"] for chunk in retrieval.get("chunks", [])]
    assert search_ids == retrieval_ids, (
        "既存の /search と並びが違う（意味論が変わっている）",
        search_ids,
        retrieval_ids,
    )
    report["same_order"] = True
    report["returned"] = len(retrieval_ids)

    # ── 2. 戦略の自動選択
    assert retrieval["selected_strategy"] == "GLOBAL", retrieval["selected_strategy"]
    assert retrieval["strategy_reason"], "理由が空"
    report["auto_strategy"] = retrieval["selected_strategy"]

    # ── 3a. 影響を問う質問は CROSS_DOCUMENT で実行される（Phase 3 で実装済み）
    status, impact = call(
        "POST",
        f"/projects/{PROJECT}/retrieval:query",
        {"query": "入力チェックを変えたら影響は？", "top_k": 3},
    )
    assert status == 200, status
    assert impact["selected_strategy"] == "CROSS_DOCUMENT", impact["selected_strategy"]
    assert not any(
        item.startswith("strategy_not_implemented:CROSS_DOCUMENT")
        for item in impact["degraded"]
    ), impact["degraded"]
    assert any(stage["stage"] == "graph" for stage in impact["stages"]), impact["stages"]
    report["cross_document_selected"] = True

    # ── 3b. 未実装の戦略（HIERARCHICAL）は別物を返さず明示する
    status, hier = call(
        "POST",
        f"/projects/{PROJECT}/retrieval:query",
        {"query": "この章の内容を教えて", "strategy": "HIERARCHICAL", "top_k": 3},
    )
    assert status == 200, status
    assert hier["selected_strategy"] == "GLOBAL", hier["selected_strategy"]
    assert any(
        item.startswith("strategy_not_implemented:HIERARCHICAL") for item in hier["degraded"]
    ), hier["degraded"]
    assert "未実装" in hier["strategy_reason"], hier["strategy_reason"]
    report["unimplemented_marked"] = True

    # ── 4. DOCUMENT_LEVEL は実際に絞る
    status, by_type = call(
        "POST",
        f"/projects/{PROJECT}/retrieval:query",
        {"query": "顧客検索の詳細設計を確認したい", "top_k": 3},
    )
    assert status == 200, status
    assert by_type["selected_strategy"] == "DOCUMENT_LEVEL", by_type["selected_strategy"]
    doc_types = {source["doc_type"] for source in by_type.get("sources", [])}
    assert doc_types <= {"DETAILED_DESIGN"} or not doc_types, doc_types
    report["document_level"] = {"chunks": len(by_type.get("chunks", [])), "types": sorted(doc_types)}

    # ── 5. trace（run + stage）
    status, runs = call("GET", f"/projects/{PROJECT}/retrieval-runs?limit=5")
    assert status == 200, status
    assert runs["items"], "実行記録が無い"
    stages = retrieval.get("stages", [])
    assert [stage["stage"] for stage in stages] == ["search", "context"], stages
    assert retrieval["context"]["tokens"] > 0
    report["trace"] = {"runs": len(runs["items"]), "stages": [s["stage"] for s in stages]}

    database = os.environ.get("PGDATABASE", "systivra_phase6_verify")
    report["cross_document"] = phase3(database)

    report["degraded"] = retrieval["degraded"]
    out = ROOT / "data" / "retrieval-eval" / "report.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False, indent=2))
    print(f"\nwritten: {out}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except AssertionError as error:
        print(f"FAILED: {error}", file=sys.stderr)
        raise
