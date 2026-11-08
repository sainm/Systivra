#!/usr/bin/env python3
"""監査ログを SIEM / Webhook へ転送する（カーソル付き NDJSON）。

使い方:
  AUDIT_WEBHOOK_URL=https://siem.example/ingest \\
  AUDIT_WEBHOOK_SECRET=... \\
  PGHOST=localhost PGUSER=systivra_scheduler PGDATABASE=systivra \\
  python3 scripts/audit-forward.py --once

成功したバッチの最後の id を `data/phase8/audit-cursor` に保存する。
"""

import argparse
import hashlib
import hmac
import json
import os
import subprocess
import sys
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CURSOR = Path(os.environ.get("AUDIT_CURSOR_FILE", ROOT / "data/phase8/audit-cursor"))
DATABASE = os.environ.get("PGDATABASE", "systivra")
BATCH = int(os.environ.get("AUDIT_BATCH_SIZE", "100"))


def read_cursor() -> int:
    try:
        return int(CURSOR.read_text().strip() or "0")
    except (OSError, ValueError):
        return 0


def write_cursor(value: int) -> None:
    CURSOR.parent.mkdir(parents=True, exist_ok=True)
    temporary = CURSOR.with_suffix(".tmp")
    temporary.write_text(f"{value}\n")
    os.replace(temporary, CURSOR)


def fetch(cursor: int) -> list[dict]:
    sql = (
        "SELECT json_build_object("
        "'id', id, 'occurred_at', occurred_at, 'actor_type', actor_type, "
        "'actor_id', actor_id, 'action', action, 'target_type', target_type, "
        "'target_id', target_id, 'result', result, 'trace_id', trace_id, "
        "'detail', detail, 'project_id', project_id"
        f")::text FROM core.audit_log WHERE id > {cursor} ORDER BY id LIMIT {BATCH};"
    )
    proc = subprocess.run(
        ["psql", "-d", DATABASE, "-Atc", sql],
        capture_output=True,
        text=True,
        check=True,
    )
    return [json.loads(line) for line in proc.stdout.splitlines() if line]


def send(url: str, secret: str, rows: list[dict]) -> None:
    body = ("\n".join(json.dumps(row, ensure_ascii=False) for row in rows) + "\n").encode()
    request = urllib.request.Request(url, data=body, method="POST")
    request.add_header("Content-Type", "application/x-ndjson")
    if secret:
        signature = hmac.new(secret.encode(), body, hashlib.sha256).hexdigest()
        request.add_header("X-Systivra-Signature", f"sha256={signature}")
    with urllib.request.urlopen(request, timeout=15) as response:
        if response.status >= 300:
            raise RuntimeError(f"webhook returned {response.status}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--once", action="store_true", help="1 バッチだけ転送する")
    args = parser.parse_args()
    url = os.environ.get("AUDIT_WEBHOOK_URL", "").strip()
    if not url:
        print("AUDIT_WEBHOOK_URL が未設定です", file=sys.stderr)
        return 2
    secret = os.environ.get("AUDIT_WEBHOOK_SECRET", "")
    cursor = read_cursor()
    total = 0
    while True:
        rows = fetch(cursor)
        if not rows:
            break
        send(url, secret, rows)
        cursor = int(rows[-1]["id"])
        write_cursor(cursor)
        total += len(rows)
        if args.once:
            break
    print(json.dumps({"forwarded": total, "cursor": cursor}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
