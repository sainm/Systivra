#!/usr/bin/env python3
import argparse
import json
import statistics
import time
import urllib.error
import urllib.request
import uuid
import zipfile
from datetime import datetime
from http.cookiejar import CookieJar
from pathlib import Path

import openpyxl
from openpyxl.styles import Font

RUN = datetime.now().strftime("%H%M%S")
SHEETS = 50


def write_xlsx(path: Path) -> list[dict]:
    wb = openpyxl.Workbook()
    wb.remove(wb.active)
    rows = []
    for i in range(1, SHEETS + 1):
        ws = wb.create_sheet(f"設計書{i:02d}")
        ws.merge_cells("A1:C1")
        ws["A1"] = f"画面仕様{i:02d} {RUN}"
        ws["A1"].font = Font(bold=True)
        for col, value in enumerate(["項目名", "入力形式", "必須"], start=1):
            ws.cell(row=2, column=col, value=value).font = Font(bold=True)
        ws.append([f"住所{i:02d}", "全角20文字以内", "必須"])
        ws.append([f"識別子{i:02d}", f"U{RUN}-{i:04d}", "一意"])
        note = f"髙橋{RUN}-{i:02d}" if 41 <= i <= 45 else f"検証語{RUN}-{i:02d}"
        ws.append([f"備考{i:02d}", note, "任意"])
        rows.append({"sheet": f"設計書{i:02d}", "rows": [3, 5]})
    wb.save(path)
    return rows


def write_docx(path: Path) -> None:
    body = []
    for j in range(1, 6):
        body.append(
            f'<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>DOC見出し{j} {RUN}</w:t></w:r></w:p>'
            f'<w:p><w:r><w:t>DOC識別子 D{RUN}-{j:03d} を定義する。</w:t></w:r></w:p>'
        )
    document = (
        '<?xml version="1.0"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>'
        + "".join(body)
        + "</w:body></w:document>"
    )
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr(
            "[Content_Types].xml",
            '<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>',
        )
        z.writestr("word/document.xml", document)


def write_pptx(path: Path) -> None:
    slides = []
    for j in range(1, 6):
        slides.append(
            '<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" '
            'xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree>'
            f'<p:sp><p:nvSpPr><p:cNvPr id="2" name="Title {j}"/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>'
            f'<p:txBody><a:p><a:r><a:t>PPTスライド{j} {RUN}</a:t></a:r></a:p></p:txBody></p:sp>'
            f'<p:sp><p:nvSpPr><p:cNvPr id="3" name="Body {j}"/><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr>'
            f'<p:txBody><a:p><a:r><a:t>PPT識別子 P{RUN}-{j:03d} を定義する。</a:t></a:r></a:p></p:txBody></p:sp>'
            "</p:spTree></p:cSld></p:sld>"
        )
    presentation = (
        '<?xml version="1.0"?><p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" '
        'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:sldIdLst>'
        + "".join(
            f'<p:sldId id="{255 + j}" r:id="rId{j}"/>' for j in range(1, 6)
        )
        + "</p:sldIdLst></p:presentation>"
    )
    rels = (
        '<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        + "".join(
            f'<Relationship Id="rId{j}" Type="slide" Target="slides/slide{j}.xml"/>'
            for j in range(1, 6)
        )
        + "</Relationships>"
    )
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr(
            "[Content_Types].xml",
            '<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>',
        )
        z.writestr("ppt/presentation.xml", presentation)
        z.writestr("ppt/_rels/presentation.xml.rels", rels)
        for j in range(1, 6):
            z.writestr(f"ppt/slides/slide{j}.xml", slides[j - 1])


def generate(corpus_dir: Path) -> tuple[list[dict], list[dict]]:
    corpus_dir.mkdir(parents=True, exist_ok=True)
    xlsx = corpus_dir / "eval-50sheets.xlsx"
    docx = corpus_dir / "eval.docx"
    pptx = corpus_dir / "eval.pptx"
    write_xlsx(xlsx)
    write_docx(docx)
    write_pptx(pptx)
    titles = {
        "xlsx": f"Phase2 Eval XLSX {RUN}",
        "docx": f"Phase2 Eval DOCX {RUN}",
        "pptx": f"Phase2 Eval PPTX {RUN}",
    }
    manifest = [
        {
            "path": xlsx.name,
            "doc_type": "DETAILED_DESIGN",
            "business_domain": "phase2-eval",
            "title": titles["xlsx"],
        },
        {
            "path": docx.name,
            "doc_type": "DETAILED_DESIGN",
            "business_domain": "phase2-eval",
            "title": titles["docx"],
        },
        {
            "path": pptx.name,
            "doc_type": "DETAILED_DESIGN",
            "business_domain": "phase2-eval",
            "title": titles["pptx"],
        },
    ]
    (corpus_dir / "manifest.jsonl").write_text(
        "\n".join(json.dumps(line, ensure_ascii=False) for line in manifest) + "\n",
        encoding="utf-8",
    )

    queries = []
    for i in range(1, 21):
        queries.append(
            {
                "category": "word",
                "query": f"住所{i:02d} 全角20文字以内",
                "expected": {"kind": "xlsx", "title": titles["xlsx"], "sheet": f"設計書{i:02d}"},
            }
        )
    for i in range(21, 41):
        queries.append(
            {
                "category": "partial",
                "query": f"証語{RUN}-{i:02d}",
                "expected": {"kind": "xlsx", "title": titles["xlsx"], "sheet": f"設計書{i:02d}"},
            }
        )
    for i in range(41, 51):
        value = f"髙橋{RUN}-{i:02d}" if i <= 45 else f"U{RUN}-{i:04d}"
        queries.append(
            {
                "category": "identifier",
                "query": value,
                "expected": {"kind": "xlsx", "title": titles["xlsx"], "sheet": f"設計書{i:02d}"},
            }
        )
    for j in range(1, 6):
        queries.append(
            {
                "category": "identifier",
                "query": f"D{RUN}-{j:03d}",
                "expected": {
                    "kind": "docx",
                    "title": titles["docx"],
                    "section": f"DOC見出し{j} {RUN}",
                },
            }
        )
    for j in range(1, 6):
        queries.append(
            {
                "category": "identifier",
                "query": f"P{RUN}-{j:03d}",
                "expected": {"kind": "pptx", "title": titles["pptx"], "slide": j},
            }
        )

    cells = []
    for i in range(1, 31):
        row = 3 + (i - 1) % 3
        value = (
            f"住所{i:02d}" if row == 3 else f"U{RUN}-{i:04d}" if row == 4 else f"検証語{RUN}-{i:02d}"
        )
        cells.append(
            {
                "query": value,
                "expected": {
                    "kind": "xlsx",
                    "title": titles["xlsx"],
                    "sheet": f"設計書{i:02d}",
                    "row": row,
                },
            }
        )
    return queries, cells


class Client:
    def __init__(self, base: str):
        self.base = base.rstrip("/")
        self.opener = urllib.request.build_opener(
            urllib.request.HTTPCookieProcessor(CookieJar())
        )

    def request(self, method: str, path: str, body=None):
        data = None
        headers = {}
        if body is not None:
            data = json.dumps(body, ensure_ascii=False).encode("utf-8")
            headers["content-type"] = "application/json"
        request = urllib.request.Request(
            f"{self.base}{path}", data=data, headers=headers, method=method
        )
        with self.opener.open(request, timeout=60) as response:
            payload = response.read()
            return json.loads(payload) if payload else None

    def login(self, login: str, password: str):
        return self.request("POST", "/api/v1/auth/login", {"login_id": login, "password": password})

    def bulk_import(self, project: str, manifest_url: str):
        return self.request(
            "POST",
            f"/api/v1/projects/{project}/documents:bulk-import",
            {"manifest_url": manifest_url, "mode": "FULL", "on_duplicate": "SKIP"},
        )

    def job(self, job_id: str):
        return self.request("GET", f"/api/v1/jobs/{job_id}")

    def documents(self, project: str, limit: int = 100):
        return self.request("GET", f"/api/v1/projects/{project}/documents?limit={limit}")

    def search(self, project: str, query: str, top_k: int = 10):
        return self.request(
            "POST",
            f"/api/v1/projects/{project}/knowledge/search",
            {
                "query": query,
                "mode": "hybrid",
                "filters": {},
                "top_k": top_k,
                "include": ["locator", "quote"],
            },
        )


def wait_job(client: Client, job_id: str, timeout: float = 120.0) -> dict:
    deadline = time.time() + timeout
    while time.time() < deadline:
        job = client.job(job_id)
        if job["status"] in {"SUCCEEDED", "SUCCEEDED_WITH_ERRORS", "FAILED", "CANCELLED", "TIMEOUT"}:
            return job
        time.sleep(0.5)
    raise TimeoutError(f"job {job_id} did not finish")


def wait_documents(client: Client, project: str, titles: set[str], timeout: float = 300.0) -> None:
    deadline = time.time() + timeout
    while time.time() < deadline:
        items = client.documents(project)["items"]
        matched = [item for item in items if item["title"] in titles]
        if len(matched) == len(titles) and all(
            item["parse_status"] in {"SUCCEEDED", "SUCCEEDED_WITH_ERRORS"} for item in matched
        ):
            return
        time.sleep(1)
    raise TimeoutError("evaluation documents did not finish parsing")


def matches(result: dict, expected: dict) -> bool:
    document = result.get("document", {})
    locator = result.get("locator", {})
    if document.get("title") != expected.get("title"):
        return False
    kind = expected.get("kind")
    if kind == "xlsx":
        if locator.get("kind") != "xlsx" or locator.get("sheet") != expected.get("sheet"):
            return False
        if "row" in expected:
            rows = locator.get("rows") or [0, 0]
            if not (rows[0] <= expected["row"] <= rows[1]):
                return False
        return True
    if kind == "docx":
        if locator.get("kind") != "docx":
            return False
        return expected.get("section") in (locator.get("section_path") or [])
    if kind == "pptx":
        return locator.get("kind") == "pptx" and locator.get("slide") == expected.get("slide")
    return False


def run_queries(client: Client, project: str, queries: list[dict]) -> dict:
    results = []
    latencies = []
    for item in queries:
        response = client.search(project, item["query"], 10)
        latencies.append(response["latency_ms"])
        rank = None
        for index, result in enumerate(response["results"], start=1):
            if matches(result, item["expected"]):
                rank = index
                break
        results.append(
            {
                "category": item["category"],
                "query": item["query"],
                "rank": rank,
                "latency_ms": response["latency_ms"],
            }
        )
    latencies.sort()
    p95 = latencies[max(0, int(round(0.95 * (len(latencies) - 1))))]
    categories = {}
    for category in sorted({item["category"] for item in results}):
        rows = [item for item in results if item["category"] == category]
        categories[category] = {
            "count": len(rows),
            "recall_at_10": sum(1 for item in rows if item["rank"] is not None) / len(rows),
            "mrr": sum(1.0 / item["rank"] for item in rows if item["rank"]) / len(rows),
        }
    return {
        "count": len(results),
        "recall_at_10": sum(1 for item in results if item["rank"] is not None) / len(results),
        "mrr": sum(1.0 / item["rank"] for item in results if item["rank"]) / len(results),
        "p95_latency_ms": p95,
        "mean_latency_ms": round(statistics.fmean(latencies), 1),
        "categories": categories,
        "misses": [item for item in results if item["rank"] is None],
    }


def run_cells(client: Client, project: str, cells: list[dict]) -> dict:
    hits = 0
    details = []
    for item in cells:
        response = client.search(project, item["query"], 3)
        rank = next(
            (index for index, result in enumerate(response["results"], start=1) if matches(result, item["expected"])),
            None,
        )
        if rank is not None:
            hits += 1
        details.append({"query": item["query"], "rank": rank})
    return {
        "count": len(cells),
        "top3": hits / len(cells),
        "details": details,
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base-url", default="http://127.0.0.1:18080")
    parser.add_argument("--login", default="alice")
    parser.add_argument("--password", default="SystivraDemo1!")
    parser.add_argument("--project-id", default="22222222-2222-4222-8222-222222222222")
    parser.add_argument("--data-dir", default="data")
    parser.add_argument("--skip-ingest", action="store_true")
    parser.add_argument("--output", default=None)
    args = parser.parse_args()

    data_dir = Path(args.data_dir).resolve()
    corpus_dir = data_dir / f"phase2-eval-{RUN}"
    queries, cells = generate(corpus_dir)
    client = Client(args.base_url)
    client.login(args.login, args.password)

    manifest_key = f"{corpus_dir.name}/manifest.jsonl"
    if not args.skip_ingest:
        batch = client.bulk_import(args.project_id, manifest_key)
        job = wait_job(client, batch["job_id"], timeout=180)
        if job["status"] not in {"SUCCEEDED", "SUCCEEDED_WITH_ERRORS"}:
            raise RuntimeError(f"bulk import failed: {job}")
        titles = {item["expected"]["title"] for item in queries}
        wait_documents(client, args.project_id, titles)

    report = {
        "run": RUN,
        "corpus_dir": str(corpus_dir),
        "queries": run_queries(client, args.project_id, queries),
        "cells": run_cells(client, args.project_id, cells),
    }
    output = Path(args.output) if args.output else corpus_dir / "report.json"
    output.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
