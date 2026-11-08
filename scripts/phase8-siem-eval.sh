#!/usr/bin/env bash
# 監査ログの SIEM 転送をローカル受信機で実測する。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PORT="${MOCK_SIEM_PORT:-11550}"
OUT="$(mktemp /tmp/systivra-siem.XXXXXX)"
CURSOR="$(mktemp /tmp/systivra-cursor.XXXXXX)"
SECRET="siem-eval-secret"

MOCK_SIEM_PORT="$PORT" MOCK_SIEM_OUT="$OUT" MOCK_SIEM_SECRET="$SECRET" \
  python3 "$ROOT/scripts/mock-siem.py" >/tmp/systivra-siem-mock.log 2>&1 &
PID=$!
trap 'kill "$PID" >/dev/null 2>&1 || true; rm -f "$OUT" "$CURSOR"' EXIT

for _ in $(seq 1 40); do
  curl -sf "http://127.0.0.1:$PORT/healthz" >/dev/null 2>&1 && break
  sleep 0.2
done

PGHOST="${PGHOST:-localhost}" PGUSER="${PGUSER:-$(whoami)}" PGDATABASE="${PGDATABASE:-systivra}" \
AUDIT_CURSOR_FILE="$CURSOR" AUDIT_BATCH_SIZE=10000 \
AUDIT_WEBHOOK_URL="http://127.0.0.1:$PORT/audit" AUDIT_WEBHOOK_SECRET="$SECRET" \
  python3 "$ROOT/scripts/audit-forward.py" --once > /tmp/systivra-forward-1.json

FIRST=$(cat /tmp/systivra-forward-1.json)
LINES=$(wc -l < "$OUT" | tr -d ' ')
PGHOST="${PGHOST:-localhost}" PGUSER="${PGUSER:-$(whoami)}" PGDATABASE="${PGDATABASE:-systivra}" \
AUDIT_CURSOR_FILE="$CURSOR" AUDIT_BATCH_SIZE=10000 \
AUDIT_WEBHOOK_URL="http://127.0.0.1:$PORT/audit" AUDIT_WEBHOOK_SECRET="$SECRET" \
  python3 "$ROOT/scripts/audit-forward.py" --once > /tmp/systivra-forward-2.json
SECOND=$(cat /tmp/systivra-forward-2.json)

python3 - "$FIRST" "$SECOND" "$LINES" <<'PY'
import json, sys
first = json.loads(sys.argv[1])
second = json.loads(sys.argv[2])
lines = int(sys.argv[3])
report = {
    "first": first,
    "second": second,
    "received_lines": lines,
    "ok": first["forwarded"] == lines and first["forwarded"] > 0 and second["forwarded"] == 0,
}
path = "data/phase8/siem-report.json"
with open(path, "w") as handle:
    json.dump(report, handle, ensure_ascii=False, indent=2)
    handle.write("\n")
print(json.dumps(report, ensure_ascii=False))
sys.exit(0 if report["ok"] else 1)
PY
