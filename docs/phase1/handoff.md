# Phase 1 → Phase 2 引き継ぎ

## Phase 2 で置き換える場所

| 領域 | 現在 | 置き換え先 |
|---|---|---|
| PDF 解析 | `crates/worker/src/pdf.rs`（`pdf-extract`） | `docling-serve` HTTP。`ParseTarget` と `save_parse_page` の契約は維持 |
| chunk | `crates/engine-rag/src/lib.rs` の `chunk_page`（800 token / 80 overlap、ページ単位） | 構造認識 chunk、親子 chunk、Excel 12 行ブロック |
| embedding | `engine-rag::embed`（決定的ハッシュ、1024 次元） | `EmbeddingProvider` 経由のモデル。`corpus.embedding_1024` の `model_id` で切替 |
| 検索融合 | `engine_rag::fuse`（fulltext + vector の固定重み） | pg_bigm / Lindera / RRF / rerank。`SearchResponse.degraded` の意味を維持 |
| 座標 | `locator.bbox=null`、UI が PDF.js テキスト層で矩形計算 | docling のページ座標。UI は `bbox` があれば優先 |
| 原本保存 | `api/src/upload.rs` + `app::put_object`（ローカル FS、HMAC PUT URL） | S3 presigned URL。`upload_url` / `object_key` の契約は不変 |
| マイグレーション | ファイル適用（`psql` / compose initdb） | sqlx `Migrator` に移す場合は履歴を再基準化する（下記） |

## 触らない場所（Phase 2 でも不変）

- 認証・認可: `api/src/auth/`、`domain/src/permission.rs`、`003_authz.sql`
- テナント境界: `SET LOCAL app.project_id`、`corpus` の RLS、`core.project_of`
- API エンベロープ: `application/problem+json`、`code` / `trace_id` / `errors[]`
- ID 規約: UUID。表示コードを API の参照キーにしない
- ジョブ: `core.job` / `job_checkpoint` / `job_event`、SSE の `Last-Event-ID` と `done` 終端
- 証拠: `corpus.evidence` の `quote` 部分文字列検証と `sha256(NFC(quote))`
- 監査: 追記専用、`core.audit_log`
- 画面外壳: `apps/web/src/components/AppShell.tsx`、`App.tsx`、i18n キー規約

## 既知の制約・注意

1. **マイグレーション履歴を書き換えた**（`kb` → `corpus`）。未コミットの旧作業ツリーで作った DB は
   そのままでは再適用できない。開発 DB は作り直す。`009_rename_corpus.sql` は旧 `kb` が残る DB の
   救済用で、新規 DB では no-op。将来 sqlx `Migrator` に移す場合は、この履歴を基準に開始する。
2. `docker compose up` は開発機に Docker が無いため未実行。`docker-compose.yml` と
   `apps/web/Dockerfile` / `apps/web/nginx.conf` は用意済み。Podman では compose provider が無く、
   同一イメージを `podman build` / `podman run` で検証する。
3. `GET /api/v1/documents/{did}/content` は Phase 1 で追加した契約（`detail/04` §1.6）。
4. SSE はサーバー実装済み。画面はポーリングを使う（`DocumentsPage` / `DocumentPage`）。
   E2E の受入は `Last-Event-ID` を直接検証している。
5. `ParseTarget` の入力は `core.job.input` の JSON。キー名を変えると再開できない。
6. `corpus.chunk` / `corpus.embedding_1024` は 32 分区。検索は全分区 fan-out を前提にしている。
   分区数を変えるときは両方を同時に変える。
7. `documents:bulk-import`、改版履歴、構造木、索引再構築、ジョブキャンセルは Phase 1 未実装。
   Phase 2 で追加する。Phase 1 の文書 API は単一 PDF の upload-url / 登録 / 一覧 / 削除 / 原本取得。
8. 009 は `009_rename_corpus.sql` が使用済み。Phase 2 の code schema は `010_code` から開始する。
