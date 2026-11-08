#!/usr/bin/env bash
# PostgreSQL 全体 + 原本データのバックアップ。
#
# 出力: <BACKUP_ROOT>/<UTC>/db.pgdump, originals.tar.gz, manifest.json
# Neo4j は PG から再構築できる導出物なので含めない（復旧訓練で再構築を検証する）。

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DB="${PGDATABASE:-systivra}"
DATA_DIR="${SYSTIVRA_DATA_DIR:-$ROOT/data}"
BACKUP_ROOT="${BACKUP_ROOT:-$ROOT/backups}"
TABLES="$ROOT/scripts/backup-tables.txt"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
DIR="$BACKUP_ROOT/$STAMP"
DUMP="$DIR/db.pgdump"
ORIGINALS="$DIR/originals.tar.gz"
GIT_REV="$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || true)"

mkdir -p "$DIR"
chmod 700 "$BACKUP_ROOT" "$DIR"

say() { printf '%s\n' "$*"; }
say "== backup =="
say "db=$DB dir=$DIR"

pg_dump --format=custom --no-owner --no-privileges --file="$DUMP" "$DB"

say "== table counts =="
SQL=""
while IFS= read -r table; do
  [ -z "$table" ] && continue
  SQL+="SELECT '$table', count(*) FROM $table UNION ALL "
done < "$TABLES"
SQL="${SQL% UNION ALL }"
psql -d "$DB" -At -F $'\t' -c "$SQL" > "$DIR/counts.tsv"
say "  $(wc -l < "$DIR/counts.tsv" | tr -d ' ') tables"

if [ -d "$DATA_DIR" ]; then
  tar -czf "$ORIGINALS" -C "$DATA_DIR" .
  say "originals: $(du -h "$ORIGINALS" | cut -f1)"
else
  say "originals: $DATA_DIR が無いため省略"
fi

ENCRYPTED=0
if [ -n "${BACKUP_ENCRYPTION_KEY:-}" ]; then
  openssl enc -aes-256-cbc -salt -pbkdf2 \
    -pass env:BACKUP_ENCRYPTION_KEY -in "$DUMP" -out "$DUMP.enc"
  mv "$DUMP.enc" "$DUMP"
  if [ -f "$ORIGINALS" ]; then
    openssl enc -aes-256-cbc -salt -pbkdf2 \
      -pass env:BACKUP_ENCRYPTION_KEY -in "$ORIGINALS" -out "$ORIGINALS.enc"
    mv "$ORIGINALS.enc" "$ORIGINALS"
  fi
  ENCRYPTED=1
  say "encrypted: AES-256-CBC (PBKDF2)"
fi

python3 - "$DIR" "$DB" "$GIT_REV" "$DATA_DIR" "$DUMP" "$ORIGINALS" "$ENCRYPTED" <<'PY'
import datetime
import hashlib
import json
import os
import subprocess
import sys

directory, database, git_rev, data_dir, dump, originals, encrypted = sys.argv[1:8]


def sha256(path):
    if not os.path.exists(path):
        return None
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def psql(sql):
    proc = subprocess.run(
        ["psql", "-d", database, "-Atc", sql],
        capture_output=True,
        text=True,
        check=True,
    )
    return proc.stdout.strip()


counts = {}
with open(os.path.join(directory, "counts.tsv")) as handle:
    for line in handle:
        table, count = line.rstrip("\n").split("\t")
        counts[table] = int(count)

hashes = {}
for table in (
    "core.project",
    "core.app_user",
    "corpus.document",
    "corpus.chunk",
    "corpus.evidence",
    "code.fact",
    "graph_meta.entity",
    "graph_meta.relationship",
    "core.external_identity",
):
    hashes[table] = psql(
        f"SELECT md5(string_agg(id::text, ',' ORDER BY id)) FROM {table};"
    )

manifest = {
    "captured_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
    "database": database,
    "git_rev": git_rev,
    "data_dir": data_dir,
    "encrypted": encrypted == "1",
    "dump": os.path.basename(dump),
    "dump_bytes": os.path.getsize(dump),
    "dump_sha256": sha256(dump),
    "originals": os.path.basename(originals) if os.path.exists(originals) else None,
    "originals_bytes": os.path.getsize(originals) if os.path.exists(originals) else None,
    "originals_sha256": sha256(originals),
    "last_audit_at": psql("SELECT COALESCE(max(occurred_at)::text, '') FROM core.audit_log;"),
    "last_job_at": psql(
        "SELECT COALESCE(max(COALESCE(finished_at, started_at, created_at))::text, '') FROM core.job;"
    ),
    "table_counts": counts,
    "content_hashes": hashes,
}

with open(os.path.join(directory, "manifest.json"), "w") as handle:
    json.dump(manifest, handle, ensure_ascii=False, indent=2)
    handle.write("\n")
print(json.dumps({"tables": len(counts), "dump_bytes": manifest["dump_bytes"]}))
PY

say "manifest: $DIR/manifest.json"

if [ "${BACKUP_RETENTION_DAYS:-0}" -gt 0 ]; then
  find "$BACKUP_ROOT" -mindepth 1 -maxdepth 1 -type d \
    -mtime +"$BACKUP_RETENTION_DAYS" ! -path "$DIR" -exec rm -rf {} +
  say "retention: ${BACKUP_RETENTION_DAYS} 日より古いバックアップを削除"
fi

if [ -n "${BACKUP_UPLOAD_CMD:-}" ]; then
  sh -c "$BACKUP_UPLOAD_CMD \"\$1\"" _ "$DIR"
  say "offsite: BACKUP_UPLOAD_CMD を実行"
fi
