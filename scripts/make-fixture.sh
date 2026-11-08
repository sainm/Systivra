#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="$ROOT/tests/fixtures"
HTML="$OUT_DIR/walking_skeleton_100p.html"
PDF="$OUT_DIR/walking_skeleton_100p.pdf"
CHROME="${CHROME_BIN:-/Applications/Google Chrome.app/Contents/MacOS/Google Chrome}"

mkdir -p "$OUT_DIR"

{
  printf '%s\n' '<!doctype html><html lang="ja"><head><meta charset="utf-8">'
  printf '%s\n' '<style>'
  printf '%s\n' '@page { size: A4; margin: 18mm; }'
  printf '%s\n' 'body { font-family: "Hiragino Sans", "Yu Gothic", sans-serif; font-size: 11pt; line-height: 1.7; color: #1b1f23; }'
  printf '%s\n' '.page { page-break-after: always; }'
  printf '%s\n' '.page:last-child { page-break-after: auto; }'
  printf '%s\n' 'h1 { font-size: 18pt; } h2 { font-size: 13pt; margin-top: 1.2em; }'
  printf '%s\n' 'table { border-collapse: collapse; width: 100%; } td, th { border: 1px solid #999; padding: 4pt 6pt; }'
  printf '%s\n' '</style></head><body>'
  for i in $(seq 1 100); do
    printf '<section class="page" id="page-%s">\n' "$i"
    printf '<h1>第%s章 顧客管理機能 詳細設計</h1>\n' "$i"
    printf '<p>本書は顧客管理機能の詳細設計を定義する。対象画面は顧客一覧、顧客編集、住所検索である。ページ %s / 100。</p>\n' "$i"
    printf '<h2>画面項目定義</h2>\n'
    if [ "$i" -eq 37 ]; then
      printf '<p>住所チェックの桁数と文字種: 住所は全角20文字以内、必須項目とする。郵便番号は半角数字7桁でハイフンを含めない。</p>\n'
      printf '<p>顧客コードは半角英数字10桁、重複を禁止する。電話番号は半角数字とハイフンのみ許可する。</p>\n'
    else
      printf '<p>項目 %s-01: 顧客コード。入力形式は半角英数字10桁。必須。重複チェックあり。</p>\n' "$i"
      printf '<p>項目 %s-02: 顧客名。入力形式は全角40文字以内。必須。前後の空白を除去する。</p>\n' "$i"
    fi
    printf '<table><tr><th>項目</th><th>型</th><th>制約</th></tr>\n'
    printf '<tr><td>顧客コード</td><td>文字列</td><td>10桁・必須・一意</td></tr>\n'
    printf '<tr><td>顧客名</td><td>文字列</td><td>40文字以内</td></tr>\n'
    printf '<tr><td>住所</td><td>文字列</td><td>全角20文字以内</td></tr></table>\n'
    printf '<h2>業務ルール</h2>\n'
    printf '<p>顧客の削除は論理削除とし、履歴を保持する。検索は顧客コードと顧客名の部分一致を許可する。監査ログには操作者、時刻、対象顧客コードを記録する。</p>\n'
    printf '<p>ページ固有の検証語: 検証キー-%s-顧客管理。この語はページ %s にのみ出現する。</p>\n' "$i" "$i"
    printf '</section>\n'
  done
  printf '%s\n' '</body></html>'
} > "$HTML"

"$CHROME" --headless --disable-gpu --no-pdf-header-footer --print-to-pdf="$PDF" "file://$HTML" >/dev/null 2>&1
printf 'generated %s (%s bytes)\n' "$PDF" "$(wc -c < "$PDF" | tr -d ' ')"
