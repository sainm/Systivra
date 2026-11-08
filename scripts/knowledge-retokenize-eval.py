#!/usr/bin/env python3
"""保存済み chunk の日本語再 tokenize を実測する。

前提: API が起動済み（既定 http://127.0.0.1:18086）。
  SYSTIVRA_BASE_URL=http://127.0.0.1:18086 PGDATABASE=systivra_final_verify \
    python3 scripts/knowledge-retokenize-eval.py

日本語形態素（Lindera / IPADIC）は実装済みだが、**既に保存された `tokenized`
は古いまま**で、`fts`（生成列）も追随していない。本文に `住所` が在っても
語彙一致検索が 0 件になる。`POST /knowledge/retokenize` で直ることを確かめる。
"""

from __future__ import annotations

import http.cookiejar
import json
import os
import pathlib
import subprocess
import sys
import urllib.error
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[1]
BASE_URL = os.environ.get("SYSTIVRA_BASE_URL", "http://127.0.0.1:18086")
DB = os.environ.get("PGDATABASE", "systivra")
PROJECT_ID = "22222222-2222-4222-8222-222222222222"

cookies = http.cookiejar.CookieJar()
opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(cookies))
REPORT: dict = {}


def psql(sql: str) -> str:
    result = subprocess.run(
        ["psql", "-d", DB, "-v", "ON_ERROR_STOP=1", "-tA", "-c", sql],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(f"psql failed: {result.stderr.strip()}")
    return result.stdout.strip()


def call(method: str, path: str, body: dict | None = None):
    data = json.dumps(body, ensure_ascii=False).encode("utf-8") if body else None
    headers = {"content-type": "application/json"} if body else {}
    request = urllib.request.Request(
        f"{BASE_URL}/api/v1{path}", data=data, headers=headers, method=method
    )
    try:
        with opener.open(request, timeout=30) as response:
            raw = response.read()
            return response.status, json.loads(raw) if raw else None
    except urllib.error.HTTPError as error:
        raw = error.read()
        try:
            return error.code, json.loads(raw) if raw else None
        except json.JSONDecodeError:
            return error.code, {"raw": raw.decode("utf-8", "replace")}


def search(query: str) -> dict:
    status, body = call(
        "POST",
        f"/projects/{PROJECT_ID}/knowledge/search",
        {"query": query, "mode": "lexical", "top_k": 5},
    )
    assert status == 200, (status, body)
    return body


def main() -> int:
    status, _ = call(
        "POST", "/auth/login", {"login_id": "alice", "password": "SystivraDemo1!"}
    )
    assert status == 200, status

    # 日本語を含む現在の chunk が対象。無ければこの検証は成立しない。
    total = int(
        psql(
            "SELECT count(*) FROM corpus.chunk "
            f"WHERE project_id = '{PROJECT_ID}' AND is_current "
            "AND text LIKE '%住所%';"
        )
    )
    assert total >= 1, "日本語を含む chunk が無い"

    # 古い索引を再現する（ASCII だけの tokenized に戻す）。
    psql(
        "UPDATE corpus.chunk SET tokenized = "
        "'UserService save UserEditAction update AddressService validate' "
        f"WHERE project_id = '{PROJECT_ID}' AND is_current "
        "AND text LIKE '%住所%';"
    )
    fts_hit = psql(
        "SELECT count(*) FROM corpus.chunk "
        f"WHERE project_id = '{PROJECT_ID}' AND is_current "
        "AND fts @@ to_tsquery('simple', '住所');"
    )
    assert fts_hit == "0", fts_hit
    before = search("住所")
    assert len(before["results"]) == 0, before

    # 保守操作: 現行 tokenizer で作り直す。
    status, rebuilt = call(
        "POST", f"/projects/{PROJECT_ID}/knowledge/retokenize"
    )
    assert status == 200, (status, rebuilt)
    assert rebuilt["updated"] >= 1, rebuilt
    assert rebuilt["remaining"] == 0, rebuilt

    fts_after = psql(
        "SELECT count(*) FROM corpus.chunk "
        f"WHERE project_id = '{PROJECT_ID}' AND is_current "
        "AND fts @@ to_tsquery('simple', '住所');"
    )
    assert fts_after != "0", fts_after
    after = search("住所")
    assert len(after["results"]) >= 1, after

    # もう一度呼んでも変化しない（冪等）。
    status, again = call(
        "POST", f"/projects/{PROJECT_ID}/knowledge/retokenize"
    )
    assert status == 200, (status, again)
    assert again["updated"] == 0, again
    # 更新済みの行を何度も読み直さないこと。ここが壊れると remaining が
    # 減らず、呼び出し側が永久にループする。
    assert again["examined"] == 0, again
    assert again["remaining"] == 0, again

    REPORT.update(
        {
            "before_hits": len(before["results"]),
            "rebuilt": rebuilt,
            "after_hits": len(after["results"]),
            "fts_before": int(fts_hit),
            "fts_after": int(fts_after),
            "idempotent_updated": again["updated"],
        }
    )
    out = ROOT / "data" / "phase2" / "retokenize.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(REPORT, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(REPORT, ensure_ascii=False, indent=2))
    print(f"\nwritten: {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
