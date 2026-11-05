# 詳細設計 05：Phase 1 タスクボード（実装直結）

対象：Phase 1「Platform Foundation + Walking Skeleton」（**4 週**）。本稿は `08-実装計画.md` Phase 1 を、Sprint の看板としてそのまま使える粒度に落としたもの。

**正本の優先順位**（矛盾したら上位を採る）：

1. `00-全体設計.md` の A-01〜A-10 / D-01〜D-07（前提と確定済み判断）
2. `detail/02-database-ddl.md`（DDL）、`detail/04-api-contract.md`（API 契約）
3. `detail/03-performance.md`（SLO）、`08-実装計画.md`（Phase 範囲）
4. 本稿（タスク分割・担当・完了条件）

> 本稿は**設計を変えない**。設計を変えたい場合は ADR を追加し、上位文書を先に更新する。

---

## 1. 目的と「完了」の定義

**目的**：1 冊の PDF を「アップロード → 解析 → chunk → 索引 → 検索 → 該当ページの証拠表示」まで 1 本の経路で通し、以降の Phase が**置き換えだけで済む**骨格を作る。

**やらないこと（明示的に）**：

| やらない | 理由 | 導入 Phase |
|---|---|---|
| XLSX 解析 | 60,000 冊の本命だが、骨格の証明とは独立 | Phase 2（`detail/01`） |
| pg_bigm / PGroonga / RRF / rerank | 日本語検索の品質は Phase 2 の評価課題 | Phase 2 |
| グラフ / Ontology / Neo4j 投影 | 影響分析に不要な間に複雑性を持ち込まない | Phase 3（`detail/07`） |
| Java / JSP / OpenRewrite | 別ワーカー、別言語 | Phase 4（`detail/06`） |
| Agent / MCP / Workflow | 能力ゲートウェイは Phase 7 | Phase 7 |
| 自動コード変更 | **D-04 により禁止**。将来導入も承認必須 | — |

**完了の定義**：§6 の受入 A-01〜A-10 がすべて通り、`08-実装計画.md` Phase 1 の 4 条件を満たすこと。デモが 1 本でも途中で止まるなら未完了。

---

## 2. 体制と週次容量

`08-実装計画.md` §1.1 の確定体制をそのまま使う。

| 役割 | 人数 | Phase 1 の主担当 |
|---|---|---|
| Rust バックエンド | 2（1 名はアーキテクト兼務） | BE-1 / BE-2 |
| フロントエンド | 1 | FE |
| 設計 | 0.5 | DS（画面・用語・契約レビュー） |
| QA | 0（Phase 5 から 0.5→1） | 受入は BE / FE が相互に実施 |
| Java | 0（Phase 4 に 0.5） | — |

週あたり名目容量（5 日/週）：BE 10 人日、FE 5 人日、DS 2.5 人日。4 週で **BE 40 / FE 20 / DS 10**。実効 80% で見ると BE 32 / FE 16 / DS 8。

本稿の見積り合計は **BE 34 / FE 12 / DS 4**（§4 の各表の合計）。名目容量に対して BE は 6 人日（15%）のバッファ、FE は余裕あり（設計レビューと受け入れ支援に回す）。**実効 80% の 32 人日を前提にすると BE は 2 人日超過**するため、W4 の `P1-FIX-01` を Phase 2 の W1 に送る前提で計画する。これを超える遅れが出た場合は §7 の削減ラインに従う。

---

## 3. タスク ID 規約と依存グラフ

`P1-<領域>-<連番>`。領域は `ENV / DB / API / JOB / PARSE / RAG / EVID / FE / QA / DOC`。

```text
P1-ENV-01 Cargo workspace / compose / CI
      │
      ├──> P1-DB-01  マイグレーション 001〜002・分区・RLS
      │        │
      │        ├──> P1-API-01 認証 / project_id 注入 / エラー envelope
      │        │        │
      │        │        ├──> P1-API-02 upload-url / 文書登録 / 一覧 / 論理削除
      │        │        │        │
      │        │        │        ├──> P1-JOB-01 job / checkpoint / SSE
      │        │        │        │        │
      │        │        │        │        ├──> P1-PARSE-01 docling-serve（PDF）
      │        │        │        │        │        │
      │        │        │        │        │        ├──> P1-RAG-01 固定長 chunk / embedding / ANN
      │        │        │        │        │        │        │
      │        │        │        │        │        │        └──> P1-EVID-01 証拠 / locator / ハイライト
      │        │        │        │        │        │                 │
      │        │        └──> P1-FE-01 ログイン / 一覧 / 詳細 / 検索 ───┘
      │        │                        │
      │        └──> P1-DOC-01 OpenAPI 生成 / 契約テスト
      │                                 │
      └────────────────────────────> P1-QA-01 統合テスト / 受入 / 降格判断
```

**並列可能な境界**：

- BE-1 が `DB / API / JOB`、BE-2 が `PARSE / RAG / EVID` を W1 から並走できる。境目は `detail/04-api-contract.md` の DTO と `contracts/` の JSON Schema で固定する。
- FE は `contracts/api/openapi.json` の**スナップショット**に対して W2 からモックで作り始める。実 API 完成を待たない。
- `P1-QA-01` は W4 だけでなく W2 末に 1 回「貫通チェック」を入れる（W4 まで放置すると手戻りが 4 週分になる）。

---

## 4. 週次タスク表

各表の「人日」は**そのタスク単体の作業量**。合計は §2 の容量に一致させる（BE 34 / FE 12 / DS 4）。

### W1：基盤（BE 11 / FE 2 / DS 2）

| ID | タスク | 担当 | 依存 | 成果物 | 完了条件（DoD） | 人日 |
|---|---|---|---|---|---|---|
| P1-ENV-01 | 13 crate の Cargo workspace、compose、CI | BE-1 | — | `Cargo.toml` / `docker-compose.yml` / `.github/workflows/ci.yml` | `docker compose up` で `/healthz` が 200。CI で `cargo test` が緑 | 2 |
| P1-DB-01 | マイグレーション 001〜002 | BE-1 | ENV-01 | `migrations/001_core.sql` / `002_core_ops.sql` | §5.1 の全条件 | 3 |
| P1-API-01 | 認証・認可・エラー基盤 | BE-1 | DB-01 | `api/src/middleware/` | §5.2 の全条件 | 3 |
| P1-DOC-01 | utoipa で OpenAPI 生成、CI 差分検査 | BE-1 | API-01 | `contracts/api/openapi.json` | 手書き YAML が無い。生成 spec とコミット済み spec の不一致で CI が落ちる | 1 |
| P1-PARSE-01a | docling-serve 接続と NDJSON 受け口 | BE-2 | ENV-01 | `worker/src/docling.rs` | 100 ページ PDF を投げて節 + 段落の JSON が返る | 2 |
| P1-FE-01a | Vite + 型生成 + ログイン骨格 | FE | DOC-01(spec 暫定) | `apps/web/` | ログイン画面が表示され、OpenAPI 由来の型が import できる | 2 |
| P1-QA-01a | 貫通チェック用の 1 シナリオ台本 | DS | — | `docs/phase1/walking-skeleton.md` | §6 の A-01〜A-10 が 1 行ずつ検証手順を持つ | 2 |

### W2：文書取込とジョブ（BE 9 / FE 4 / DS 0）

| ID | タスク | 担当 | 依存 | 成果物 | 完了条件 | 人日 |
|---|---|---|---|---|---|---|
| P1-API-02 | upload-url / 登録 / 一覧 / 論理削除 | BE-1 | API-01 | `api/src/routes/document.rs` | `detail/04` §1.1〜§1.4 の契約テストが通る | 2 |
| P1-JOB-01 | job / checkpoint / 再試行 / SSE | BE-1 | DB-01, API-02 | `store/src/job.rs` | §5.4。worker kill → 再起動で再開 | 3 |
| P1-DOC-02 | 監査ログ（全書き込み） | BE-1 | DB-01 | `store/src/audit.rs` | 「誰がいつどのファイルを上げたか」を照会できる。UPDATE / DELETE を REVOKE | 1 |
| P1-PARSE-01 | PDF 解析 → DocNode 木 | BE-2 | PARSE-01a | `worker/src/pdf.rs` | §5.5。100 ページ 5 分以内 | 2 |
| P1-FE-01b | プロジェクト一覧 / 文書一覧 / アップロード | FE | API-02 | 3 画面 | presigned PUT → 登録 → 一覧に出る。カーソルページング | 4 |
| P1-QA-01b | W2 貫通チェック（モック可） | BE-1 + BE-2 | API-02, PARSE-01 | チェック結果 | アップロード → job 完了まで通る。詰まりを W3 に持ち越さない | 1 |

### W3：検索と証拠（BE 9 / FE 4 / DS 1）

| ID | タスク | 担当 | 依存 | 成果物 | 完了条件 | 人日 |
|---|---|---|---|---|---|---|
| P1-RAG-01 | 800 token 固定長 chunk / 単一モデル / ANN | BE-2 | PARSE-01 | `engine-rag/` | §5.6。任意クエリで top-k が返る | 3 |
| P1-EVID-01 | 証拠レコード、locator、quote 検証 | BE-2 | RAG-01 | `engine-rag/src/evidence.rs` | §5.7。`quote` が原文部分文字列でないと 422 | 2 |
| P1-API-03 | search / chunks / evidence / jobs / events | BE-1 | RAG-01, JOB-01 | `api/src/routes/knowledge.rs` | `detail/04` §2.1〜§3.2 の契約テストが通る | 3 |
| P1-FE-01c | 文書詳細 + 検索結果 + 証拠ドロワー | FE | API-03 | 2 画面 | 検索結果クリックで PDF 該当ページが開きハイライト | 4 |
| P1-DOC-03 | 用語集・日本語 UI レビュー | DS | FE-01c | `07-UI.md` 差分 | 禁則処理、全角括弧、和暦、日文字体が適用 | 1 |
| P1-QA-01c | 契約テスト（OpenAPI ↔ 実装） | BE-1 | API-03 | CI ジョブ | `oasdiff` で破壊的変更 0 | 1 |

### W4：結合・受入（BE 5 / FE 2 / DS 1 / 残りはバッファ）

| ID | タスク | 担当 | 依存 | 成果物 | 完了条件 | 人日 |
|---|---|---|---|---|---|---|
| P1-QA-01 | A-01〜A-10 の全受入 | 全員 | 全タスク | 受入記録 | 10 件すべて緑。降格が必要なら §7 に従い記録 | 3 |
| P1-PERF-01 | Phase 1 規模のレイテンシ測定 | BE-2 | QA-01 | 測定ログ | 100 ページ PDF で検索 P95 < 300ms（Phase 1 は実質 1 分区）。`detail/03` §7 の形式で残す | 1 |
| P1-DOC-04 | Phase 2 への引継ぎメモ | DS + BE-1 | QA-01 | `docs/phase1/handoff.md` | 「置き換える箇所」と「触らない箇所」を列挙 | 1 |
| P1-FIX-01 | 受入で出た不具合の収束 | 全員 | QA-01 | — | 未解決 P1 不具合 0 | 3 |

> W4 は 3 人日 × 2 名で 6 人日を使い、残りはバッファ。`P1-FIX-01` が 0 で済めば **Phase 2 の設計レビューに前倒し**する。**Phase 2 の実装を前倒しで始めない**（骨格の評価が曖昧になる）。実効 80% 前提では BE が 2 人日超過するため、`P1-FIX-01` を Phase 2 W1 に送る判断を W3 末に行う。

## 5. 主要タスクの実装契約

### 5.1 `P1-DB-01` マイグレーション

`detail/02-database-ddl.md` §7 の `001_core` / `002_core_ops` を適用する。Phase 1 で PDF しか扱わなくても**以下を初日から作る**。

| 項目 | Phase 1 での要求 | 理由 |
|---|---|---|
| `kb.chunk` / `kb.embedding_1024` の **32〜64 分区** | `HASH(id)` / `HASH(chunk_id)` で**全分区を作成** | 後から非分区表を分区化するのは 1,200 万行で実質不可能。`project_id` はキーにしない（A-02 が単一プロジェクト） |
| `kb.embedding_1024.project_id` | 冗長列として保持し、`kb.chunk.project_id` と一致させる | 検索のスコープは RLS + 索引で強制。pruning に依存しない |
| `core.outbox_event` | `event_id` PK、`seq` IDENTITY、`outbox_dead_letter` まで作成 | Phase 3 の投影で列を足さない |
| `graph_meta.entity.projection_seq` | 列だけ作る（値は 0） | Phase 3 で ALTER しない |
| `core.audit_log` | `PRIMARY KEY (id, occurred_at)` + 月次 RANGE 分区、12 か月先まで | 追記専用。UPDATE / DELETE を REVOKE |
| `kb.document.doc_type` | §0.3 の列挙で `CHECK` | `DETAILED_SPEC` を誤登録させない。Phase 1 の PDF は `MANUAL` または `OTHER` |

**RLS**：`project_id` を持つ全表に `USING (project_id = current_setting('app.project_id')::uuid)` を適用。アプリはトランザクション開始時に `SET LOCAL app.project_id = …` を**サーバー側コンテキストから**設定する。リクエスト本文の値は使わない。

### 5.2 `P1-API-01` 認証・認可・エラー

- ブラウザ：`HttpOnly + SameSite=Lax` セッション Cookie。プログラム：`Authorization: Bearer <pat>`（`core.api_token.token_hash` にハッシュ保存）。
- ミドルウェアの順序を固定：`trace_id → 認証 → パス {pid} とセッション範囲の照合 → SET LOCAL app.project_id → permission 検査 → ハンドラ`。
- パス `{pid}` が許可範囲外なら **404**（403 にしない。存在を隠す）。本文に `project_id` が残っていれば破棄し、乖離していれば `VALIDATION_SCHEMA`。
- エラーは `application/problem+json`。`code` / `trace_id` / `errors[]` を必ず入れる（`detail/04` §0.1）。`E-DOC-xxxx` のような独自体系を作らない。
- 全レスポンスに `X-Trace-Id`。OTel の trace id と一致させる。
- 全リソース ID は **UUID**。表示用 `code` は `id` と併記するだけ（`detail/04` §0.2）。

### 5.3 `P1-API-02` 文書 API

`detail/04-api-contract.md` §1.1〜§1.4 をそのまま実装する。Phase 1 固有の差分は **locator の `kind` だけ**：

```json
{"kind":"pdf","page":12,"bbox":[72.0,118.5,523.0,206.25],"char_range":[1024,1180]}
```

正本 `detail/04` §2.1 の例は xlsx だが、Phase 1 は上の PDF locator を**同じ契約の `locator` フィールド**で流す。

- `upload-url` は DB に document を書かない。登録 API で初めて 1 行作る。
- `content_hash` 重複は `CONFLICT_ALREADY_IMPORTED`。
- 一覧は**カーソルのみ**、`?limit=50&cursor=<opaque>`、`include` で列を絞る。OFFSET 禁止。
- 論理削除は即時 `deleted_at`、索引からの除外も即時。物理削除は非同期（Phase 3 の削除伝播と同じ規約）。

### 5.4 `P1-JOB-01` ジョブと SSE

- 状態：`QUEUED / RUNNING / SUCCEEDED / SUCCEEDED_WITH_ERRORS / FAILED / CANCELLED`。
- 取得は `FOR UPDATE SKIP LOCKED LIMIT 200`。`Idempotency-Key` 未指定でも内容ダイジェストで生成し、重複送信は同一 `job_id` を返す。
- `job_checkpoint` に「処理済みページ番号 / 完了 chunk 数」を 1,000 chunk ごとに保存。worker kill → 再起動で未完了だけ再処理。
- `progress` は **ファイル / セル / chunk の 3 指標**を必ず持たせる（Phase 1 はセル = 0 でもキーを残す。`detail/01` §11 と同じ形にしておく）。
- SSE は全イベントに単調増加の `id`、`Last-Event-ID` で再開、20 秒ごとに `: ping`、`done` / `error` で接続を閉じる（`detail/04` §3.2）。
- 長い処理は `202 + {job_id, links}`。HTTP 接続を握らせない。

### 5.5 `P1-PARSE-01` PDF 解析

- **自作 `doc-worker` を書かない**。`quay.io/docling-project/docling-serve-cpu:v1.38.1` をそのまま使う。
- 返ってくる節・段落を `kb.doc_node`（Document / Section / Paragraph）へ。セル単位ノードは作らない。
- 証拠位置は `page + bbox + char_range`。PDF が縦書き / ルビを含む場合は `writing_mode: vertical` を `metadata` に残し、ルビは独立テキスト層として保持する（`00-全体設計.md` J-04）。
- 日本語ファイル名・見出しは **原文 NFC で保存**、検索用にだけ NFKC + 旧字体マップを掛ける（J-03）。`quote_hash = sha256(NFC(quote))`。
- タイムアウト 5 分。超過は `VALIDATION_SCHEMA` でラップし、`detail.code` に内部コードを残す。

### 5.6 `P1-RAG-01` 最小 RAG

| 項目 | Phase 1 | Phase 2 で置き換えるもの |
|---|---|---|
| chunk | 固定長 **800 token**、overlap 80 | 構造認識、親子 chunk、Excel 12 行ブロック |
| 日本語処理 | Lindera(IPADIC) + `to_tsvector('simple', tokenized)` | L2 pg_bigm + クエリルーティング + RRF |
| embedding | 単一モデル、`kb.embedding_1024` | モデル切替 / `index_build` / halfvec |
| 検索 | ベクトルのみ | ハイブリッド + rerank + graph assist |
| 分区 | 32〜64 へ fan-out（実質ヒットは少数） | 本番 64 区並列（`detail/03` §2） |

> **Phase 1 から fan-out の呼び出し形を作る**。1 分区だけを決め打ちで叩く実装にすると、Phase 2 で 1,200 万行対応のときに検索層を作り直すことになる。

- `project_id = $1 AND is_current = true` を**必ず**クエリに含める。RLS は第二の防衛線であり、アプリ側の条件を省略しない。
- `hnsw.ef_search = 80` を起点。`SET LOCAL hnsw.iterative_scan = relaxed_order`。
- `latency_ms` はサーバー側処理時間（キュー待ちを除く）。全検索を `kb.retrieval_log` に記録し、P95 を出せるようにする。

### 5.7 `P1-EVID-01` 証拠

- `kb.evidence` に `locator / quote / quote_hash / resolution / confidence` を保存。`evidence` は 1 Fact につき 1 件以上（Phase 4 の `code.fact_evidence` と同じ規約）。
- `quote` は**原文の完全な部分文字列**。サーバーが検証し、不一致なら 500 ではなく `EVIDENCE_QUOTE_MISMATCH`（422）。
- JSON の `quote_hash` は `sha256:` + lowercase hex 64 文字。DB は `bytea`。
- UI は `locator` だけを見てページを開く。**AI に locator を生成させない**（Phase 1 に AI は無いが、規約を先に固定する）。

### 5.8 `P1-FE-01` フロント

5 画面：ログイン / プロジェクト一覧 / 文書一覧 / 文書詳細 / 検索。Phase 1 では `07-UI.md` の**ナビゲーション入口を段階公開**する（Skills / Tools / Users / Data Sources / Settings は非表示）。

- 型は `openapi-typescript` で生成。手書きの DTO を置かない。
- 一覧は仮想スクロール 50 行。全件 DOM を作らない。
- 日本語 UI：禁則処理、全角括弧、和暦、日文字体。エラーは 2 行目に業務 `code` と `trace_id` を小さく出す。
- PDF ビューアは該当ページのみ描画（100 ページ全部を DOM に載せない）。

---

## 6. 受入テスト（A-01〜A-10）

| # | 手順 | 期待 | 対応する 08 の受入 |
|---|---|---|---|
| A-01 | `docker compose up` | `/healthz` 200、ログイン画面が表示 | compose 起動 |
| A-02 | 100 ページ PDF をアップロード | 5 分以内に `status=SUCCEEDED`、`doc_node` と chunk が生成 | 100 ページ / 5 分 |
| A-03 | 任意の日本語クエリで検索 | top-k が返り、全件に `locator` と `latency_ms` | 位置特定可 |
| A-04 | 検索結果をクリック | PDF の該当ページが開き、`bbox` がハイライト | 該当ページを開く |
| A-05 | 解析中に worker を kill → 再起動 | 未完了ジョブが `job_checkpoint` から再開、重複 chunk 0 | チェックポイント再開 |
| A-06 | 切断した SSE を `Last-Event-ID` で再接続 | 欠番なく `done` まで届く | SSE 進捗 |
| A-07 | 同じ PDF を `Idempotency-Key` 付きで 2 回登録 | 同一 `job_id`、追加 chunk 0、`CONFLICT_ALREADY_IMPORTED` | 冪等 |
| A-08 | 監査ログを照会 | 誰がいつどのファイルを上げたか trace_id まで追える | 監査 |
| A-09 | 別プロジェクトの `{pid}` をパスに指定 | 404（403 ではない）。他プロジェクトの文書が 1 件も混じらない | 分離 |
| A-10 | `oasdiff` を実行 | 破壊的変更 0。フロント型が再生成されても差分なし | 契約 |

**A-02 と A-05 は週次で再実行する**（W2 末・W3 末・W4 末）。骨格が壊れたまま 3 週進むのを防ぐ。

---

## 7. 降格案と判断基準

`08-実装計画.md` §8 の降格方針に従う。**黙った機能削減を禁止**し、必ず UI / 帳票に `degraded` として出す。

| 詰まり | 判断タイミング | 降格案 | 復帰条件 |
|---|---|---|---|
| docling-serve が閉域網で起動できない | W1 末 | 自作せず、PDF を `pdftotext -layout` + ページ座標の簡易パーサに降格。`locator` はページ単位 | Phase 2 でコンテナ配布手段を確立 |
| Embedding Provider が使えない | W2 末 | ローカル推論に切替。不可なら全文検索のみとし `degraded: ["vector_unavailable"]` | Provider 疎通 |
| Lindera 辞書が組み込めない | W2 末 | `to_tsvector('simple', text)` の文字単位に降格し、日本語 recall を計測して明示 | 辞書同梱 |
| PDF ハイライト座標が取れない | W3 末 | ページ単位ハイライトに降格。`bbox=null` を返す | 座標抽出 |
| 4 週で A-01〜A-10 が揃わない | W4 末 | **期間延長を選ぶ**。機能を削って「完了」と宣言しない | — |

---

## 8. CI ゲート

`08-実装計画.md` §6 を Phase 1 用に具体化：

```text
cargo fmt --check
cargo clippy -- -D warnings
cargo test --workspace
sqlx migrate run（空 DB）→ 001/002 が冪等に適用できること
contracts/api/openapi.json と utoipa 生成 spec の一致
oasdiff（破壊的変更 0）
fixture: 100 ページ PDF → 5 分以内（週次ジョブ、PR では 10 ページ版）
RLS テスト: app.project_id を切替えて他プロジェクト 0 件
supersede / outbox: 重複送信で published_at が 1 回だけ更新
docs/design/ja/**/*.md: CP932 で符号化できない文字を含まない（簡体字の混入検出）
```

> **Phase 1 でも 1,200 万行前提の DDL を落とす**。性能ゲート（`detail/03` §7）の本番値は Phase 2 / 4 で測るが、分区・主キー・RLS の形だけはここで固定する。

---

## 9. 看板の運用ルール

| ルール | 内容 |
|---|---|
| 状態 | `TODO / IN_PROGRESS / REVIEW / DONE / BLOCKED`。`BLOCKED` には依存 ID と解消条件を 1 行で書く |
| 完了 | DoD を満たし、対応する契約テストが CI で緑になったときだけ `DONE`。「コードを書いた」は `REVIEW` |
| 設計変更 | タスク内で設計を変えない。ADR を起こし、`00` / 該当 `detail/` を更新してからタスクを再見積もり |
| 引継ぎ | `P1-DOC-04` に「置き換える境目」を明記。特に `RAG` の chunk 生成と `PARSE` の locator は Phase 2 で差し替える |
| 数値 | 規模・SLO・版の照会は必ず正本へ。本稿の数値を顧客への約束に使わない |
