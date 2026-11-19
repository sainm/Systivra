#!/usr/bin/env bash
# 既存 DB を最新マイグレーションへ追いつかせる。
#
#   ./scripts/migrate-catchup.sh systivra_phase6_verify
#   PGHOST=... PGUSER=... ./scripts/migrate-catchup.sh mydb
#
# 何をするか:
#   1. `core.schema_migration` が無ければ作る（`migrations/032`）。
#      既存 DB（既に `core.project` がある）なら `000_baseline` を入れる。
#   2. `migrations/*.sql` をファイル名順に見て、**記録が無いものだけ**当てる。
#   3. 当てたものを 1 ファイル 1 行で記録する。
#
# 何をしないか:
#   - 適用済みファイルの再実行（非冪等な ALTER があるため）。
#   - 失敗の握り潰し（途中で落ちたらそのまま止まる）。
set -euo pipefail

DB="${1:-}"
if [ -z "$DB" ]; then
  echo "usage: $0 <database>" >&2
  exit 2
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MIGRATIONS="$ROOT/migrations"
BASELINE_MAX="031"

say() { printf '%s\n' "$*"; }

applied() {
  psql -d "$DB" -tAc "SELECT count(*) FROM core.schema_migration WHERE filename = '$1';"
}

record() {
  psql -d "$DB" -q -c "INSERT INTO core.schema_migration (filename) VALUES ('$1')
                       ON CONFLICT (filename) DO NOTHING;"
}

has_table() {
  [ "$(psql -d "$DB" -tAc "SELECT to_regclass('$1') IS NOT NULL;")" = "t" ]
}

applied_count=0
skipped_count=0

if ! has_table 'core.schema_migration'; then
  if has_table 'core.project'; then
    # 既存 DB。032 を当てて、001〜031 は「確認済み」として baseline で覆う。
    psql -d "$DB" -v ON_ERROR_STOP=1 -q -f "$MIGRATIONS/032_schema_migration.sql"
    # ループでもう一度当てないよう、ここで記録する。
    record '032_schema_migration.sql'
    applied_count=1
    say "既存 DB と判定: 032 を適用し、001〜${BASELINE_MAX} は baseline として扱います。"
    say "  ※ 心当たりが無い場合は scripts/verify-db.sh で作り直してください。"
  else
    # 空 DB に近い。001 から順に全部当てる（032 もループで当たる）。
    say "新規 DB と判定: 001 から順に適用します。"
  fi
fi

# 032 より前を当てた時点では記録表が無い。後でまとめて記録する。
pending_record=()
for migration in "$MIGRATIONS"/*.sql; do
  name="$(basename "$migration")"
  number="${name%%_*}"

  if has_table 'core.schema_migration' && [ "$(applied "$name")" = "1" ]; then
    skipped_count=$((skipped_count + 1))
    continue
  fi

  # baseline は 031 までを覆う。**032 以降は必ず個別に記録する**ので、
  # ここで飛ばしてよいのは baseline より古いファイルだけ。
  if has_table 'core.schema_migration' && [ "$(applied '000_baseline')" = "1" ] \
     && [[ "$number" < "$BASELINE_MAX" || "$number" == "$BASELINE_MAX" ]]; then
    skipped_count=$((skipped_count + 1))
    continue
  fi

  say "適用: $name"
  psql -d "$DB" -v ON_ERROR_STOP=1 -q -f "$migration"
  if has_table 'core.schema_migration'; then
    record "$name"
  else
    pending_record+=("$name")
  fi
  applied_count=$((applied_count + 1))
done

# 記録表ができた後に、それ以前に当てた分をまとめて記録する。
if has_table 'core.schema_migration' && [ ${#pending_record[@]} -gt 0 ]; then
  for name in "${pending_record[@]}"; do
    record "$name"
  done
fi

say ""
say "適用 $applied_count 件 / スキップ $skipped_count 件"
