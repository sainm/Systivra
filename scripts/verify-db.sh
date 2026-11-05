#!/usr/bin/env bash
# scripts/verify-db.sh
# P1-DB-01 の完了条件（detail/05 §5.1）を機械で確かめる。
#
# 使い方:
#   ./scripts/verify-db.sh
#   PGHOST=/tmp/pg PGPORT=5432 PGUSER=postgres ./scripts/verify-db.sh
#
# 接続は libpq の標準環境変数（PGHOST / PGPORT / PGUSER / PGPASSWORD）に従う。
# 一時 DB（systivra_verify_<pid>）を作り、最後に必ず消す。
# 前提: psql が PATH にあり、管理者権限のロールで接続できること。

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DB="systivra_verify_$$"
FAIL=0

export PGDATABASE="${PGDATABASE:-postgres}"

cleanup() {
  psql -d postgres -q -c "DROP DATABASE IF EXISTS $DB;" >/dev/null 2>&1 || true
}
trap cleanup EXIT

say() { printf '%s\n' "$*"; }
ok()  { printf '  \033[32mok\033[0m   %s\n' "$*"; }
no()  { printf '  \033[31mFAIL\033[0m %s\n' "$*"; FAIL=1; }

psql -d postgres -q -c "CREATE DATABASE $DB;" || { say "一時 DB を作れません"; exit 1; }
export PGDATABASE="$DB"

say "== 適用 =="
# 順序が重要：schema を作ってから権限を付ける。
psql -v ON_ERROR_STOP=1 -q -f "$ROOT/migrations/001_core.sql"
ok "001_core.sql"
psql -v ON_ERROR_STOP=1 -q -f "$ROOT/migrations/002_core_ops.sql"
ok "002_core_ops.sql"
psql -v ON_ERROR_STOP=1 -q -f "$ROOT/migrations/003_authz.sql"
ok "003_authz.sql"
psql -v ON_ERROR_STOP=1 -q -f "$ROOT/deployment/postgres/roles.sql"
ok "roles.sql（migrations の後）"

say "== RLS =="
psql -q >/dev/null <<'SQL'
INSERT INTO core.workspace (code,name) VALUES ('ws','WS');
INSERT INTO core.project (workspace_id,code,name,source_kind)
  SELECT id,'P1','P1','DIR' FROM core.workspace;
INSERT INTO core.project (workspace_id,code,name,source_kind)
  SELECT id,'P2','P2','DIR' FROM core.workspace;
SQL
PID=$(psql -tAc "SELECT id FROM core.project WHERE code='P1';" | tr -d ' ')

n=$(psql -U systivra_app -tA -c "SET app.project_id='$PID'; SELECT count(*) FROM core.project;" | tail -1)
[ "$n" = "1" ] && ok "project_id 設定時は自プロジェクトのみ（1 件）" || no "自プロジェクトが見えない（${n:-空} 件）"

n=$(psql -U systivra_app -tA -c "SELECT count(*) FROM core.project;" | tail -1)
[ "$n" = "0" ] && ok "未設定時は 0 件・エラーなし" || no "未設定時の挙動が違う（${n:-空} 件）"

say "== 権限分離 =="
if psql -U systivra_app -q -c "UPDATE core.job SET status='RUNNING';" >/dev/null 2>&1; then
  no "app が core.job を UPDATE できてしまう"
else
  ok "app は core.job を UPDATE できない"
fi

if psql -U systivra_app -q -c "UPDATE core.audit_log SET result='x';" >/dev/null 2>&1; then
  no "app が audit_log を UPDATE できてしまう"
else
  ok "audit_log は追記専用"
fi

say "== ジョブ取り出し =="
psql -q >/dev/null <<SQL
INSERT INTO core.job (project_id,kind,priority,status,idempotency_key,input)
VALUES ('$PID','PDF_PARSE',5,'QUEUED','k1','{}'),
       ('$PID','PDF_PARSE',9,'QUEUED','k3','{}'),
       ('$PID','PDF_PARSE',1,'QUEUED','k2','{}');
SQL
# FOR UPDATE は集約と併用できないので、副問い合わせで先に固定する。
got=$(psql -U systivra_scheduler -tA -c \
  "SELECT string_agg(priority::text, ',') FROM (SELECT priority FROM core.job WHERE status='QUEUED' ORDER BY priority DESC, created_at FOR UPDATE SKIP LOCKED) t;" | tail -1)
[ "$got" = "9,5,1" ] && ok "priority DESC で取り出せる（9,5,1）" || no "取り出し順が違う（${got:-空}）"

say "== 監査ログ分区 =="
n=$(psql -tAc "SELECT count(*) FROM pg_inherits WHERE inhparent='core.audit_log'::regclass;")
[ "${n:-0}" -ge 12 ] && ok "月次分区が $n 個ある" || no "分区が足りない（${n:-0} 個）"

say "== SET LOCAL 残留（接続再利用）=="
# 同じ接続で「スコープ付き → コミット → スコープ無し」を続けて実行する。
# SET LOCAL はコミット時に「未設定」ではなく空文字へ戻るため、NULLIF が無いと
# ''::uuid で落ちる。接続プールでは必ず起きる。
out=$(psql -U systivra_app -tA \
  -c "BEGIN; SELECT set_config('app.project_id', '$PID', true); COMMIT;" \
  -c "SELECT count(*) FROM core.project;" 2>&1)
last=$(printf '%s\n' "$out" | tail -1)
[ "$last" = "0" ] && ok "コミット後の未スコープ参照が落ちない（0 件）" || no "SET LOCAL 残留で失敗（$last）"

say "== 冪等キー =="
if psql -q -c "INSERT INTO core.job (project_id,kind,status,idempotency_key,input) VALUES ('$PID','PDF_PARSE','QUEUED','k1','{}');" >/dev/null 2>&1; then
  no "同じ idempotency_key で二重登録できてしまう"
else
  ok "同じ idempotency_key は拒否される"
fi

echo
if [ "$FAIL" -eq 0 ]; then
  printf '\033[32m全て通過\033[0m\n'
else
  printf '\033[31m失敗あり\033[0m\n'
  exit 1
fi
