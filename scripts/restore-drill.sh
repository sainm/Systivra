#!/usr/bin/env bash
# バックアップからの復旧訓練。
#
# 別 DB へ pg_restore し、件数・内容ハッシュ・RLS・FTS・Fact 一意性を照合した後、
# 復元 DB に対して API を起動し Neo4j の再構築と drift 0 を確認する。
#
# 使い方:
#   ./scripts/restore-drill.sh [backup dir]
#   KEEP=1 ./scripts/restore-drill.sh   # 復元 DB と展開先を残す

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BACKUP_ROOT="${BACKUP_ROOT:-$ROOT/backups}"
BACKUP_DIR="${1:-$(ls -1d "$BACKUP_ROOT"/*/ 2>/dev/null | sort | tail -1)}"
BACKUP_DIR="${BACKUP_DIR%/}"
MANIFEST="$BACKUP_DIR/manifest.json"
TABLES="$ROOT/scripts/backup-tables.txt"
DB="systivra_restore_$$"
PORT="${RESTORE_API_PORT:-18091}"
KEEP="${KEEP:-0}"
DATA_TMP="$(mktemp -d /tmp/systivra-restore-data.XXXXXX)"
API_PID=""
FAIL=0
LOG="$ROOT/data/phase8/restore-drill.log"
START_MS="$(python3 -c 'import time; print(int(time.time()*1000))')"

mkdir -p "$(dirname "$LOG")"

say() { printf '%s\n' "$*"; }
ok()  { printf '  \033[32mok\033[0m   %s\n' "$*"; }
no()  { printf '  \033[31mFAIL\033[0m %s\n' "$*"; FAIL=1; }

cleanup() {
  if [ -n "$API_PID" ]; then kill "$API_PID" >/dev/null 2>&1 || true; fi
  if [ "$KEEP" != "1" ]; then
    psql -d postgres -q -c "DROP DATABASE IF EXISTS $DB;" >/dev/null 2>&1 || true
    rm -rf "$DATA_TMP"
  fi
}
trap cleanup EXIT

[ -f "$MANIFEST" ] || { say "manifest がありません: $MANIFEST"; exit 1; }

ENCRYPTED=$(python3 -c 'import json,sys;print(int(json.load(open(sys.argv[1])).get("encrypted",False)))' "$MANIFEST")
DUMP_NAME=$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["dump"])' "$MANIFEST")
ORIG_NAME=$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1])).get("originals") or "")' "$MANIFEST")
DUMP_FILE="$BACKUP_DIR/$DUMP_NAME"
ORIG_FILE="$BACKUP_DIR/$ORIG_NAME"
ORIG_TAR="$ORIG_FILE"

say "== restore drill =="
say "backup=$BACKUP_DIR"
say "db=$DB"

if [ "$ENCRYPTED" = "1" ]; then
  if [ -z "${BACKUP_ENCRYPTION_KEY:-}" ]; then
    say "BACKUP_ENCRYPTION_KEY が必要です"
    exit 1
  fi
  openssl enc -d -aes-256-cbc -pbkdf2 \
    -pass env:BACKUP_ENCRYPTION_KEY -in "$DUMP_FILE" -out "$DATA_TMP/db.pgdump"
  DUMP_FILE="$DATA_TMP/db.pgdump"
  if [ -n "$ORIG_NAME" ]; then
    openssl enc -d -aes-256-cbc -pbkdf2 \
      -pass env:BACKUP_ENCRYPTION_KEY -in "$ORIG_FILE" -out "$DATA_TMP/originals.tar.gz"
    ORIG_TAR="$DATA_TMP/originals.tar.gz"
  fi
  ok "バックアップを復号"
fi

psql -d postgres -q -c "CREATE DATABASE $DB;"
pg_restore --no-owner --no-privileges -d "$DB" "$DUMP_FILE"
psql -d "$DB" -v ON_ERROR_STOP=1 -q -f "$ROOT/deployment/postgres/roles.sql"
psql -d "$DB" -q -c "UPDATE core.session SET revoked_at = now() WHERE revoked_at IS NULL;"
ok "pg_restore + roles.sql（復元後は全セッションを失効）"

say "== counts =="
python3 - "$MANIFEST" <<'PY' > /tmp/systivra-expected-counts.tsv
import json, sys
manifest = json.load(open(sys.argv[1]))
for table, count in manifest["table_counts"].items():
    print(f"{table}\t{count}")
PY
while IFS=$'\t' read -r table expected; do
  actual=$(psql -d "$DB" -Atc "SELECT count(*) FROM $table;" | tr -d ' ')
  if [ "$actual" = "$expected" ]; then
    ok "$table = $actual"
  else
    no "$table expected=$expected actual=$actual"
  fi
done < /tmp/systivra-expected-counts.tsv

say "== content hashes =="
python3 - "$MANIFEST" <<'PY' > /tmp/systivra-expected-hashes.tsv
import json, sys
manifest = json.load(open(sys.argv[1]))
for table, digest in manifest["content_hashes"].items():
    print(f"{table}\t{digest}")
PY
while IFS=$'\t' read -r table expected; do
  actual=$(psql -d "$DB" -Atc "SELECT md5(string_agg(id::text, ',' ORDER BY id)) FROM $table;" | tr -d ' ')
  if [ "$actual" = "$expected" ]; then
    ok "$table hash"
  else
    no "$table hash expected=$expected actual=$actual"
  fi
done < /tmp/systivra-expected-hashes.tsv

say "== RLS / scope =="
PID=$(psql -d "$DB" -Atc "SELECT id FROM core.project ORDER BY code LIMIT 1;" | tr -d ' ')
DOC=$(psql -d "$DB" -Atc "SELECT id FROM corpus.document LIMIT 1;" | tr -d ' ')
n=$(psql -U systivra_app -d "$DB" -Atc "SELECT count(*) FROM corpus.chunk;" | tail -1)
[ "$n" = "0" ] && ok "未スコープの chunk は 0 件" || no "未スコープで chunk が見える（$n）"
n=$(psql -U systivra_app -d "$DB" -Atc "SET app.project_id='$PID'; SELECT count(*) FROM corpus.chunk;" | tail -1)
expected=$(python3 -c "import json;print(json.load(open('$MANIFEST'))['table_counts']['corpus.chunk'])")
[ "$n" = "$expected" ] && ok "スコープ内の chunk は $n 件" || no "スコープ内 chunk expected=$expected actual=$n"
resolved=$(psql -d "$DB" -Atc "SELECT core.project_of('document', '$DOC');" | tr -d ' ')
[ "$resolved" = "$PID" ] && ok "core.project_of が document を解決" || no "project_of document 不一致"

say "== search / evidence / fact =="
n=$(psql -d "$DB" -Atc "SELECT count(*) FROM corpus.chunk WHERE fts IS NOT NULL;" | tr -d ' ')
[ "$n" -gt 0 ] && ok "FTS 列が復元されている（$n 件）" || no "FTS 列が空"
n=$(psql -d "$DB" -Atc "SELECT count(*) FROM corpus.evidence e JOIN corpus.chunk c ON c.id=e.chunk_id;" | tr -d ' ')
[ "$n" -gt 0 ] && ok "evidence と chunk の結合が通る（$n 件）" || no "evidence/chunk の結合が 0 件"
n=$(psql -d "$DB" -Atc "SELECT count(*) FROM (SELECT project_id, revision_id, subject_key, relation, object_key, analysis_engine FROM code.fact WHERE valid_to IS NULL GROUP BY 1,2,3,4,5,6 HAVING count(*) > 1) d;" | tr -d ' ')
[ "$n" = "0" ] && ok "live Fact の重複 0" || no "live Fact の重複 $n"
n=$(psql -d "$DB" -Atc "SELECT count(*) FROM core.session WHERE revoked_at IS NULL;" | tr -d ' ')
[ "$n" = "0" ] && ok "復元後の live session は 0（全失効）" || no "復元後に live session が残る（$n）"

say "== originals =="
ORIG_SHA=$(python3 -c "import json;print(json.load(open('$MANIFEST')).get('originals_sha256') or '')")
if [ -n "$ORIG_SHA" ] && [ -f "$ORIG_FILE" ]; then
  actual=$(shasum -a 256 "$ORIG_FILE" | cut -d' ' -f1)
  [ "$actual" = "$ORIG_SHA" ] && ok "originals の SHA-256 一致" || no "originals の SHA-256 不一致"
  tar -xzf "$ORIG_TAR" -C "$DATA_TMP"
  files=$(find "$DATA_TMP" -type f | wc -l | tr -d ' ')
  [ "$files" -gt 0 ] && ok "originals を展開できる（$files ファイル）" || no "originals が空"
else
  say "  (originals なし)"
fi

say "== graph rebuild =="
DATABASE_URL="postgres://systivra_app@localhost:5432/$DB" \
PORT="$PORT" \
SESSION_SECRET=0123456789abcdef0123456789abcdef \
SESSION_COOKIE_SECURE=false \
PUBLIC_BASE_URL="http://127.0.0.1:$PORT" \
SYSTIVRA_DATA_DIR="$DATA_TMP" \
NEO4J_URI="${NEO4J_URI:-bolt://localhost:7687}" \
NEO4J_USER="${NEO4J_USER:-neo4j}" \
NEO4J_PASSWORD="${NEO4J_PASSWORD:-neo4j}" \
"$ROOT/target/debug/systivra-api" > /tmp/systivra-restore-api.log 2>&1 &
API_PID=$!

for _ in $(seq 1 50); do
  if curl -sf "http://127.0.0.1:$PORT/healthz" >/dev/null 2>&1; then break; fi
  sleep 0.2
done

COOKIE="$(mktemp /tmp/systivra-restore-cookie.XXXXXX)"
LOGIN=$(curl -s -o /tmp/systivra-restore-login.json -w '%{http_code}' \
  -c "$COOKIE" -H 'Content-Type: application/json' \
  -d "{\"login_id\":\"alice\",\"password\":\"${SEED_PASSWORD:-SystivraDemo1!}\"}" \
  "http://127.0.0.1:$PORT/api/v1/auth/login" || true)
[ "$LOGIN" = "200" ] && ok "復元 DB のユーザーでログイン" || no "復元 DB のログイン（$LOGIN）"

if [ "${CLEAN_ROOM:-0}" = "1" ]; then
  cypher-shell -a "${NEO4J_URI:-bolt://localhost:7687}" \
    -u "${NEO4J_USER:-neo4j}" -p "${NEO4J_PASSWORD:-neo4j}" \
    "MATCH (n {project_id: '$PID'}) DETACH DELETE n" >/dev/null
  remaining=$(cypher-shell -a "${NEO4J_URI:-bolt://localhost:7687}" \
    -u "${NEO4J_USER:-neo4j}" -p "${NEO4J_PASSWORD:-neo4j}" \
    --format plain "MATCH (n {project_id: '$PID'}) RETURN count(n)" | tail -1 | tr -d ' ')
  [ "$remaining" = "0" ] && ok "Neo4j のプロジェクト投影を空にした" || no "Neo4j に $remaining ノードが残る"
fi

REBUILD=$(curl -s -o /tmp/systivra-restore-rebuild.json -w '%{http_code}' \
  -b "$COOKIE" -X POST "http://127.0.0.1:$PORT/api/v1/projects/$PID/graph/rebuild" || true)
[ "$REBUILD" = "200" ] && ok "graph rebuild が 200" || no "graph rebuild（$REBUILD）"

STATS=$(curl -s -b "$COOKIE" "http://127.0.0.1:$PORT/api/v1/projects/$PID/graph/stats" || true)
printf '%s' "$STATS" > /tmp/systivra-restore-stats.json
DRIFT=$(printf '%s' "$STATS" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("drift_rate", 1))' 2>/dev/null || echo 1)
PGN=$(printf '%s' "$STATS" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("pg_nodes", -1))' 2>/dev/null || echo -1)
NEON=$(printf '%s' "$STATS" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("neo4j_nodes", -2))' 2>/dev/null || echo -2)
[ "$DRIFT" = "0.0" ] && [ "$PGN" = "$NEON" ] && ok "Pg/Neo4j の node 数一致・drift 0（$PGN）" || no "graph drift=$DRIFT pg=$PGN neo4j=$NEON"

kill "$API_PID" >/dev/null 2>&1 || true
API_PID=""
END_MS="$(python3 -c 'import time; print(int(time.time()*1000))')"
RTO_MS=$((END_MS - START_MS))

python3 - "$MANIFEST" "$RTO_MS" "$LOG" "$FAIL" "$DRIFT" "$PGN" "$NEON" "$BACKUP_DIR" <<'PY'
import json, sys
manifest_path, rto_ms, log_path, fail, drift, pg_nodes, neo4j_nodes, backup_dir = sys.argv[1:9]
report = {
    "backup_dir": backup_dir,
    "captured_at": json.load(open(manifest_path))["captured_at"],
    "rto_ms": int(rto_ms),
    "rto_sec": round(int(rto_ms) / 1000, 2),
    "rpo_note": "pg_dump の一貫したスナップショット。WAL 継続転送は未実装。",
    "checks_failed": int(fail),
    "graph": {"drift_rate": float(drift), "pg_nodes": int(pg_nodes), "neo4j_nodes": int(neo4j_nodes)},
}
path = "data/phase8/restore-drill.json"
with open(path, "w") as handle:
    json.dump(report, handle, ensure_ascii=False, indent=2)
    handle.write("\n")
print(json.dumps(report, ensure_ascii=False))
PY

if [ "$FAIL" -eq 0 ]; then
  printf '\033[32m復旧訓練 合格（RTO %s 秒）\033[0m\n' "$(python3 -c "print(round($RTO_MS/1000,2))")"
else
  printf '\033[31m復旧訓練 失敗\033[0m\n'
  exit 1
fi
