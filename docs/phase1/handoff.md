# Phase 1 → Phase 2 引き継ぎ

> Phase 2 の受入結果・未実施項目は [`docs/phase2/acceptance.md`](../phase2/acceptance.md) を参照。
> Phase 3 は [`docs/phase3/acceptance.md`](../phase3/acceptance.md)、Phase 4 の
> source-only 縦断実装は [`docs/phase4/acceptance.md`](../phase4/acceptance.md)、Phase 5 の
> ENTITY / DIFF Impact Draft は [`docs/phase5/acceptance.md`](../phase5/acceptance.md)、
> Phase 6 のテスト工学は [`docs/phase6/acceptance.md`](../phase6/acceptance.md) を参照。

## Phase 4 で追加した境界

| 領域 | Phase 4 の実装結果 |
|---|---|
| code schema | `013_code.sql` で repository / revision / source_file / symbol / analysis_run / classpath_index / fact / fact_evidence |
| 入力 | `DIR` / `SNAPSHOT` を `SYSTIVRA_DATA_DIR` 配下から同期。Git / SVN は未実装 |
| 文字コード | engine-code の text で元バイト偏移を保持。worker は `engine_code::text` を再 export |
| 解析 | Tree-sitter Java / JSP、基本的な XML / SQL。OpenRewrite と顧客 classpath は未接続 |
| Fact | `supersede_then_insert`、evidence 1 件以上、未変更 Fact の増分再利用 |
| API / UI | `/projects/{pid}/code/*`、`/code/analysis-runs/{id}`、`/code/files/{id}/content`、コード解析画面 |

Phase 4 の未完了事項を「完了」と表現しない。特に `classpath_complete=false`、
source-only の `RESOLVED`、Neo4j 無効 E2E、Fact の行 INSERT は既知の制約である。

## Phase 5 で追加した境界

| 領域 | Phase 5 の実装結果 |
|---|---|
| impact schema | `014_impact.sql` の `impact.assessment_draft`（report は JSONB） |
| 入力 | ENTITY / DIFF のみ。自然言語 seed は未実装 |
| 走査 | PG graph を in-memory に読み、双方向 BFS、hub / depth / budget 打切り |
| 証拠 | code Fact evidence + document chunk evidence。証拠なしは `unverified` |
| 安全性 | `approval_state=DRAFT`、コード変更・テスト実行なし |
| API / UI | `/projects/{pid}/impacts`、`/impacts/{iid}`、ImpactPage |

OpenRewrite / `javac` は導入しない。文書リンクは simple symbol 名の chunk 一致による
`INFERRED` であり、コード境界や実行時データフローの証明ではない。

## Phase 2 で置き換える場所

| 領域 | Phase 1 | Phase 2 の実装結果 |
|---|---|---|
| PDF 解析 | `crates/worker/src/pdf.rs`（`pdf-extract`） | 変更なし。docling-serve は未使用 |
| chunk | `chunk_page`（800 token / 80 overlap、ページ単位） | XLSX 12 行 / 800 token、DOCX section、PPTX slide の構造チャンクを追加。親子 chunk は未実装 |
| embedding | `engine-rag::embed`（決定的ハッシュ、1024 次元） | `embed_for_revision` と `index_build` で 2 版を切替。外部モデルは未接続 |
| 検索融合 | `engine_rag::fuse`（固定重み） | Lindera(IPADIC) + bigram、RRF（k=60）、ローカル語彙一致 rerank。pg_bigm は未導入 |
| 座標 | `locator.bbox=null`、UI が PDF.js テキスト層で矩形計算 | PDF は同じ。XLSX/DOCX/PPTX は OOXML の locator（sheet/range、section_path、slide）を追加 |
| 原本保存 | ローカル FS、HMAC PUT URL | 変更なし。S3 は未対応 |
| マイグレーション | ファイル適用（`psql` / compose initdb） | 010 を追加。sqlx `Migrator` には未移行 |

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
7. `documents:bulk-import`、構造木、索引再構築、Chunk Explorer は Phase 2 で実装済み。
   改版履歴 UI、版指定検索、ジョブキャンセルは未実装。
8. 009 は `009_rename_corpus.sql`、010 は `010_index_build_active.sql`、011 / 012 は Phase 3 の
   グラフ（`011_graph.sql` / `012_graph_relationship_seq.sql`）、013 は Phase 4 の
   code schema（`013_code.sql`）、014 は Phase 5 の `014_impact.sql`、
   015 は Phase 6 の `015_test.sql`、016 は Phase 7 の `016_ai.sql`、
   017 は Q-01、018 は OIDC SSO、019 はサーバー側セッション、021 は IdP 紐付け、
   022 は SCIM、023 は `core.first_project()` が使用済み。020 は ops 用に予約。
   **次の migration は 024 以降**。
9. `apps/web/src/pages/ImpactPage.tsx` は ENTITY / DIFF の Impact Draft API に接続済み。
   自然言語問合せ、5 セクション SSE、テスト候補は未実装で、Q-01 全体契約へは未到達。

## Phase 6 で追加した境界

| 領域 | Phase 6 の実装結果 |
|---|---|
| test schema | `015_test.sql` の 11 表。全表 RLS、`core.project_of` に 5 分支追加 |
| ルール / 派生 | `engine-test` の純ロジック。競合は両方残し、読めない記述は数える |
| DSL | 保存時検証と承認時検証。違反は 422 / `SCHEMA_VIOLATION` |
| run | `TEST_RUN` job は `READY` まで準備し、別プロセスの `workers/runner/` へ引き渡す |
| Runner | JSON DSL を 4 並列で実行。失敗時のみ 4 点成果物と HAR、HAR は伏せ字化 |
| 実行結果 | `executions:import` に `execution_source=PLAYWRIGHT` を付けて戻す。PASSED を捏造しない |
| 被覆 | 5 指標。Requirement は PG の `VERIFIES` 辺から。ルール由来は `INFERRED` |
| API / UI | `/projects/{pid}/test/*` 8 端点、`TestPage.tsx` |

`015_test.sql` は `core.project_of` を `CREATE OR REPLACE` で全体書換えする。
014 の 10 分支を消さないこと。`scripts/verify-db.sh` に回帰検査がある。
詳細な未実施項目は [`docs/phase6/acceptance.md`](../phase6/acceptance.md)。

## Phase 7 で追加した境界

| 領域 | Phase 7 の実装結果 |
|---|---|
| ai schema | `016_ai.sql` の 10 表。全表 RLS、`core.project_of` に 4 分支追加 |
| AI Provider | `OpenAiCompatProvider`（http）と `DisabledAiProvider`。未設定なら生成しない |
| Agent | 登録・版・Prompt、`AGENT_RUN`、1 回の LLM 呼び出しと構造検証 |
| MCP | 12 ツール公開、JSON-RPC 3 メソッド、`schema_hash` 記録。外部接続は未実装 |
| Workflow | JSON 定義、承認ノードで `WAITING_APPROVAL`、承認後に再開 |
| Workflow エディタ | SVG で配置・接続・属性編集・保存。座標は `x/y` に保存 |
| 承認 / 監査 | `approval_request` と `ai.*` の監査。判断は `AgentRun` の write 権限 |

## Q-01 で追加した境界（Phase 5 追補）

| 領域 | 実装結果 |
|---|---|
| 入力 | ENTITY / NATURAL_LANGUAGE（決定的な部分一致）。DIFF は 400 |
| シード | 一意なら進行、曖昧なら `NEEDS_CLARIFICATION` + 候補、0 件は `NOT_FOUND_SEED` |
| 5 セクション | ImpactReport からの投影。`summary` は件数テンプレート |
| 配信 | `/events` は NDJSON 一括。`summary` → 4 段 → `done` |
| テスト | 既存は `covers_impacts`、草稿は最大 20 件（保存しない） |
| 計画 | `test-plans:derive` で DRAFT 計画を作る。`test_run` は作らない |
| 出力 | `/change-inquiries/{iid}/report.md` と `/impacts/{iid}/report.md`。PDF は未対応 |
| AI 叙述 | `/change-inquiries/{iid}/narrate` と `/impacts/{iid}/narrate`。未設定なら `ai_unavailable`、設定時は `ai_inferred` |
| run の範囲 | `plan_id` 指定時は当該計画のケースのみ。`null` は全承認済みケース |

未実施は [`docs/phase7/acceptance.md`](../phase7/acceptance.md)。

## Phase 8 で追加した境界（運用引渡しの縦断切片）

| 領域 | 実装結果 |
|---|---|
| 認証 DDL | `018_sso.sql` の `core.external_identity`。RLS 対象外（認証前参照） |
| OIDC | discovery / JWKS / 認可コード + PKCE(S256) / state / nonce。未知 subject は自動登録しない |
| セッション | `019_session.sql` の `core.session`。Cookie は session id を含み、logout / back-channel で失効。復元時は全失効 |
| セッション管理 | 一覧・個別失効・全失効、アイドル期限、期限切れ削除、`/security` 画面 |
| 認証強化 | 認証 3 経路の IP レート制限（429 + Retry-After）、セキュリティヘッダ、password 監査 |
| SCIM / ユーザー | 022/023。Users / Groups、グループ→ロール、`/users` 画面 |
| 運用 | PITR 訓練、暗号化・保持・オフサイトフック、clean-room rebuild、SIEM 転送 |
| API | `/auth/config`、`/auth/oidc/start`、`/auth/oidc/callback`。未設定は 503 |
| UI | ログイン画面の SSO 入口と理由別エラー。パスワードログインは併存 |
| バックアップ | `scripts/backup.sh`。PG + 原本 + manifest。Neo4j は導出物として含めない |
| 復旧訓練 | `scripts/restore-drill.sh`。件数・ハッシュ・RLS・FTS・graph rebuild・全セッション失効を検証 |

未実施は [`docs/phase8/acceptance.md`](../phase8/acceptance.md)。
