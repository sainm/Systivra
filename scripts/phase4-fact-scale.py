#!/usr/bin/env python3
"""250 万 Fact 相当の COPY 永続化と supersede 増分を実測する。

専用の一時 DB を作り、code.fact へ COPY で投入し、一部を supersede する。
解析による Fact 生成や Neo4j 再投影はここでは行わない（別途 phase4-scale-eval）。
"""

from __future__ import annotations

import hashlib
import json
import os
import pathlib
import subprocess
import sys
import time
import uuid

ROOT = pathlib.Path(__file__).resolve().parents[1]
FACTS = int(os.environ.get("PHASE4_FACTS", "2500000"))
SUPERSEDE = int(os.environ.get("PHASE4_SUPERSEDE", "10000"))
PGHOST = os.environ.get("PGHOST", "localhost")
PGUSER = os.environ.get("PGUSER", os.environ.get("USER", "postgres"))
DB = f"systivra_fact_scale_{os.getpid()}"
REPORT = ROOT / "data/phase4-scale/fact-copy-report.json"
PROJECT_ID = str(uuid.uuid4())
REVISION_ID = str(uuid.uuid4())


def psql(sql: str, database: str = "postgres") -> str:
    proc = subprocess.run(
        ["psql", "-h", PGHOST, "-U", PGUSER, "-d", database, "-v", "ON_ERROR_STOP=1", "-Atc", sql],
        capture_output=True,
        text=True,
        check=True,
    )
    return proc.stdout.strip()


def apply_migrations() -> None:
    for path in sorted((ROOT / "migrations").glob("*.sql")):
        subprocess.run(
            ["psql", "-h", PGHOST, "-U", PGUSER, "-d", DB, "-v", "ON_ERROR_STOP=1", "-q", "-f", str(path)],
            check=True,
            capture_output=True,
        )


def fact_row(index: int, version: str) -> str:
    subject = f"s{index}"
    object_key = f"o{index}"
    digest = hashlib.md5(f"{subject}|{object_key}".encode()).hexdigest()
    row_id = str(uuid.uuid4())
    return (
        f"{row_id},{PROJECT_ID},{REVISION_ID},METHOD,{subject},CALLS,METHOD,{object_key},"
        f"RESOLVED,1.0,scale,{version},\\x{digest}\n"
    )


def copy_facts(count: int, version: str, start: int = 0) -> float:
    command = [
        "psql",
        "-h",
        PGHOST,
        "-U",
        PGUSER,
        "-d",
        DB,
        "-v",
        "ON_ERROR_STOP=1",
        "-c",
        "\\copy code.fact (id,project_id,revision_id,subject_kind,subject_key,relation,"
        "object_kind,object_key,resolution,confidence,analysis_engine,engine_version,fact_hash) "
        "FROM STDIN WITH (FORMAT csv)",
    ]
    started = time.monotonic()
    proc = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    assert proc.stdin is not None
    batch = []
    for index in range(start, start + count):
        batch.append(fact_row(index, version))
        if len(batch) >= 10000:
            proc.stdin.write("".join(batch).encode())
            batch.clear()
    if batch:
        proc.stdin.write("".join(batch).encode())
    proc.stdin.close()
    proc.wait()
    stderr = proc.stderr.read() if proc.stderr else b""
    if proc.returncode != 0:
        raise RuntimeError(stderr.decode("utf-8", "replace"))
    return time.monotonic() - started


def main() -> int:
    REPORT.parent.mkdir(parents=True, exist_ok=True)
    psql(f'CREATE DATABASE "{DB}";')
    try:
        apply_migrations()
        copy_seconds = copy_facts(FACTS, "1")
        live = int(psql("SELECT count(*) FROM code.fact WHERE valid_to IS NULL;", DB))

        started = time.monotonic()
        psql(
            f"CREATE TABLE public.supersede_old AS "
            f"SELECT id, subject_key, object_key FROM code.fact "
            f"ORDER BY length(subject_key), subject_key LIMIT {SUPERSEDE}; "
            f"UPDATE code.fact f SET valid_to = now() "
            f"FROM public.supersede_old s WHERE f.id = s.id;",
            DB,
        )
        psql(
            "CREATE TABLE public.supersede_new (id uuid, subject_key text, object_key text);",
            DB,
        )
        command = [
            "psql", "-h", PGHOST, "-U", PGUSER, "-d", DB, "-v", "ON_ERROR_STOP=1", "-c",
            "\\copy public.supersede_new FROM STDIN WITH (FORMAT csv)",
        ]
        proc = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        assert proc.stdin is not None
        rows = []
        for index in range(SUPERSEDE):
            rows.append(f"{uuid.uuid4()},s{index},o{index}\n")
        proc.stdin.write("".join(rows).encode())
        proc.stdin.close()
        proc.wait()
        stderr = proc.stderr.read() if proc.stderr else b""
        if proc.returncode != 0:
            raise RuntimeError(stderr.decode("utf-8", "replace"))
        psql(
            "INSERT INTO code.fact (id,project_id,revision_id,subject_kind,subject_key,relation,"
            "object_kind,object_key,resolution,confidence,analysis_engine,engine_version,fact_hash) "
            "SELECT n.id, f.project_id, f.revision_id, 'METHOD', n.subject_key, 'CALLS', 'METHOD', "
            "n.object_key, 'RESOLVED', 1.0, 'scale', '2', f.fact_hash "
            "FROM public.supersede_new n JOIN public.supersede_old o ON o.subject_key = n.subject_key "
            "JOIN code.fact f ON f.id = o.id; "
            "UPDATE code.fact f SET superseded_by = n.id "
            "FROM public.supersede_new n JOIN public.supersede_old o ON o.subject_key = n.subject_key "
            "WHERE f.id = o.id; "
            "UPDATE code.fact n SET superseded_by = NULL WHERE n.engine_version = '2';",
            DB,
        )
        supersede_seconds = time.monotonic() - started
        live_after = int(psql("SELECT count(*) FROM code.fact WHERE valid_to IS NULL;", DB))
        total_after = int(psql("SELECT count(*) FROM code.fact;", DB))

        report = {
            "facts": FACTS,
            "copy_seconds": round(copy_seconds, 2),
            "copy_rows_per_second": int(FACTS / copy_seconds) if copy_seconds else None,
            "live_after_copy": live,
            "superseded": SUPERSEDE,
            "supersede_seconds": round(supersede_seconds, 2),
            "live_after_supersede": live_after,
            "total_after_supersede": total_after,
            "ok": live == FACTS and live_after == FACTS and total_after == FACTS + SUPERSEDE,
        }
        REPORT.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        print(json.dumps(report, ensure_ascii=False, indent=2))
        return 0 if report["ok"] else 1
    finally:
        psql(f'DROP DATABASE IF EXISTS "{DB}";')


if __name__ == "__main__":
    sys.exit(main())
