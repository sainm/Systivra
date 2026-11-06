# Changelog

## 2026-11-06 — P0 docs/contract alignment

- `P1-API-02` の Phase 1 範囲を `detail/04` §1.1/§1.2/§1.4 に確定し、`documents:bulk-import` を Phase 2 へ明示的に延期。
- `detail/02` の migration order を実ファイルに合わせ、`009_rename_corpus` / `010_code` 以降へ再整理。
- `detail/04` と `06-API` に実装済みの `content`、ローカル upload PUT、project-scoped `audit-logs` を追加。
- `06-API` の未実装端点を Phase 2 と明記し、README / handoff / walking-skeleton に同じ境界を反映。

## 2026-11-06 — Phase 1: Platform Foundation + Walking Skeleton

### Added

- 文書 API: upload-url（HMAC 署名 PUT）、登録、カーソル一覧、論理削除、原本取得。
- `corpus` schema: document / revision / parse_run / doc_node / chunk（32 HASH 分区）/
  embedding_1024（32 分区 + HNSW）/ evidence / retrieval_log / index_build。
- worker: PDF 抽出（`pdf-extract`）、DocNode、800 token / 80 overlap chunk、
  1024 次元ローカル埋め込み、per-page checkpoint 再開。
- knowledge API: 検索（FTS + vector 融合、32 分区 fan-out）、chunk、evidence、job、SSE。
- フロント 5 画面（ログイン / プロジェクト / 設計書一覧・取込 / 文書詳細 / 検索 + エビデンスドロワー）。
  PDF.js のテキスト層で quote を矩形ハイライト。ja / en / zh-CN。
- OpenAPI を utoipa から生成（`contracts/api/openapi.json`）し、CI で一致検査。
- `docker-compose.yml` に web（nginx）と共有 `/data` ボリュームを追加。

### Changed

- 文書 schema 名を `kb` から `corpus` に統一（設計正本・コード・ロール権限・compose 初期化）。
  `009_rename_corpus.sql` は旧 DB 救済用。新規 DB では no-op。
- 検索語の正規化: PDF 由来の制御文字を除去し、NFKC で康熙部首などを統一。

### Verification

- `cargo fmt --check` / `cargo clippy -D warnings` / `cargo test --workspace` 緑。
- `scripts/verify-db.sh` 全項目緑（32 分区、HNSW 32、RLS 9 表、`core.project_of`、越境 0 件）。
- 受入 A-02〜A-10 実測。A-01 は Docker / compose provider 不在のため、同一 Dockerfile・初期化 SQL・
  環境変数で Podman に 4 サービスを起動し、コンテナ経由の E2E（70 矩形ハイライト）まで検証。
- 詳細: `docs/phase1/walking-skeleton.md`、引き継ぎ: `docs/phase1/handoff.md`。
