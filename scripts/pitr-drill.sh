#!/usr/bin/env bash
# PITR（Point-In-Time Recovery）訓練。
#
# 専用の一時 PostgreSQL クラスタを 2 つ作り、archive_mode + WAL アーカイブで
# 「ベースバックアップ後の変更を、指定時刻より前の状態へ戻せる」ことを実測する。
# システムの PostgreSQL には触れない。
#
# 使い方:
#   ./scripts/pitr-drill.sh
#   PITR_KEEP=1 ./scripts/pitr-drill.sh   # 一時クラスタを残す

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export PATH="/opt/homebrew/opt/postgresql@18/bin:$PATH"
export LC_ALL=C
export LANG=C
KEEP="${PITR_KEEP:-0}"
BASE_PORT="${PITR_BASE_PORT:-55432}"
RESTORE_PORT="${PITR_RESTORE_PORT:-55433}"
RESTORE2_PORT="${PITR_RESTORE2_PORT:-55434}"
TMP="$(mktemp -d /tmp/systivra-pitr.XXXXXX)"
PRIMARY="$TMP/primary"
ARCHIVE="$TMP/archive"
BACKUP="$TMP/base"
RESTORE="$TMP/restore"
RESTORE2="$TMP/restore2"
START_MS="$(python3 -c 'import time; print(int(time.time()*1000))')"
FAIL=0

mkdir -p "$ARCHIVE" "$ROOT/data/phase8"

say() { printf '%s\n' "$*"; }
ok()  { printf '  \033[32mok\033[0m   %s\n' "$*"; }
no()  { printf '  \033[31mFAIL\033[0m %s\n' "$*"; FAIL=1; }

cleanup() {
  pg_ctl -D "$RESTORE2" -m immediate stop >/dev/null 2>&1 || true
  pg_ctl -D "$RESTORE" -m immediate stop >/dev/null 2>&1 || true
  pg_ctl -D "$PRIMARY" -m immediate stop >/dev/null 2>&1 || true
  if [ "$KEEP" != "1" ]; then rm -rf "$TMP"; fi
}
trap cleanup EXIT

wait_ready() {
  local port="$1"
  for _ in $(seq 1 60); do
    if pg_isready -h 127.0.0.1 -p "$port" -U postgres >/dev/null 2>&1; then return 0; fi
    sleep 0.25
  done
  return 1
}

say "== init primary =="
initdb -D "$PRIMARY" -A trust -U postgres --no-locale -E UTF8 >/dev/null
{
  printf "port = %s\n" "$BASE_PORT"
  printf "listen_addresses = '127.0.0.1'\n"
  printf "wal_level = replica\n"
  printf "archive_mode = on\n"
  printf "archive_command = 'test ! -f %s/%%f && cp %%p %s/%%f'\n" "$ARCHIVE" "$ARCHIVE"
  printf "max_wal_senders = 4\n"
} >> "$PRIMARY/postgresql.conf"
pg_ctl -D "$PRIMARY" -l "$PRIMARY/server.log" -w start >/dev/null

export PGHOST=127.0.0.1
export PGPORT="$BASE_PORT"
export PGUSER=postgres

psql -d postgres -v ON_ERROR_STOP=1 -q -c "CREATE DATABASE pitr;"
export PGDATABASE=pitr
for migration in "$ROOT"/migrations/*.sql; do
  psql -v ON_ERROR_STOP=1 -q -f "$migration"
done
ok "専用クラスタに 001〜021 を適用"

psql -v ON_ERROR_STOP=1 -q <<'SQL'
CREATE TABLE public.drill_marker (
  id integer PRIMARY KEY,
  note text NOT NULL,
  at timestamptz NOT NULL DEFAULT now()
);
INSERT INTO public.drill_marker (id, note) VALUES (1, 'before-base');
SQL

say "== base backup =="
pg_basebackup -h 127.0.0.1 -p "$BASE_PORT" -U postgres -D "$BACKUP" -Fp -X stream -c fast >/dev/null
ok "pg_basebackup（$(du -sh "$BACKUP" | cut -f1)）"

TARGET1="$(psql -Atc "SELECT now()::text;")"
psql -v ON_ERROR_STOP=1 -q -c "INSERT INTO public.drill_marker (id, note) VALUES (2, 'after-base');"
TARGET_LSN="$(psql -Atc "SELECT pg_current_wal_lsn();")"
BEFORE_ARCHIVE="$(psql -Atc "SELECT COALESCE(last_archived_wal,'') FROM pg_stat_archiver;")"
psql -v ON_ERROR_STOP=1 -q -c "SELECT pg_switch_wal();" >/dev/null
for _ in $(seq 1 40); do
  archived=$(psql -Atc "SELECT COALESCE(last_archived_wal,'') FROM pg_stat_archiver;")
  if [ "$archived" != "$BEFORE_ARCHIVE" ] && [ -n "$archived" ]; then break; fi
  sleep 0.25
done
archived=$(psql -Atc "SELECT COALESCE(last_archived_wal,'') FROM pg_stat_archiver;")
[ -n "$archived" ] && [ "$archived" != "$BEFORE_ARCHIVE" ] && ok "WAL がアーカイブされた（$(find "$ARCHIVE" -type f | wc -l | tr -d ' ') 本）" || no "WAL がアーカイブされていない"
TARGET2="$(psql -Atc "SELECT now()::text;")"

say "== restore to target (before after-base) =="
cp -R "$BACKUP" "$RESTORE"
printf "\nrestore_command = 'cp %s/%%f %%p'\nrecovery_target_time = '%s'\nrecovery_target_action = 'promote'\n" "$ARCHIVE" "$TARGET1" >> "$RESTORE/postgresql.auto.conf"
touch "$RESTORE/recovery.signal"
pg_ctl -D "$RESTORE" -o "-p $RESTORE_PORT" -l "$RESTORE/server.log" -w start >/dev/null
if wait_ready "$RESTORE_PORT"; then ok "復元クラスタが起動"; else no "復元クラスタが起動しない"; fi
restored1=$(PGPORT="$RESTORE_PORT" psql -Atc "SELECT count(*) FROM public.drill_marker WHERE id=2;")
[ "$restored1" = "0" ] && ok "指定時刻より後の行は復元されない（id=2 が 0）" || no "PITR 後に行が見える（$restored1）"
has1=$(PGPORT="$RESTORE_PORT" psql -Atc "SELECT count(*) FROM public.drill_marker WHERE id=1;")
[ "$has1" = "1" ] && ok "ベースバックアップ時点の行は残る（id=1）" || no "ベース行が無い（$has1）"

say "== restore to latest =="
cp -R "$BACKUP" "$RESTORE2"
printf "\nrestore_command = 'cp %s/%%f %%p'\nrecovery_target_lsn = '%s'\nrecovery_target_action = 'promote'\n" "$ARCHIVE" "$TARGET_LSN" >> "$RESTORE2/postgresql.auto.conf"
touch "$RESTORE2/recovery.signal"
pg_ctl -D "$RESTORE2" -o "-p $RESTORE2_PORT" -l "$RESTORE2/server.log" -w start >/dev/null
if wait_ready "$RESTORE2_PORT"; then ok "最新復元クラスタが起動"; else no "最新復元クラスタが起動しない"; fi
restored2=$(PGPORT="$RESTORE2_PORT" psql -Atc "SELECT count(*) FROM public.drill_marker WHERE id=2;")
[ "$restored2" = "1" ] && ok "WAL を最後まで再生すると id=2 が戻る" || no "最新復元で id=2 が無い（$restored2）"

END_MS="$(python3 -c 'import time; print(int(time.time()*1000))')"
RTO_MS=$((END_MS - START_MS))

python3 - "$RTO_MS" "$TARGET1" "$ARCHIVE" "$FAIL" <<'PY'
import json, sys, os
rto_ms, target, archive, fail = sys.argv[1:5]
report = {
    "rto_ms": int(rto_ms),
    "rto_sec": round(int(rto_ms) / 1000, 2),
    "recovery_target": target,
    "wal_files": len([p for p in os.listdir(archive) if os.path.isfile(os.path.join(archive, p))]),
    "checks_failed": int(fail),
    "mode": "専用一時クラスタ。システムの PostgreSQL には触れない。",
}
path = "data/phase8/pitr-drill.json"
with open(path, "w") as handle:
    json.dump(report, handle, ensure_ascii=False, indent=2)
    handle.write("\n")
print(json.dumps(report, ensure_ascii=False))
PY

if [ "$FAIL" -eq 0 ]; then
  printf '\033[32mPITR 訓練 合格（%s 秒）\033[0m\n' "$(python3 -c "print(round($RTO_MS/1000,2))")"
else
  printf '\033[31mPITR 訓練 失敗\033[0m\n'
  exit 1
fi
