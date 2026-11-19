#!/usr/bin/env bash
# 初回起動時にマイグレーションを**ファイル名順に全部**当てる。
#
# 以前は docker-compose.yml に 1 ファイルずつ列挙していたため、
# 031〜033 の追加時に漏れ、`docker compose up` した DB だけ
# 列が足りない状態になっていた（列挙は必ず漏れる）。
# ここではディレクトリを丸ごと見るので、追加しても編集不要。
set -euo pipefail

# 既定はコンテナ内のパス。ローカル検証のときだけ差し替える。
MIGRATIONS_DIR="${MIGRATIONS_DIR:-/migrations}"

if [ ! -d "$MIGRATIONS_DIR" ]; then
  echo "[init] $MIGRATIONS_DIR がマウントされていません" >&2
  exit 1
fi

count=0
for migration in "$MIGRATIONS_DIR"/*.sql; do
  [ -e "$migration" ] || continue
  echo "[init] apply $(basename "$migration")"
  psql -v ON_ERROR_STOP=1 -q -f "$migration"
  count=$((count + 1))
done

if [ "$count" -eq 0 ]; then
  echo "[init] マイグレーションが 1 つも無い" >&2
  exit 1
fi

echo "[init] applied $count migrations"
