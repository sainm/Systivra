# Phase 1 受入記録（Walking Skeleton）

日付：2026-11-06
対象：`docs/design/ja/detail/05-phase1-taskboard.md` §6 A-01〜A-10

環境：macOS arm64 / PostgreSQL 18.4 + pgvector 0.8.2 / Rust 1.99 / Node 26 / Google Chrome（Playwright 駆動）

## 判定

| # | 判定 | 証拠 |
|---|---|---|
| A-01 | **可（等価検証）** | Docker / compose provider が無いため `docker compose up` 自体は未実行。`docker-compose.yml` と同じ Dockerfile・初期化 SQL（001〜009 + roles）・ネットワーク・環境変数で Podman 5.8 に db / api / worker / web を起動し、`/healthz` 200・ログイン画面 200・**コンテナ経由の E2E（取込 → 解析 → 検索 → 原文 → 70 矩形ハイライト）** まで通過。compose YAML は構文検証済み |
| A-02 | **可** | 100 ページ PDF → 100 chunk。通常解析は job 作成〜完了 **1.70〜2.65 秒**（5 分以内） |
| A-03 | **可** | 「住所チェックの桁数と文字種」で top-12 が返り、1 位は 37 ページ。`latency_ms` は 20 回で min 18ms / median 18ms / **P95 49ms** / max 71ms（受入上限 300ms） |
| A-04 | **可（降格あり）** | 検索結果から 37 ページを開き、PDF.js テキスト層で **70 個の矩形**をハイライト。API の `bbox` は `null`（§降格） |
| A-05 | **可** | 41/100 ページ・checkpoint 41 で worker を kill。再起動後 42〜100 を処理し、chunk **100 件 / distinct 100 件**、checkpoint 100 |
| A-06 | **可** | 途中の `Last-Event-ID` から再接続し、**95 イベント + done 1 件**。DB の該当イベント集合と完全一致、id は単調増加 |
| A-07 | **可** | 同じ `Idempotency-Key` の解析は同一 `job_id` を返し chunk 100 のまま。同一 `content_hash` の登録は `CONFLICT_ALREADY_IMPORTED`（409） |
| A-08 | **可** | 監査ログに `document.register` / `document.parse` / `document.parse.completed`。ユーザー操作は `trace_id` 付き |
| A-09 | **可** | 別プロジェクトの文書 ID・一覧は 404。RLS 未設定時は corpus も 0 件（`scripts/verify-db.sh`） |
| A-10 | **可** | 生成 spec と `contracts/api/openapi.json` が一致（`cargo test -p api --test openapi_contract`）。`oasdiff diff` は `{}`（exit 0）。`openapi-typescript` 再生成でフロント型に差分なし |

## 実測ログ

```text
# A-02（通常解析）
job fdbeffbc… 23.75s（うち 300ms/頁の意図的遅延 20s）
job 64faba5a…  2.24s
job 64e5592a…  1.70s

# A-03（20 回）
min=18ms / median=18ms / P95=49ms / max=71ms

# A-05
kill 時: status=RUNNING files_done=41 checkpoint=41 chunks=41
再開後: status=SUCCEEDED files_done=100 checkpoint=100 chunks=100 distinct=100
```

## 降格（明示）

| 項目 | 採用した方式 | 理由 | 復帰条件 |
|---|---|---|---|
| PDF 解析 | `docling-serve` ではなくプロセス内 `pdf-extract` | 開発機に Docker が無く docling を起動できない | Phase 2 でコンテナ配布手段を確立 |
| ハイライト座標 | API は `bbox=null`、UI は PDF.js テキスト層で矩形を計算 | 抽出器が座標を返さない | docling の座標付き出力へ切替 |
| Embedding | 1024 次元の決定的ローカルハッシュ | 外部 Provider / モデル配布が閉域で使えない | Provider 疎通後に差し替え |
| 日本語検索 | 文字 unigram/bigram + カタカナ語。FTS は `simple` | Lindera 辞書を同梱していない | Lindera / pg_bigm / RRF |
| 原本保存 | ローカル FS + HMAC 署名 PUT URL | S3/MinIO が無い | S3 presigned URL |
| 構造木 API | 未実装（Phase 1 の画面は PDF ビューアのみ） | A-01〜A-10 に含まれない | Phase 2 |
| 一括取込 | 未実装（`documents:bulk-import` は Phase 2） | Phase 1 は単一 PDF の Walking Skeleton | Phase 2 |

検索応答の `degraded` は `embedding_local_fallback` / `bbox_unavailable` を返す。
`rerank=true` は `CAPABILITY_UNSUPPORTED`（409）。

## 再実行

```bash
# 1. 空 DB（corpus schema を含む）
createdb systivra
for f in migrations/0*.sql; do psql -d systivra -v ON_ERROR_STOP=1 -f "$f"; done
psql -d systivra -f deployment/postgres/roles.sql
./scripts/seed-dev.sh

# 2. API / worker / web
PORT=18080 DATABASE_URL=postgres://systivra_app@localhost:5432/systivra \
SESSION_SECRET=$(openssl rand -hex 32) SESSION_COOKIE_SECURE=false \
PUBLIC_BASE_URL=http://127.0.0.1:18080 SYSTIVRA_DATA_DIR=$PWD/data \
  cargo run --bin systivra-api

DATABASE_URL=postgres://systivra_scheduler@localhost:5432/systivra \
WORKER_TICK_SEC=1 SYSTIVRA_DATA_DIR=$PWD/data cargo run --bin systivra-worker

cd apps/web && npm ci && npm run dev   # http://localhost:15173

# 3. 受入スモーク（ログイン → 取込 → 解析 → 検索 → 原文 → 高亮）
cd apps/web && WEB_BASE=http://localhost:15173 node scripts/e2e.mjs
```

初期アカウント：`alice` / `SystivraDemo1!`（`scripts/seed-dev.sh`、開発専用）。
