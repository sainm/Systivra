# Systivra

Software Engineering Intelligence Platform — 設計書・ソースコード・データベース・テストを「証拠」でつなぎ、改修の影響範囲を計算・検証・監査できるようにする分析基盤。

## 設計文書

- 正式版（日本語）: [`docs/design/ja/00-全体設計.md`](docs/design/ja/00-全体設計.md)
- 一致性監査（履歴）: [`docs/design/review-2026-11-05.md`](docs/design/review-2026-11-05.md)

## 状態

Phase 0（設計）完了。**Phase 1（Platform Foundation + Walking Skeleton）完了**。
受入 A-02〜A-10 は実測済み。A-01 の `docker compose up` だけは開発機に Docker / compose provider が無いため
未実行だが、同じ Dockerfile・初期化 SQL・環境変数で Podman に db / api / worker / web を起動し、
コンテナ経由の E2E まで通している（詳細と降格は [`docs/phase1/walking-skeleton.md`](docs/phase1/walking-skeleton.md)）。

できていること：

- PDF 1 冊を **アップロード → 解析 → chunk → 索引 → 検索 → 該当ページの証拠ハイライト** まで通す
- 文書 API … upload-url / 登録 / カーソル一覧 / 論理削除 / 原本取得。重複は `CONFLICT_ALREADY_IMPORTED`。
  `documents:bulk-import` は Phase 2（Phase 1 は単一 PDF の取込）
- 解析 … プロセス内 PDF 抽出 → DocNode → 800 token / 80 overlap chunk → 1024 次元ベクトル
- 検索 … `corpus.chunk` / `corpus.embedding_1024` の 32 分区 fan-out、FTS + ベクトル融合
- 証拠 … `quote` の部分文字列検証、`sha256(NFC(quote))`、RAG 結果から原文ページへ遷移
- ジョブ … 冪等 enqueue、`SKIP LOCKED`、checkpoint 再開、SSE（`Last-Event-ID` / `done`）
- 監査 … 登録・解析・削除を `trace_id` 付きで追記。RLS と 3 ロールで分離
- 画面 … ログイン / プロジェクト / 設計書一覧・取込 / 文書詳細 / 検索 + エビデンスドロワー（ja / en / zh-CN）
- テスト … `cargo test --workspace`、`clippy -D warnings`、`fmt --check`、OpenAPI 一致、i18n キー一致が緑

```bash
cargo test --workspace
PGHOST=localhost PGUSER=postgres ./scripts/verify-db.sh   # 空 DB に 001〜009 を適用して RLS / 分区 / HNSW を確認
cargo run -p api --bin openapi -- --check                 # 生成 spec とコミット済み spec の一致
cd apps/web && npm ci && npm run check:i18n && npm run build
```

### ローカル起動

```bash
# DB（pgvector が必要）
createdb systivra
for f in migrations/0*.sql; do psql -d systivra -v ON_ERROR_STOP=1 -f "$f"; done
psql -d systivra -f deployment/postgres/roles.sql
./scripts/seed-dev.sh          # alice / SystivraDemo1!（開発専用）

# API（Vite の proxy 先は 18080）
PORT=18080 DATABASE_URL=postgres://systivra_app@localhost:5432/systivra \
SESSION_SECRET=$(openssl rand -hex 32) SESSION_COOKIE_SECURE=false \
PUBLIC_BASE_URL=http://127.0.0.1:18080 SYSTIVRA_DATA_DIR=$PWD/data \
  cargo run --bin systivra-api

# worker（別ロール）
DATABASE_URL=postgres://systivra_scheduler@localhost:5432/systivra \
WORKER_TICK_SEC=1 SYSTIVRA_DATA_DIR=$PWD/data cargo run --bin systivra-worker

# 画面
cd apps/web && npm run dev     # http://localhost:15173
```

ヘルスチェック：`curl -i localhost:18080/healthz`（DB 非依存）/ `readyz`（DB 依存）。
`docker-compose.yml` は db + api + worker + web。Docker がある環境では `docker compose up` で
`http://localhost:3000` にログイン画面が出る（A-01 の条件付き項目）。

## 構成

```text
crates/                    13 crate（domain / contracts / store / providers /
                           engine-rag / engine-graph / engine-code / engine-impact /
                           engine-test / runtime / app / api / worker）
migrations/                001〜009（core / corpus / job_event / project_of / rename）
deployment/postgres/       roles.sql（3 ロール + 権限。冪等）
deployment/docker/         API / worker の Dockerfile
apps/web/                  Vite + React + Tailwind。Dockerfile は nginx 配信
docker-compose.yml         db + api + worker + web
scripts/verify-db.sh       空 DB に全マイグレーションを適用し RLS / 分区 / HNSW を確認
scripts/seed-dev.sh        開発ユーザーとプロジェクト
scripts/make-fixture.sh    100 ページ日本語 PDF fixture
apps/web/scripts/e2e.mjs   ログイン → 取込 → 解析 → 検索 → 原文ハイライトのブラウザ受入
docs/phase1/               受入記録と Phase 2 への引き継ぎ
.github/workflows/ci.yml   fmt / clippy / test / DB 検証 / OpenAPI / フロント / 文書検査

docs/design/ja/            設計正本（日本語）
  00〜09                  全体設計 9 部
  reference/              付録 3 本（日本語検索 / OSS 評価 / 規模算定）
  detail/                 実装直結の詳細設計（01 Excel 取込 〜 09 テスト実行）
docs/design/               中国語版（内部参照・凍結）+ 一致性監査 review-2026-11-05.md
```

> **日本語版が唯一の正本**。2026-11-05 以降の設計変更は `ja/` にのみ反映する。中国語版は当時の意思決定の記録として凍結し、数字・版・契約の照会には使わない。
