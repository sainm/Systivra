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

    # ── 3. 未実装の戦略は明示される
    status, impact = call(
        "POST",
        f"/projects/{PROJECT}/retrieval:query",
        {"query": "入力チェックを変えたら影響は？", "top_k": 3},
    )
    assert status == 200, status
    assert impact["selected_strategy"] == "GLOBAL", impact["selected_strategy"]
    assert any(
        item.startswith("strategy_not_implemented:CROSS_DOCUMENT")
        for item in impact["degraded"]
    ), impact["degraded"]
    assert "未実装" in impact["strategy_reason"], impact["strategy_reason"]
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

    report["degraded"] = retrieval["degraded"]
    out = ROOT / "data" / "retrieval-eval" / "phase2.json"
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
