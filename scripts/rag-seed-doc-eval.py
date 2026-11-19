#!/usr/bin/env python3
"""RAG シードの端到端受入（文書 → コード実体）。

前提: 001〜032 適用済み DB、API + worker、`cupsfilter`（macOS 標準）。
  SYSTIVRA_BASE_URL=http://127.0.0.1:18089 PGDATABASE=systivra_phase6_verify \
    python3 scripts/rag-seed-doc-eval.py

流れ:
  1. コード実体名（Hello）を含む設計書をテキストから PDF にして取り込む
  2. 索引を構築する
  3. **Hello を含まない**言い換えの質問を投げ、RAG 側が Hello を候補にすること
  4. 影響評価が 1 件以上の項目を返すこと

文書だけの DB では候補が出ない（Document はコードへ辿れない）ので、
この受入は「コード実体があるプロジェクト」でだけ意味を持つ。
"""

from __future__ import annotations

import http.cookiejar
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[1]
BASE = os.environ.get("SYSTIVRA_BASE_URL", "http://127.0.0.1:18089")
DB = os.environ.get("PGDATABASE", "systivra")
PROJECT = os.environ.get("SYSTIVRA_PROJECT_ID", "22222222-2222-4222-8222-222222222222")

OPENER = urllib.request.build_opener(
    urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar())
)

DOC_TEXT = """第1章 挨拶機能の設計

本章では、挨拶文を組み立てる処理について説明する。

本処理は Hello クラスの責務である。greet メソッドは受け取った名前に対して
定型の挨拶文を返す。呼び出し元は Caller であり、画面 JSP からも利用される。

注意事項として、挨拶文の組み立てに失敗した場合は空文字を返し、
例外を送出しない。ログには対象の名前だけを残す。
"""

QUESTION = "挨拶を組み立てる処理はどこにありますか"


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


def wait_job(job_id: str) -> dict:
    terminal = {"SUCCEEDED", "SUCCEEDED_WITH_ERRORS", "FAILED", "CANCELLED"}
    for _ in range(600):
        _, job = call("GET", f"/jobs/{job_id}")
        if job.get("status") in terminal:
            if job["status"] not in {"SUCCEEDED", "SUCCEEDED_WITH_ERRORS"}:
                raise RuntimeError(f"job {job_id} ended as {job['status']}: {job.get('errors')}")
            return job
        time.sleep(0.25)
    raise TimeoutError(f"job {job_id} did not finish")


def make_pdf(text: str) -> bytes:
    """cupsfilter でテキストから PDF を作る（外部フォント不要）。"""
    with tempfile.TemporaryDirectory() as tmp:
        source = pathlib.Path(tmp) / "design.txt"
        source.write_text(text, encoding="utf-8")
        result = subprocess.run(
            ["cupsfilter", "-m", "application/pdf", str(source)],
            capture_output=True,
        )
        if result.returncode != 0 or not result.stdout.startswith(b"%PDF"):
            raise RuntimeError(f"cupsfilter failed: {result.stderr[:200]!r}")
        return result.stdout


def upload(data: bytes, filename: str) -> str:
    _, created = call(
        "POST",
        f"/projects/{PROJECT}/documents:upload-url",
        {"filename": filename, "size_bytes": len(data), "mime_type": "application/pdf"},
    )
    url = created["upload_url"]
    if url.startswith("/"):
        url = BASE + url
    request = urllib.request.Request(
        url, data=data, method="PUT", headers={"Content-Type": "application/octet-stream"}
    )
    OPENER.open(request, timeout=120).read()
    return created["object_key"]


def main() -> int:
    status, _ = call("POST", "/auth/login", {"login_id": "alice", "password": "SystivraDemo1!"})
    assert status == 200, status

    pdf = make_pdf(DOC_TEXT)
    object_key = upload(pdf, "rag-seed-design.pdf")
    status, registered = call(
        "POST",
        f"/projects/{PROJECT}/documents",
        {
            "object_key": object_key,
            "doc_type": "BASIC_DESIGN",
            "title": "RAG seed design",
            "mime_type": "application/pdf",
        },
    )
    assert status in (200, 201), (status, registered)
    document_id = registered["document"]["id"]
    revision_id = registered["document"]["current_revision"]["id"]
    status, accepted = call(
        "POST",
        f"/documents/{document_id}/revisions/{revision_id}:parse",
        {},
    )
    assert status in (200, 202), (status, accepted)
    wait_job(accepted["job_id"])

    status, build = call(
        "POST", f"/projects/{PROJECT}/index-builds", {"model": "local-hash-1024"}
    )
    assert status in (200, 202), (status, build)
    wait_job(build["job_id"])
    build_id = build["build_id"] if "build_id" in build else build.get("id")
    if build_id:
        call("POST", f"/projects/{PROJECT}/index-builds/{build_id}:activate", {})

    # 言い換えた質問（Hello を含まない）。名前一致では当たらない。
    status, inquiry = call(
        "POST",
        f"/projects/{PROJECT}/change-inquiries",
        {"question": QUESTION, "input": {"kind": "NATURAL_LANGUAGE"}},
    )
    assert status in (200, 202), (status, inquiry)
    assert inquiry["status"] != "FAILED", inquiry
    detail = call("GET", f"/projects/{PROJECT}/change-inquiries/{inquiry['inquiry_id']}")[1]
    candidates = detail.get("candidates") or []
    labels = [candidate["label"] for candidate in candidates]
    assert any("Hello" in label for label in labels), f"Hello が候補に無い: {labels}"

    # 候補が複数なら設計どおり `NEEDS_CLARIFICATION` で止まる（勝手に 1 つへ絞らない）。
    # ここでは利用者が Hello を選んだ場合を再現し、影響が出るところまで通す。
    impact_id = inquiry.get("impact_id")
    if not impact_id:
        chosen = next(
            candidate for candidate in candidates if "Hello" in candidate["label"]
        )
        status, selected = call(
            "POST",
            f"/projects/{PROJECT}/change-inquiries",
            {
                "question": QUESTION,
                "input": {"kind": "ENTITY", "entity_ids": [chosen["entity_id"]]},
            },
        )
        assert status in (200, 202), (status, selected)
        if selected.get("job_id"):
            wait_job(selected["job_id"])
        impact_id = selected.get("impact_id")
        assert impact_id, selected
    if impact_id:
        report = call("GET", f"/impacts/{impact_id}")[1].get("report") or {}
        items = len(report.get("items") or [])
        assert items >= 1, f"影響項目が 0 件: {report.get('degraded')}"
    else:
        items = 0

    # 後始末。受入で作った文書を残すと、他の受入の検索結果が変わる。
    status, _ = call("DELETE", f"/documents/{document_id}")
    assert status in (200, 204), status

    result = {
        "question": QUESTION,
        "first_status": inquiry["status"],
        "candidates": labels,
        "impact_items": items,
    }
    out = ROOT / "data" / "phase5-eval" / "rag-seed-doc.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(result, ensure_ascii=False, indent=2))
    print(f"\nwritten: {out}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as error:  # noqa: BLE001 - 受入は失敗内容をそのまま出す
        print(f"FAILED: {error}", file=sys.stderr)
        raise
