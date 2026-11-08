#!/usr/bin/env python3
"""ジョブ回収（worker が落ちたときの復旧）を実測する。

前提: worker が **起動済み**（`WORKER_TICK_SEC=1`）。API は不要。
DB に直接 job 行を作り、worker の回収と再実行を観察する。

  PGDATABASE=systivra_final_verify python3 scripts/job-recovery-eval.py

確かめること:
  1. 心拍が途切れ、試行回数が残っている job → QUEUED に戻り、**再実行される**
  2. 心拍が途切れ、試行回数を使い切った job → FAILED（無限に再キューしない）
  3. 心拍が新しい RUNNING の job → **触らない**（生きている job を奪わない）
  4. 24 時間を過ぎた PENDING の承認 → `EXPIRED`（新しい PENDING は残る）
"""

from __future__ import annotations

import json
import os
import pathlib
import subprocess
import sys
import time
import uuid

ROOT = pathlib.Path(__file__).resolve().parents[1]
DB = os.environ.get("PGDATABASE", "systivra")
PROJECT_ID = "22222222-2222-4222-8222-222222222222"
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


def job_row(job_id: str) -> tuple[str, int, str]:
    row = psql(
        "SELECT status || '|' || attempt || '|' || "
        "COALESCE(error->>'code', '') FROM core.job WHERE id = '%s';" % job_id
    )
    status, attempt, code = row.split("|")
    return status, int(attempt), code


def insert_job(kind: str, attempt: int, max_attempts: int, stale_seconds: int) -> str:
    job_id = str(uuid.uuid4())
    psql(
        "INSERT INTO core.job "
        "(id, project_id, kind, status, priority, idempotency_key, input, attempt,"
        " max_attempts, locked_by, heartbeat_at, started_at) VALUES "
        f"('{job_id}', '{PROJECT_ID}', '{kind}', 'RUNNING', 0, 'recovery-{job_id}', "
        f"'{{}}'::jsonb, {attempt}, {max_attempts}, 'gone-worker', "
        f"now() - interval '{stale_seconds} seconds', now() - interval '600 seconds');"
    )
    return job_id


def wait_status(job_id: str, wanted: set[str], seconds: float) -> tuple[str, int, str]:
    deadline = time.time() + seconds
    last = ("", 0, "")
    while time.time() < deadline:
        last = job_row(job_id)
        if last[0] in wanted:
            return last
        time.sleep(0.3)
    return last


def main() -> int:
    # 1. 心拍断 + 試行残り → 再実行される
    recoverable = insert_job("NOOP_RECOVERY", attempt=1, max_attempts=3, stale_seconds=600)
    status, attempt, code = wait_status(recoverable, {"FAILED", "SUCCEEDED"}, 30)
    assert status == "FAILED", (status, attempt, code)
    # kind が未対応なので処理自体は失敗する。大事なのは**再クレームされた**ことで、
    # それは attempt が 1 → 2 に増えたことで分かる（回収されなければ 1 のまま）。
    assert attempt == 2, (status, attempt, code)
    REPORT["recoverable"] = {"status": status, "attempt": attempt}

    # 回収は job_event にも残る（静かな状態書換えにしない）。
    recover_events = psql(
        "SELECT event || '|' || (data->>'reason') FROM core.job_event "
        f"WHERE job_id = '{recoverable}' ORDER BY id;"
    )
    assert "progress|stalled_heartbeat" in recover_events, recover_events
    REPORT["recoverable"]["events"] = recover_events.splitlines()

    # 2. 心拍断 + 試行を使い切り → FAILED で止まる（再クレームしない）
    poison = insert_job("NOOP_POISON", attempt=3, max_attempts=3, stale_seconds=600)
    status, attempt, code = wait_status(poison, {"FAILED"}, 30)
    assert status == "FAILED", (status, attempt, code)
    assert attempt == 3, (status, attempt, code)
    assert code == "WORKER_HEARTBEAT_LOST", (status, attempt, code)
    time.sleep(3)  # 再キューされないことを少し待って確かめる
    status2, attempt2, _ = job_row(poison)
    assert (status2, attempt2) == ("FAILED", 3), (status2, attempt2)
    poison_events = psql(
        "SELECT event || '|' || (data->>'code') FROM core.job_event "
        f"WHERE job_id = '{poison}' ORDER BY id;"
    )
    assert "error|WORKER_HEARTBEAT_LOST" in poison_events, poison_events
    REPORT["poison"] = {
        "status": status2,
        "attempt": attempt2,
        "code": code,
        "events": poison_events.splitlines(),
    }

    # 3. 心拍が新しい RUNNING → 奪わない
    alive = insert_job("NOOP_ALIVE", attempt=1, max_attempts=3, stale_seconds=0)
    time.sleep(4)
    status, attempt, code = job_row(alive)
    assert status == "RUNNING", (status, attempt, code)
    assert attempt == 1, (status, attempt, code)
    REPORT["alive"] = {"status": status, "attempt": attempt}

    # 4. 承認待ちは期限切れになる（EXPIRED は enum に在るだけで誰も使って
    #    いなかった。古い依頼を後から承認できてしまう状態を直したか）。
    old_approval = str(uuid.uuid4())
    fresh_approval = str(uuid.uuid4())
    for approval_id, age in ((old_approval, "25 hours"), (fresh_approval, "1 minute")):
        psql(
            "INSERT INTO ai.approval_request "
            "(id, project_id, action, payload_digest, payload, status, "
            " requested_by, created_at) VALUES "
            f"('{approval_id}', '{PROJECT_ID}', 'expiry-probe', "
            "sha256('{}'::text::bytea), '{}'::jsonb, 'PENDING', 'probe', "
            f"now() - interval '{age}');"
        )

    def approval_status(approval_id: str) -> str:
        return psql(
            "SELECT status FROM ai.approval_request "
            f"WHERE id = '{approval_id}';"
        )

    expired = ""
    deadline = time.time() + 30
    while time.time() < deadline:
        expired = approval_status(old_approval)
        if expired == "EXPIRED":
            break
        time.sleep(0.3)
    assert expired == "EXPIRED", expired
    assert approval_status(fresh_approval) == "PENDING"
    REPORT["approval_expiry"] = {
        "old": expired,
        "fresh": approval_status(fresh_approval),
    }

    # 片付け（検証用の行を残さない）
    psql(
        "DELETE FROM core.job_event WHERE job_id IN "
        f"('{recoverable}','{poison}','{alive}');"
    )
    psql(
        "DELETE FROM core.job WHERE id IN "
        f"('{recoverable}','{poison}','{alive}');"
    )
    psql(
        "DELETE FROM ai.approval_request WHERE id IN "
        f"('{old_approval}','{fresh_approval}');"
    )

    out = ROOT / "data" / "phase8" / "job-recovery.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(REPORT, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(REPORT, ensure_ascii=False, indent=2))
    print(f"\nwritten: {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
