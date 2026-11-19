#!/usr/bin/env bash
# Dockerfile / nginx の静的検査（Docker デーモン不要）。
#
#   ./scripts/check-dockerfile.sh
#
# 目的は「`docker build` して初めて分かる」種類の事故を手元で潰すこと:
#   - 新しい crate を足したのに Dockerfile のキャッシュ層が知らない
#   - 実行ファイル名が Cargo.toml と食い違う
#   - nginx の proxy_pass 先が compose のサービス名 / ポートと合わない
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DOCKERFILE="$ROOT/deployment/docker/Dockerfile"
COMPOSE="$ROOT/docker-compose.yml"
NGINX="$ROOT/apps/web/nginx.conf"

FAIL=0
ok() { printf '  \033[32mok\033[0m   %s\n' "$*"; }
no() { printf '  \033[31mFAIL\033[0m %s\n' "$*"; FAIL=1; }
say() { printf '%s\n' "$*"; }

[ -f "$DOCKERFILE" ] || { echo "Dockerfile が無い"; exit 1; }

say "== crates が Dockerfile に載っているか =="
# キャッシュ層は Cargo.toml を個別に COPY し、stub の lib.rs を作る。
# どちらか欠けると、crate を足したときに **イメージのビルドが落ちる**。
for dir in "$ROOT"/crates/*/; do
  name="$(basename "$dir")"
  [ -f "$dir/Cargo.toml" ] || continue
  if grep -q "crates/$name/Cargo.toml" "$DOCKERFILE"; then
    ok "COPY crates/$name/Cargo.toml"
  else
    no "crates/$name/Cargo.toml が COPY されていない"
  fi
done
# stub 生成ループ（\`for d in a b c; do\`）に載っているか。
# バックスラッシュ継続とセミコロンを空白に潰してから単語一致で見る。
STUB_TOKENS="$(sed -n '/for d in/,/; do/p' "$DOCKERFILE" | tr -d '\\;' | tr '\n' ' ')"
for dir in "$ROOT"/crates/*/; do
  name="$(basename "$dir")"
  [ -f "$dir/Cargo.toml" ] || continue
  if printf '%s\n' "$STUB_TOKENS" | grep -qw "$name"; then
    ok "stub 生成に $name"
  elif grep -q "crates/$name/src/main.rs" "$DOCKERFILE"; then
    ok "stub（bin）に $name"
  else
    no "$name の stub 生成が見当たらない（依存キャッシュ層でビルドが落ちる）"
  fi
done

say "== 実行ファイル名 =="
for bin in systivra-api systivra-worker; do
  crate="${bin#systivra-}"
  if grep -q "name = \"$bin\"" "$ROOT/crates/$crate/Cargo.toml" 2>/dev/null \
     || ls "$ROOT/crates/$crate/src/main.rs" >/dev/null 2>&1; then
    ok "$bin は crates/$crate に存在"
  else
    no "$bin が crates/$crate に見当たらない"
  fi
  if grep -q "COPY --from=builder /src/target/release/$bin" "$DOCKERFILE"; then
    ok "$bin を最終イメージへコピー"
  else
    no "$bin が最終イメージにコピーされていない"
  fi
done

say "== nginx と compose の整合 =="
if [ -f "$NGINX" ]; then
  ok "apps/web/nginx.conf"
  upstream="$(grep -oE 'proxy_pass http://[a-z0-9_-]+:[0-9]+' "$NGINX" | head -1 | sed -E 's#.*http://##')"
  host="${upstream%%:*}"
  port="${upstream##*:}"
  if grep -qE "^  $host:" "$COMPOSE"; then
    ok "proxy_pass の宛先 $host が compose にある"
  else
    no "proxy_pass の宛先 $host が compose に無い"
  fi
  if grep -q "PORT: \"$port\"" "$COMPOSE"; then
    ok "api の PORT は $port"
  else
    no "api の PORT が $port になっていない（proxy_pass とずれる）"
  fi
  # SPA のフォールバックが無いと、画面遷移で 404 になる。
  if grep -q 'try_files $uri /index.html' "$NGINX"; then
    ok "SPA フォールバック"
  else
    no "try_files のフォールバックが無い"
  fi
else
  no "apps/web/nginx.conf が無い"
fi

say ""
if [ "$FAIL" -eq 0 ]; then say "全て通過"; else say "失敗あり"; fi
exit "$FAIL"
