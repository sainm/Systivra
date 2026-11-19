#!/usr/bin/env bash
# docker-compose.yml の静的検査（Docker デーモン不要）。
#
#   ./scripts/check-compose.sh
#
# `docker compose up` でしか分からない種類の事故を、手元で先に潰す:
#   - バインドマウントの元パスが存在しない（タイポ・移動）
#   - マイグレーションを 1 ファイルずつ列挙している（追加時に必ず漏れる）
#   - Dockerfile のパスが無い
#   - init スクリプトが実行可能でない
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
COMPOSE="$ROOT/docker-compose.yml"
[ -f "$COMPOSE" ] || { echo "docker-compose.yml が無い"; exit 1; }

FAIL=0
ok() { printf '  \033[32mok\033[0m   %s\n' "$*"; }
no() { printf '  \033[31mFAIL\033[0m %s\n' "$*"; FAIL=1; }
say() { printf '%s\n' "$*"; }

say "== バインドマウント =="
# `- ./path:/target[:ro]` の ./path 部分だけを取り出す。
# BSD sed には \s が無い。空白クラスを明示する。
grep -oE '^[[:space:]]+- \./[^:]+:' "$COMPOSE" | sed -E 's/^[[:space:]]+- //; s/:$//' | while read -r rel; do
  if [ -e "$ROOT/$rel" ]; then
    printf '  \033[32mok\033[0m   %s\n' "$rel"
  else
    printf '  \033[31mFAIL\033[0m %s（存在しない）\n' "$rel"
    echo "missing" >> /tmp/check-compose-fail
  fi
done
[ -f /tmp/check-compose-fail ] && { FAIL=1; rm -f /tmp/check-compose-fail; }

say "== マイグレーションの渡し方 =="
if grep -qE '^[[:space:]]+- \./migrations/[0-9]+_.*\.sql:' "$COMPOSE"; then
  no "マイグレーションを列挙している（追加時に漏れる。ディレクトリごと渡すこと）"
else
  ok "ファイル列挙なし"
fi
if grep -qE '^[[:space:]]+- \./migrations:/migrations:ro' "$COMPOSE"; then
  ok "ディレクトリをマウントしている"
else
  no "./migrations のディレクトリマウントが無い"
fi
if [ -x "$ROOT/deployment/postgres/init/10-apply-migrations.sh" ]; then
  ok "init スクリプトが実行可能"
else
  no "deployment/postgres/init/10-apply-migrations.sh が実行可能でない"
fi
# init スクリプトが実際に全ファイルを回しているか（1 つでも列挙していたら疑う）。
# パスは環境変数で差し替えられる（ローカル検証用）ので、変数でも直書きでも受ける。
if grep -qE 'for migration in "?\$?\{?MIGRATIONS_DIR\}?"?/\*\.sql|for migration in /migrations/\*\.sql' "$ROOT/deployment/postgres/init/10-apply-migrations.sh"; then
  ok "init スクリプトは全ファイルを走査する"
else
  no "init スクリプトが *.sql を走査していない"
fi

say "== Dockerfile =="
grep -oE 'dockerfile: \S+' "$COMPOSE" | sed 's/dockerfile: //' | while read -r rel; do
  if [ -f "$ROOT/$rel" ]; then
    printf '  \033[32mok\033[0m   %s\n' "$rel"
  else
    printf '  \033[31mFAIL\033[0m %s（存在しない）\n' "$rel"
    echo "missing" >> /tmp/check-compose-fail
  fi
done
[ -f /tmp/check-compose-fail ] && { FAIL=1; rm -f /tmp/check-compose-fail; }

say "== 必須の環境変数 =="
for key in SESSION_SECRET SYSTIVRA_DATA_DIR PUBLIC_BASE_URL; do
  if grep -q "$key:" "$COMPOSE"; then ok "$key"; else no "$key が無い"; fi
done

say ""
if [ "$FAIL" -eq 0 ]; then
  say "全て通過"
else
  say "失敗あり"
fi
exit "$FAIL"
