# Changelog

## 2026-11-08 — Phase 6 正式受入: 24 画面 / 300 ルール・失敗モード・保持・VERIFIES 投影

### Added

- **SAMPLE 24 画面 / 300 ルール**の合成 fixture（`samples/sample-web/sample/s/01..24/edit/`）と
  正解集（`samples/phase6/golden.json`）。24 計画で **1,200 組**のデータを派生し、
  承認 120 ケースを実行して初回合格率 **100%** / 正解集一致 **100%** を実測。
  100 ケース / 4 並列は **12.32 秒**。`scripts/phase6-sample-eval.py`。
- `derive` の `base_path` を API から指定できるようにし、画面ごとの計画へ分けた。
- Runner（`workers/runner/runner.mjs`）に `detail/09` §13 の失敗モードを実装:
  先頭 3 件全滅で `ENV` へ切替えて run 停止（残り `SKIPPED`）、flaky 集計、
  `DATA` で reset 強制、成果物 0 件は `UPLOAD_FAILED` + run 警告、環境ロック待ち、
  ケース 180 秒 / run 24 時間上限。`scripts/phase6-failure-eval.py` が 9 シナリオを実測。
- `storage_state` の鮮度確認（`STORAGE_STATE` / `STORAGE_STATE_MAX_AGE_MIN`）と
  run 開始時のデータ seed / reset（`DATA_MODE` / `SEED_URL` / `RESET_URL`）。
  期限切れ / 欠落は run を作らず `ENV` で中断する。
- 成果物の容量上限と保持: trace 20 MB（超過は `TRUNCATED`）、run 合計 20 GB
  （超過後は trace 無効）、90 日 / trace 30 日の保持で実体を削除し `EXPIRED` を記録。
  `POST /projects/{pid}/test/artifacts:expire` と
  `POST /projects/{pid}/test/runs/{rid}/artifacts:cleanup` を追加し、worker が日次で適用する。
  UI に成果物チップ（`TRUNCATED` / `EXPIRED` / `UPLOAD_FAILED`）と削除ボタンを追加。
  `scripts/phase6-retention-eval.py` が上限・冪等・worker 日次を実測。
- `TestCase -[:VERIFIES]-> Requirement` を **outbox 経由で Neo4j へ投影**。
  `derive` が同じ transaction で `EntityUpserted` / `RelationshipUpserted` を積み、
  worker の projector が MERGE する。Neo4j の `VERIFIES` は `resolution=INFERRED`、
  Requirement は `natural_key=rule:<uuid>`。`scripts/phase6-graph-eval.py` で
  PG と Neo4j の件数一致（3 / 13 / 12）を実測。`natural_key` を Neo4j に保存する。
- Runner 単体テスト（`workers/runner/runner.test.mjs`、8 件）を追加し、
  `.github/workflows/ci.yml` の `runner` ジョブで実行する。

### Verification

- `cargo test --workspace`: **233 passed** / fmt / clippy / OpenAPI 一致。
- `scripts/verify-db.sh`: 001〜027 全項目通過。
- i18n 569 keys / frontend build / Phase 6 UI（1440 / 607）が緑。
- 受入の詳細と未検証（Collector 分離 / 16 ワーカー / 実案件 90 秒）は
  `docs/phase6/acceptance.md` に記載。

## 2026-11-08 — 日本語検索: 保存済み chunk の再 tokenize

### Fixed

- ドキュメントは「形態素分割（日本語）は未実装で `住所` では当たらない」と
  書いていたが、**Lindera + 内嵌 IPADIC は実装済み**（単体テストあり）。
  真の原因は `corpus.chunk.tokenized` が保存済みの古い値で固定され、
  `fts`（`to_tsvector('simple', tokenized)` の生成列）が追随しないこと。
  本文に `住所` が在っても索引に語彙が無く 0 件になっていた。

### Added

- `POST /api/v1/projects/{pid}/knowledge/retokenize`: 現行 tokenizer で
  `tokenized` を最大 5000 件ずつ作り直す。`updated` / `examined` / `remaining`
  を返し、`remaining=0` まで繰り返せる。`DocumentIngest` の write 権限が必要、
  監査 `knowledge.retokenize` を残す。`fts` は生成列なので自動で追随する。
- `RetokenizeResponse` 契約、OpenAPI / TS 型を再生成。
- バッチは**古い行だけ**を選ぶ（本文に和文があるのに `tokenized` に和文が
  無い行）。全件を毎回読むと更新済みの先頭 5000 件がまた返って
  `remaining` が減らず、呼び出し側が永久ループしていた。
- `scripts/knowledge-retokenize-eval.py`: 古い `tokenized` を再現 → `住所` 0 件
  → retokenize → 1 件、`fts` 命中 0 → 1、再実行は
  `updated=0` / `examined=0` / `remaining=0`（収束と冪等を両方検査）。

### Verification

- `{"before_hits":0,"rebuilt":{"updated":1,"examined":1,"remaining":0},
  "after_hits":1,"fts_before":0,"fts_after":1,"idempotent_updated":0}`
- fmt / clippy / test / OpenAPI / i18n / frontend build が緑。

## 2026-11-08 — 処理中の心拍を 2 worker で実測

### Added

- これまで「stale 窓を超える job kind が検証環境に無い」として
  **処理中の心拍だけ未実測**だった。`scripts/mock-llm.py` に検証用の遅延
  （`MOCK_SLOW_HEARTBEAT` / `MOCK_LLM_SLOW_SEC`）を足し、
  `phase7-agent-loop-eval.py` で `AGENT_RUN` を意図的に長くした。
- 同 eval が `WORKER_STALE_SEC=3`（心拍 1 秒間隔）の worker を**2 台**起動し、
  片方が処理中にもう片方が回収を試みる状況を作る。

### Verification

- `heartbeat_during_processing = {beats: 10, status: SUCCEEDED_WITH_ERRORS,
  attempt: 1, workers: 2}`。
  心拍が 10 回進み、2 台目に奪われず `attempt=1` のまま完了。
  心拍が無ければ stale 3 秒で再キューされ `attempt=2` になっていた。
- 回収側が効くことは `job-recovery-eval.py` で別途実測済み。
  2 つを合わせ「新しければ奪われない／途切れれば回収される」を実測で示した。
- `docs/phase8/operations.md` の「未実測」を削除。実案件規模の多重度は依然未検証。

## 2026-11-08 — 承認の起票を本人で隔離する

### Fixed

- `pending_approval` が `action` + `payload` だけで既存 PENDING を探していた。
  別のユーザーが同じツール・同じ引数で起票していると**その人の承認 ID を返し**、
  相手が承認しても `approval_pass` は `requested_by` 不一致で使えず、
  こちらは 423 のまま永久に進まなかった。
- 承認 ID の安定ハッシュにも起票者を混ぜた。これが無いと別ユーザーが同じ引数を
  投げたとき `ON CONFLICT (id) DO NOTHING` で相手の ID を返してしまう。

### Verification

- `phase7-eval`: 同一 action / 引数の他人の PENDING を DB に置き、
  `tools/call` が**別の** `approval_id` を返すことを実測。
  承認→1 回実行→再呼び出し 423 の既存フローも引き続き緑。
- fmt / clippy / test / OpenAPI が緑。

## 2026-11-08 — job 回収をイベントに残す

### Fixed

- `requeue_stalled` が状態だけを書き換えていた。`/jobs/{id}/events` には
  「RUNNING で始まった」という行しか残らず、後から見ると「なぜ急に FAILED に
  なったのか」「なぜ再キューされたのか」が分からなかった。
- `QUEUED` 復帰には `progress` / `phase=RECOVERED` / `reason=stalled_heartbeat`、
  試行切れの `FAILED` には `error` / `code=WORKER_HEARTBEAT_LOST` と
  `attempt` / `max_attempts` を、更新と同じ CTE で挿入する（状態と記録が分離しない）。

### Verification

- `job-recovery-eval`: `progress|stalled_heartbeat` と
  `error|WORKER_HEARTBEAT_LOST` の両イベントを実測。
- fmt / clippy / test / OpenAPI が緑。

## 2026-11-08 — ドキュメント整合性監査（コード変更なし）

### Fixed

- `README.md` / `08-実装計画.md` / `phase7/acceptance.md` が今も
  「12 ツール中 11 本が実行可能」と書いていた。`test.run.execute` は承認→1 回実行
  まで実装済みなので、「11 本は即実行 / 1 本は承認後」に改めた。
- `phase7/acceptance.md` の `Markdown 出力 = 未実装` は**同じ行の注記と矛盾**していた
  （「Phase 5 追補で実装」とある）。Phase 5 で実装・実測済みとして整理。
- 「外部書き込みの承認フローは未接続」が、ローカル 12 Tool の承認フロー（接続済み）
  と混同されうる記述だった。**外部 MCP Client 経由の Tool** に限る旨を明記。
- 古い現行手順の値を更新: `verify-db.sh` 001〜024/025 → **001〜027**、
  i18n 556 keys → **564 keys**、`phase6` の再現コマンド 001〜015 → 001〜027。
- 各 phase の受入表に残る `001〜0xx` / キー数は**その時点の履歴値**であること、
  現行値は phase7 §1・§2 を正本とすることを README に明記。
  過去の実測値を書き換えて「当時は無かった検証をした」ように見せない。

## 2026-11-08 — 承認待ちに期限を付ける（EXPIRED を実装）

### Added

- `status` の enum に `EXPIRED` は在ったが、PENDING をそれに遷移させるコードが
  **無かった**。1 か月前の依頼でも今日「承認」でき、判断者は当時の文脈を持たない。
- worker がティックごとに `APPROVAL_TTL_SEC`（既定 86400 = 24 時間）を過ぎた
  PENDING を `EXPIRED` に閉じる。判断者は立てない（`decided_by` は触らない）。
- `EXPIRED` への `decide` は store の `WHERE status='PENDING'` により 409。

### Verification

- `job-recovery-eval`: 25 時間前の PENDING → `EXPIRED`、1 分前の PENDING は据え置き。
- `phase7-eval`: `EXPIRED` な承認への APPROVE → 409。
- fmt / clippy / test / OpenAPI が緑。`.env.example` に `APPROVAL_TTL_SEC` を追記。

## 2026-11-08 — 影響分析: 冪等キーと草稿のずれを修正

### Fixed

- `enqueue_impact_assessment` の冪等キーは依頼内容から決まるが、**草稿行は
  作り直されうる**（バックアップ復元、検証用のリセットなど）。その状態で同じ依頼を
  投げると、返ってくる job は*前の*草稿を処理した完了済みの job になり、新しい草稿は
  `PENDING` のまま取り残されていた。API は「QUEUED」と答え、job も SUCCEEDED に
  見えるので、呼び出し側からは成功に見える。
- 完了済み job が返り、かつ草稿が `SUCCEEDED` でない場合は、時刻を付けた別キーで
  投入し直す。正常系（草稿が完了済み）の冪等性は変えない。

### Verification

- 回帰で発見: Phase 6 のリセット後に Phase 7 を流すと `impact_get_report` が
  `report=null`。修正後は draft `PARTIAL` + report 有り、job は
  `impact:<digest>` と `impact:<digest>:<millis>` の 2 本（前者が前回、後者が今回）。
- Phase 6 E2E / Phase 6 Runner E2E / Phase 7 E2E が緑。fmt / clippy / test も緑。

## 2026-11-07 — Phase 6/7 追補: 環境ガードを fail-closed に

### Fixed

- `guard_execution`（R-07）が **fail-open** だった。「`PRODUCTION` と等しいか」で
  判定していたため、`prod` / `PRODUCTION `（空白付き）/ 未知の環境名が
  「本番ではない」と解釈され、破壊的ケースが素通りしていた。
  **許可リスト**（`LOCAL` / `STAGING` 以外は実行しない）に変更。
- `validate_import` の `destructive_in_production` も同じ形に揃えた。片方だけ
  「PRODUCTION か」で見ると、未知の環境名で 2 つの判定が食い違う。
- REST の `test/runs` は `PRODUCTION` を**受け付けない**。設計は「正式環境での
  実行は承認必要」だが、REST 側に承認を通す経路が未接続。素通しにすると
  実行権限だけを持つ人が承認なしで本番に流せる。

### Verification

- 単体テスト追加: `prod` / `PRODUCTION ` / `PREPROD` / 空文字では破壊的ケースを
  実行しない。`LOCAL` / `local` / `STAGING` / `staging` では実行する。
- Phase 7 E2E: `guard_kind=PROD` → 400、`guard_kind=PRODUCTION` → 400。
- fmt / clippy / test（engine-test 28 件）/ OpenAPI が緑。

## 2026-11-07 — Phase 7 追補: 予算 4 項目をすべて判定する

### Fixed

- `BudgetUsage.wall_time_sec` / `cost_usd` が常に 0 で、`max_wall_time_sec` と
  `max_cost_usd` が**飾り**だった（判定関数は見ていたが、計測していなかった）。
  Agent ループで経過時間を毎ターン測り、トークンからコストを積む。

### Added

- `providers::AiPricing`。`AI_PRICE_INPUT_PER_1M` / `AI_PRICE_OUTPUT_PER_1M`
  （USD / 100 万トークン）から単価を読む。**単価をコードに埋めない**。
- `agent_run.cost` に `wall_time_sec` / `cost_usd` / `priced` を追加。
  単価未設定なら `cost_usd=null` / `priced=false`（0 ドルと区別する）。

### Verification

- mock LLM（11 + 7 トークン）で `max_cost_usd=0.000001` →
  `BUDGET_EXCEEDED` / `budget=max_cost_usd`、`cost_usd=1.8e-05`。
  通常予算の run は `priced=true` / `cost_usd=3.6e-05`（2 コール）。
- `AiPricing` の単体テスト 2 本（未設定は判定不能、負トークンは 0 扱い）。
- fmt / clippy / test / OpenAPI が緑。`.env.example` に単価 2 項目を追記。

## 2026-11-07 — Phase 8 追補: worker 異常終了後のジョブ回収

### Fixed

- 処理中に心拍を打っていなかった。`WORKER_STALE_SEC`（既定 15 秒）を超える job は
  別 worker から「落ちた」と見なされ、**同じ job が二重に走る**状態だった。
  worker は job を処理している間 `WORKER_STALE_SEC / 3` 間隔で `heartbeat_at` を更新する。
- `requeue_stalled` が試行回数を見ていなかった。プロセスごと落とす job は
  `QUEUED` に戻り続け、永久に再実行されていた。`attempt >= max_attempts` の job は
  `FAILED` / `error.code = WORKER_HEARTBEAT_LOST` で**止める**。
- worker の回収ログを出す（`stalled jobs recovered`）。従来は黙って回収していた。

### Added

- `scripts/job-recovery-eval.py`。DB に直接 job 行を作り、回収・再クレーム・
  停止・非奪取の 4 点を実測する。

### Verification

- `{"recoverable":{"status":"FAILED","attempt":2},`
  `"poison":{"status":"FAILED","attempt":3,"code":"WORKER_HEARTBEAT_LOST"},`
  `"alive":{"status":"RUNNING","attempt":1}}`
- fmt / clippy / test が緑。`verify-db.sh` はスキーマ変更なし。

## 2026-11-07 — Phase 7 追補: 承認の起票元と使用済みを画面に出す

### Added

- `ApprovalView` に `workflow_run_id` / `run_id` / `consumed_at` を追加。
  これまで Workflow 由来と MCP 由来の承認が同じ形で並び、API からも
  「どの run の承認か」「もう使えないか」が判別できなかった。
- 承認一覧に**起票元**列（Workflow / MCP / Agent）を追加。`consumed_at` がある
  ものは「使用済み」を併記する（`status=APPROVED` のままなので、無いと
  まだ使えるように見える）。
- i18n 5 キー（ja / en / zh-CN）。564 キー。

### Changed

- `scripts/phase7-eval.py` の承認絞り込みを `requested_by` の文字列から
  `workflow_run_id` に変更（契約で判別できるようになったため）。
  MCP の許可証が一覧で `consumed_at` 付き・`workflow_run_id=null` であることも検査。
- `ui-check-phase7` に「承認行が 5 セル」「使用済みが見える」を追加。

### Verification

- Phase 7 E2E: 承認 → 実行 → 再呼び出し 423、一覧で `consumed_at` あり。
- UI: `{"agents":1,"mcp":1,"workflows":2,"workflowRuns":2,"approvals":5,"columns":1,"errors":[]}`。
- fmt / clippy / test / OpenAPI（再生成）/ i18n 564 / frontend build が緑。

## 2026-11-07 — Phase 7 追補: MCP の承認を「1 回の実行への許可証」に

### Added

- `027_approval_consume.sql`: `ai.approval_request.consumed_at` と部分索引
  `t7_approval_pass_idx`。承認を 1 回の実行で消費する。
- `App::request_tool_approval` / `App::take_tool_approval`。承認必須ツールの
  呼び出しは、未消費の承認があれば**消費して通し**、無ければ起票して 423。
- `Store::approval_pass` / `consume_approval` / `pending_approval`。
  照合は `action` + `payload` の **jsonb 等価**（引数まで含む）。
  起票者本人・未消費・10 分以内だけを許可証として認める。

### Changed

- MCP `tools/call` の 423 応答に `approval_id` を含める。従来は「承認が必要」で
  終わり、承認しても実行する経路が無かった（**行き止まり**）。いまは
  起票 → 承認 → 同じ引数で再呼び出し → 1 回だけ実行、まで通る。
- 同じ引数の連打では承認依頼を増やさない（安定 ID + PENDING 再利用）。

### Verification

- 未承認 → 423（`approval_id=c8a2137d-…`）、同引数の再呼び出し → 同じ ID、
  引数を 1 つ変えた呼び出し → 別 ID。
- 承認 → 同引数で実行し test run `95f9c211-…` を 1 件作成。再呼び出し → 423。
  DB 上も `mcp.call:test_run_execute` は APPROVED かつ `consumed_at` 済みが 1 行。
- fmt / clippy / test / OpenAPI / `verify-db.sh` 001〜027 / Phase 7 E2E が緑。

## 2026-11-07 — Phase 7 追補: `test.case_generate` を「既存計画への追記」に

### Changed

- `DeriveTestPlanRequest.plan_id` を追加。指定すると**計画を新規作成せず**、
  その計画へ草稿ケースを追記する。追記は計画の `scope`（rule / impact item /
  not_covered）を union で更新し、`impact_id` は最初の派生元を残す。
  承認済み（`APPROVED`）の計画へは追記しない（409）。
- `DeriveTestPlanResponse` に `appended` と `plan_case_count` を追加。
  `draft_case_count` は**その呼び出しで生成した数**で、計画の総数ではない。
- `test_plan_generate` は新規作成のみ（`plan_id` を渡すと 400）、
  `test_case_generate` は追記のみ（`plan_id` 必須、無ければ 400）。同じ派生器に
  流していたため、区別が付かないまま「毎回新しい計画ができる」状態だった。
- `input_schema` を実態に合わせた（`test_case_generate` は `plan_id` 必須）。

### Verification

- 追記: 4 件の計画へ 20 件を派生 → `appended=true`、`plan_id` 不変、
  `plan_case_count=20`（4 件は id が一致し `ON CONFLICT DO NOTHING`、
  新規は 16 件。`created_at` が 2 時刻に分かれることで確認）。
- 取り違え: `test_case_generate` に `plan_id` 無し → 400、
  `test_plan_generate` に `plan_id` 有り → 400。
- Q-01 経由（`plan_id=None`）の派生は従来どおり。`phase5-inquiry-eval` が緑。
- fmt / clippy / test / OpenAPI（再生成）/ `verify-db.sh` 001〜026 / i18n /
  frontend build / Phase 7 E2E が緑。

## 2026-11-07 — Phase 7 追補: 承認後の破壊的実行（`test.run.execute`）

### Added

- 能力 `test.run.execute` を実装した。`CreateTestRunRequest`（`name` / `plan_id` /
  `environment`）を Workflow run の `input` から受け取り、`QUEUED` の test run を
  作る。worker が `case_count` を確定して `READY` にし、**Runner は起動しない**。
- `026_workflow_actor.sql`: `ai.workflow_run.requested_by`。capability ノードの
  実行主体は「その run を起動した人間」で、`Authenticated::for_job` で復元する。

### Changed

- Workflow の非 read capability ノードを、**承認ノードを経路上に持つ定義**に限り
  許可するようにした（`approval_preceded`）。判定は `start` からその node への
  経路上に承認があること。状態を `(node, 承認済み)` に広げて到達可能性を見るので、
  「定義のどこかに承認ノードがある」だけでは通らない。
  従来は非 read を一律 400 で拒否しており、破壊的操作の実行経路が存在しなかった。
- capability ノードに run の `input` を引数として渡す（従来は空オブジェクト）。
  これで `knowledge.search` の `query` や `test.run.execute` の `environment` を
  定義側でなく実行時に与えられる。

### Verification

- 承認あり: `start → gate → exec(test.run.execute) → end` を登録し、
  `WAITING_APPROVAL`（この時点で `exec` の node_run は無い）→ 承認 →
  `SUCCEEDED`、`exec` が `PASSED` で test run `12e817f2-…` を作成、
  `case_count=1`、DB 上も `READY`。
- 承認なし: 同じ `test.run.execute` 定義は 400。
- fmt / clippy / test / OpenAPI / `verify-db.sh` 001〜026 / Phase 7 E2E が緑。

## 2026-11-07 — Phase 7 追補: MCP 12 ツールの実行可否を §4.7 に合わせる

### Fixed

- 草稿生成 `test_plan_generate` / `test_case_generate` の `approval_required` を
  `false` にした。`04-コアエンジン.md` §4.7「人間との境界」は
  **テストケース生成=承認不要（草稿）**、**正式環境でのテスト実行=承認必要**と
  定めており、旧定義は境界が逆転していた（草稿すら 423 で作れなかった）。
  承認必須は `test_run_execute` だけになる。

### Added

- `tools/call` が要求者の `Authenticated` を能力実行まで渡すようにした
  （`execute_capability(Some(auth), ..)`）。`knowledge.search` /
  `impact.analyze` / 草稿生成は身元が無ければ実行しない。
- Agent のツールループは `agent_run.requested_by` から身元を復元する
  （`Authenticated::for_job`）。**Agent ではなく依頼した人間**の権限で判定する。
- `knowledge_search` / `impact_analyze` / `test_plan_generate` /
  `test_case_generate` の `input_schema` を具体化。

### Changed

- `ai/overview` の `degraded` から `workflow_editor_not_implemented` を削除した。
  SVG エディタを実装した後も「未実装」を返し続けており、画面に実装済みの機能が
  降格として出ていた。降格は**実際にできないこと**だけを並べる。
- `roles.sql`: `systivra_worker` / `systivra_scheduler` に
  `core.project_member` の **SELECT のみ**を付与。worker が job 起動者のロールを
  実行時に引き直すため（`requested_role` の snapshot では降格を反映できない）。

### Verification

- `tools/call` 実測: read 9 本すべて実行（`knowledge_search` は実 `search_id`、
  `graph_trace_path` は found、`coverage_get` は snapshot 無しで
  `coverage_not_computed` を明示）、草稿 2 本は同一 `plan_id` を返す冪等動作、
  `test_run_execute` のみ 423、未知ツール 400。
- Agent ループ実測: `knowledge.search` だけを持つ Agent で step `PASSED`、
  返送 tool 結果に `SRH-…` を確認。ホワイトリスト外は従来どおり
  `AGENT_CAPABILITY_DENIED`。
- fmt / clippy / test / OpenAPI / `verify-db.sh` 001〜025 / Phase 7 E2E /
  agent-loop E2E が緑。

## 2026-11-07 — Phase 7 追補: MCP 読み取りツールを 7 件へ

### Added

- `tools/call` で `graph_query` / `graph_trace_path` / `code_get_symbol` /
  `knowledge_get_evidence` を実行可能にした。`graph_query` は実体 ID で隣接、
  文字列で実体検索。`code_get_symbol` は FQN / 単純名の部分一致。
- 12 ツールの `input_schema` を具体化（`graph_query` / `graph_trace_path` /
  `code_get_symbol` / `knowledge_get_evidence`）。

### Verification

- `graph_query` 2 件、`code_get_symbol` 6 件、`knowledge_get_evidence` found。
  `test.run_execute` は 423、未知ツールは 400 のまま。
- fmt / clippy / test / OpenAPI / `verify-db.sh` 001〜025 / Phase 7 E2E が緑。

## 2026-11-07 — Phase 5/7 追補: Impact の AI 叙述

### Added

- `POST /impacts/{iid}/narrate`。Q-01 と同じ Provider / 未設定時の扱い。
  AI 未設定なら `narrative=null` + `ai_unavailable`、設定時は `ai_inferred` を付けて
  `impact.assessment_draft.report.narrative` に保存する。
- `ImpactAssessmentDraft.narrative` / `narrative_source`。Impact 画面に
  「AI に要約させる」ボタンと折り畳み表示。

### Verification

- Markdown にも「AI の要約（INFERRED）」を出力（叙述がある場合のみ）。
- 未設定: `narrative=null` / `ai_unavailable`、影響項目 22 件は不変。
  Markdown に叙述セクションが出ないことも確認。
- mock LLM: `model=mock-1` / `ai_inferred`、GET で `narrative_source=AI`。
- fmt / clippy / test / OpenAPI / `verify-db.sh` 001〜025 / i18n 559 キー /
  frontend build / Phase 5 inquiry E2E / Phase 7 agent-loop E2E が緑。

## 2026-11-07 — Phase 4 正式受入: Git・大規模性能・Neo4j 再投影

### Added

- `GIT` repository sync: データディレクトリ内のローカル bare path / `file://`、
  opt-in の HTTP(S) / SSH / `git://`、clone / atomic fetch、branch / tag / SHA 解決、
  commit SHA revision、scanned-files tree digest。既存 scan / hash diff / analysis に接続。
- Git 同期失敗を `CODE_SYNC_FAILED` の `FAILED` job で終端し、エラーを保存して
  revision を追加しない。`SVN` は明示的な unsupported。
- `CODE_ANALYZE` の Fact / evidence を 5,000 行 COPY 化。未変更 Symbol / Fact は
  `UNNEST` バッチで current revision にコピーし、entity / relationship / outbox も再生成。
- Neo4j outbox drain と `/graph/rebuild` を keyset scan + `UNWIND` 1,000 件バッチ化。
- `scripts/phase4-git-eval.py`、`scripts/phase4-scale-eval.py`、
  `scripts/phase4-fact-scale-eval.py`、`scripts/run-phase4-fact-scale.sh`。
- `024_code_schema_repair.sql`: 既存 DB を現在の code schema へ前進修復する冪等 migration。
- UI で選択中 repository kind を chip 表示し、Git 選択時は default branch を revision 初期値に使う。

### Verification

- Git local bare E2E **18 / 18**: path / `file://`、clone / fetch、branch / tag → SHA、
  tree digest、direct SHA 冪等、parent、1 ファイル増分、存在しない branch の `FAILED`。
- 13,000 Java 相当: sync **4.06s**、初回解析 **21.79s**（596.65 files/s）、
  **91,000 Symbol / 104,000 Fact**、増分 **1 / 13,000（0.0077%）**、増分 **203.73s**。
- 2,500,000 Fact COPY: **19.253s / 129,847.44 rows/s**、5,000 行 × 500 文。
  25,000 supersede **5.384s**、live 2,500,000 / historical 25,000 / total 2,525,000。
- Neo4j: PG graph **5,500 entity / 2,500,000 relationship**、再投影 **176.168s**、
  PG / Neo4j / API 件数一致。
- `cargo fmt --check`、`clippy -D warnings`、`cargo test --workspace` **213 passed**、
  OpenAPI / i18n **544 keys** / frontend build / `verify-db.sh` 001〜024 が緑。
- Phase 4 E2E: 12 files → 1 file（8.33%）、84 Fact evidence **84 / 84** byte range 一致。
- 増分 r1 / r2 の graph relationship は PG / Neo4j とも **84 / 84**。
- UI 1440 / 607px: GIT kind 表示、metric 5、Symbol 45、Fact 84、未解決 33、
  HTTP / console error 0。

### Known boundaries

- SVN は未導入。HTTP(S) / SSH Git remote は `GIT_ALLOW_REMOTE=1` の opt-in で、
  実閉域 server の E2E は未実施（local bare と `file://` は実施）。
- OpenRewrite / Java 21 semantic worker / 顧客 classpath は外部依存で未接続。
- 2.5M は Fact-equivalent synthetic data。解析そのものによる 2.5M Fact 生成、
  1,000+ JSP、顧客案件の意味解析品質はこの受入に含めない。
- Neo4j は 2026.05 Community で実測。設計上の 5.26 LTS での検証は別途必要。

## 2026-11-07 — Phase 5 正式受入: line seed・Golden set・SSE/PDF・6-hop

### Added

- Git unified diff parser と base/head 原文の決定的行差分。変更行を含む最小
  Class / Method / Field Symbol を seed にし、`changed_hunks` を API / UI / 帳票へ返す。
- Java 解析の Field 宣言、`READS` / `WRITES` Fact。Field 変更から読み書き Method へ
  閉包し、`CALLS` の incoming で呼び出し元へ逆依存探索する。
- Impact worker の DB frontier loader。全 `graph_meta` をロードせず、深さ・hub・
  node / edge budget を保ったまま層ごとに取得する。
- 20 変更 Golden set（`tests/fixtures/impact/golden-phase5.json`）と
  `scripts/phase5-golden-eval.py`。
- Q-01 の Git diff 入力、真の `text/event-stream`、`Last-Event-ID` 再開。
- Impact / Q-01 の `report.pdf`。日本語フォントをサブセット埋め込みし、
  フォント未配置は `CAPABILITY_UNSUPPORTED` として明示。
- UI の unified diff 入力、変更行、Impact / Q-01 PDF ダウンロード。
- 2.5M edge fixture に低次数 6-hop chain を追加し、frontier 性能を同じ runner で測定。

### Verification

- Golden set **20 changes**: recall **1.0**、precision **0.833333**、
  actual 66 items、evidence / path / why / quote / locator failure **0**。
- Line seed: `UserService.save` new line 11 → Method。
  Field seed: `UserEditAction.userService` L6 → `READS` → `UserEditAction.update()`。
- Q-01: `text/event-stream`、event ids 1〜6、`Last-Event-ID: 3` で
  `code → tests → done` のみ再送。`NEEDS_CLARIFICATION` 2 candidates、
  `NOT_FOUND_SEED`、counts / total 一致。
- Summary: ENTITY **0.018s**、自然言語 **0.086s**、DIFF **0.012s**。
- Markdown: Impact 7,412B / Inquiry 6,870B。PDF: Impact **117,073B** /
  Inquiry **133,554B**。日本語テキストを PDF から再抽出して検証。
- 2.5M Fact / **2,500,005 relationships** / Neo4j 再投影 PASS。
  6-hop frontier は **1.07 秒 / 6 items / depth 0〜5**。
- UI 1440 / 607px: diff 21 items / evidence 21、changed hunks 1、PDF link 1、
  Q-01 counts 5 / drafts 20、HTTP / console error 0。
- `cargo fmt --check` / clippy `-D warnings` / **222 tests passed** / OpenAPI /
  i18n **549 keys** / frontend build / `verify-db.sh` 001〜024 が緑。

### Known boundaries

- OpenRewrite / `javac` / 顧客 classpath は取得・実行しない。
- Golden set は合成 SAMPLE の人手確認用 fixture。顧客案件固有の意味精度ではない。
- RAG / reranker 併用 seed は未接続。
- 6-hop は 2.5M edge 中の低次数 chain。背景 graph の高次数 node は hub で剪定する。

## 2026-11-07 — Phase 8 残タスク: PITR・SCIM・運用完成

### Added

- `scripts/pitr-drill.sh`: 専用の一時 PostgreSQL クラスタで `archive_mode` + WAL アーカイブ、
  ベースバックアップ、指定時刻 / 最新 LSN への復元を実測（2.85 秒）。
- `scripts/backup.sh`: AES-256-CBC（PBKDF2）暗号化、保持期間、`BACKUP_UPLOAD_CMD`
  オフサイトフック。`scripts/restore-drill.sh` は復号と `CLEAN_ROOM=1` の
  Neo4j 投影クリア → rebuild に対応。
- `deployment/systemd/` と `deployment/launchd/` にバックアップ / 監査転送の定期実行テンプレート。
- `021_session_identity.sql` / `022_scim.sql` / `023_first_project.sql`。
  session の provider / subject / idp_sid、SCIM グループとメンバー、単一プロジェクト解決。
- OIDC back-channel logout（logout token の署名・iss・aud・exp・events・jti リプレイ検証、
  sub / sid で失効）。
- セッション管理 API / `/security` 画面（一覧・個別失効・他端末全失効）、
  アイドル期限（`SESSION_IDLE_MINUTES`）、worker による期限切れ削除。
- SCIM Users / Groups（Bearer、作成・一覧・PATCH active・削除、role 拡張）と、
  OIDC `groups` claim / SCIM メンバーからのグループ→ロール同期。
- `/projects/{pid}/members` と `/users` 画面（Owner がロール変更）。
- `scripts/audit-forward.py`（カーソル付き NDJSON、HMAC）と `scripts/mock-siem.py`、
  `scripts/phase8-siem-eval.sh`。

### Verification

- SSO/SCIM/セッション E2E **39 / 39**。back-channel 後の 401、logout token リプレイ 400、
  SCIM group→ARCHITECT、2 セッション → logout-all → 1 件、アイドル 401。
- PITR **2.85 秒**、指定時刻で変更前・最新 LSN で変更後を復元。
- 暗号化 + clean-room 復旧 **29 表**、RTO **5.20 秒**、drift 0。
- 監査 SIEM 転送 **326 行**、再実行 0 行（カーソル 326）。
- `cargo test --workspace` **211 passed**、fmt / clippy / OpenAPI / i18n 536 keys / build /
  `verify-db.sh` 001〜023 / UI 1440・607 が緑。

## 2026-11-07 — Phase 8 追補: セッション失効と認証強化

### Added

- `019_session.sql`: `core.session`（id / user_id / expires_at / revoked_at / last_seen_at /
  ip_hash / user_agent）。認証中参照のため RLS 対象外。
- セッション Cookie を `session_id:user_id:exp` の HMAC 署名に変更。毎リクエスト
  DB で失効・期限を判定し、`logout` は現在のセッションを `revoked_at` で失効させる。
  復旧訓練では復元後に全セッションを失効させる。
- 認証 3 経路（`/auth/login` / `/auth/oidc/start` / `/auth/oidc/callback`）に
  IP 単位のトークンバケット。既定 容量 10 / 毎分 10 補充。超過は
  `429 AUTH_RATE_LIMITED` + `Retry-After`。`TRUST_PROXY=1` のときだけ
  `X-Forwarded-For` を使う。
- 全 API 応答に `nosniff` / `Referrer-Policy: no-referrer` / `X-Frame-Options: DENY` /
  `Permissions-Policy`。`SESSION_COOKIE_SECURE=true` のときだけ HSTS。
- `auth.password.login` の監査（SUCCESS / FAILED、login_id は SHA-256、理由付き）。
- `docs/phase8/operations.md`（バックアップ / 復旧 / SSO / セッション失効 /
  レート制限 / 監査 / インシデント）。

### Verification

- SSO/セキュリティ E2E: **26 / 26**。logout 後の Cookie 再利用 401、
  6 回目で 429 + `Retry-After: 60`、ヘッダ 4 種、パスワード監査の SUCCESS / FAILED
  （累積。最新実行で FAILED 8 / SUCCESS 2）。
- 復旧訓練: 27 表一致、復元後 live session 0、RTO **2.51 秒**、graph drift **0.0**。
- `cargo test --workspace` **211 passed**、fmt / clippy / OpenAPI / i18n / build /
  `verify-db.sh` 001〜019 / UI 1440・607 が緑。

## 2026-11-07 — Phase 8: 運用引渡し（復旧訓練 + OIDC SSO）

### Added

- `018_sso.sql`: `core.external_identity`（provider=issuer, subject, email, last_login_at）。
  `(provider, subject)` と `(user_id, provider)` の一意制約。認証前参照のため RLS 対象外。
- OIDC 認可コード + PKCE(S256)。discovery / JWKS / state / nonce / ID token 検証は
  `openidconnect` 4.1。`providers::OidcRuntime` は discovery を遅延キャッシュし、
  交換失敗時に捨てる。
- `GET /api/v1/auth/config`、`GET /api/v1/auth/oidc/start`、
  `GET /api/v1/auth/oidc/callback`。未設定・IdP 停止は `503 AUTH_SSO_UNAVAILABLE`。
- フロー Cookie（10 分、HttpOnly、SameSite=Lax、Path 限定）に state / nonce / verifier /
  return_to を署名して保持。オープンリダイレクトを拒否する。
- 未知の subject は自動プロビジョニングせず `/login?sso_error=not_linked`。
  管理 SQL で `core.external_identity` に紐付ける。
- `auth.sso.login` の監査（SUCCESS / FAILED、subject は SHA-256、失敗理由）。
- ログイン画面の SSO 入口と理由別エラー（ja / en / zh-CN 9 キー）。
- `scripts/backup.sh`（PG dump + 原本 tar + manifest）、`scripts/restore-drill.sh`
  （別 DB 復元、26 表の件数、9 表の内容ハッシュ、RLS、FTS、evidence、原本 SHA-256、
  graph rebuild と drift 0）。`scripts/backup-tables.txt` を共有。
- `scripts/mock-oidc-idp.py`（stdlib + openssl、RS256）、`scripts/phase8-sso-eval.py`、
  `apps/web/scripts/ui-check-phase8.mjs`。
- `docs/phase8/acceptance.md`。

### Verification

- 復旧訓練: 26 表一致、9 表ハッシュ一致、未スコープ chunk 0 / スコープ内 1839、
  FTS 1839、evidence 1996、原本 386 ファイル、RTO **2.46 秒**、graph drift **0.0**（Pg/Neo4j 5）。
- OIDC E2E: **19 / 19**、ログイン往復 **10〜13 ms**。state 改竄 / nonce 不一致 / 未紐付け /
  IdP 停止 / パスワード併存を実測。
- UI: 1440 / 607 で SSO 入口とパスワードフォームが併置、607px 横 overflow なし。
- `cargo fmt --check` / `clippy -D warnings` / `cargo test --workspace`（203 passed）/
  OpenAPI 一致 / i18n 505 keys / `verify-db.sh` 001〜018 が緑。

## 2026-11-07 — Phase 5 追補: Q-01 変更影響問合せ

### Added

- `017_inquiry.sql`: `impact.change_inquiry`（質問・入力種別・状態・候補・5 セクション）。
  RLS と `core.project_of('change_inquiry')` を追加。
- Q-01 の 5 セクション（summary / screens / documents / code / tests）。
  ImpactReport からの投影で、新しい走査はしない。`summary.headline` は件数テンプレート。
- 自然言語シード解決（グラフ実体名の決定的な部分一致）。一意なら進行、
  曖昧なら `NEEDS_CLARIFICATION` + 候補、0 件は `NOT_FOUND_SEED`。DIFF は 400。
- テスト編成: 既存ケースは `covers_impacts` で照合、草稿は最大 20 件を提案（保存しない）。
- `/projects/{pid}/change-inquiries`（POST/GET）、`/{iid}`、`/{iid}/events`（NDJSON）。
- `/change-inquiries/{iid}/test-plans:derive`。Q-01 から DRAFT 計画を作る
  （`test_run` は作らない）。
- `InquiryPage.tsx`（5 セクション、候補選択、イベント順、計画作成）と三語の 34 キー、
  `ui-check-phase5-inquiry.mjs`。
- Markdown 出力: `GET /change-inquiries/{iid}/report.md` と
  `GET /impacts/{iid}/report.md`（`text/markdown` + attachment）。Q-01 は 5 セクション、
  Impact は件数・影響項目・証拠・未検証・打切り・降格を表にする。
- 画面のダウンロードボタン（Q-01 / Impact）。
- `POST /change-inquiries/{iid}/narrate`。AI 未設定なら `narrative=null` +
  `ai_unavailable`（5 セクションは残る）。設定時は `ai_inferred` の叙述を返し、
  レポートに保存する。`InquirySummary.narrative` を追加。
- `scripts/phase5-inquiry-eval.py` と `data/phase5-inquiry/report.json`。

### Verification

- ENTITY: コード 2 件、テスト草稿 20 件、`counts` と各 `total` が一致、
  先頭項目に `why` / `quote` / `locator`。
- 自然言語: 一意で `SUCCEEDED`、同名 2 件で `NEEDS_CLARIFICATION`（候補 2）、
  0 件で `NOT_FOUND_SEED`。同じ質問は同じ `inquiry_id`。DIFF は 400。
- `/events` の順序は `summary` → `screens` → `documents` → `code` → `tests` → `done`。
- Markdown: Q-01 3,198 bytes / Impact 1,038 bytes。見出し・表・証拠・降格を含む。
- AI 叙述: 未設定で `narrative=null` / 5 段残存。mock LLM で生成し `ai_inferred`、
  `model=mock-1`。
- 「計画に入れる」で草稿 20 件の DRAFT 計画が作られ、`test_run` は 0 件のまま。
- 計画を指定した run は当該計画のケースだけを数える（承認前 DRAFT なので 0 件）。
  全件 run（`plan_id=null`）は従来どおり全承認済みケースを対象にする。
- UI: コード 2 件 / 草稿 20 件 / 件数カード 5 / 607px 1 列 / console・HTTP error 0。
- fmt / clippy / test / OpenAPI / `verify-db.sh`（001〜017）/ 文書検査。

### Known limits

- `/events` は当時の NDJSON 一括返却。同日の Phase 5 正式受入で真の SSE に置換した。
- 自然言語は RAG を使わず部分一致のみ。`DIFF` 入力、Markdown / PDF、
  20 変更の recall / precision は未実装。
- テスト草稿は保存しない（`test-plans:derive` で正式化する設計のまま）。


## 2026-11-07 — Phase 7: AI Agent / MCP / Workflow 縦断実装

### Added

- `016_ai.sql`: ai schema 10 表（全表 RLS）、`core.project_of` に agent_run /
  workflow_run / approval / mcp_server の 4 分支。roles / verify-db / compose を同期。
- `providers::AiProvider` と `OpenAiCompatProvider`（http の `/chat/completions`）、
  `DisabledAiProvider`。`model_profile` を環境変数へ写像し、未設定なら生成しない。
- `runtime` に `BudgetUsage` / `budget_exceeded` と、impact / test / coverage /
  project の能力を追加。
- Agent 登録と版の自動繰り上げ、Prompt 保存、`AGENT_RUN` job、
  `narrative` / `hypotheses` の構造検証、`AI_UNAVAILABLE` の明示。
- Agent のツール呼び出しループ。`tools` / `tool_calls` を OpenAI 互換で送受し、
  Agent の `capabilities` と照合、外れたら `AGENT_CAPABILITY_DENIED` で停止。
  ツール結果は 2,000 字で切って `role=tool` で返す。
- MCP Server: 12 ツールの公開、`initialize` / `tools/list` / `tools/call`、
  `schema_hash` の安定記録、承認必須ツールの 423。
- Workflow: JSON 定義（start / capability / approval / end）、`WORKFLOW_RUN` job、
  承認ノードで `WAITING_APPROVAL`、承認後の再開。read 以外の能力は定義時点で拒否。
- Workflow の可視化エディタ（SVG）。ノードのドラッグ配置・接続・属性編集・
  新版本保存。`WorkflowNode.x/y` を追加（実行には未使用）。`GET /workflows/{wid}`。
- 承認の起票・判断・留痕、`ai.*` の監査、`/projects/{pid}/ai/*` 14 パス。
- `AiPage.tsx` と ja / en / zh-CN の 43 キー、`scripts/phase7-eval.py`、
  `ui-check-phase7.mjs`、`docs/phase7/acceptance.md`。

### Verification

- AI 未設定で E2E: `ai_status=unavailable`、`agent_run=FAILED/AI_UNAVAILABLE`、
  `output=null`、12 ツール、`schema_hash` 2 回同一、承認必須 423、未知ツール拒否、
  viewer の Agent 登録 403 と MCP 読み取り 200、Workflow の停止 → 承認 → `SUCCEEDED`、
  監査 23 行。
- mock LLM（OpenAI 互換）でツール呼び出しループを実測:
  `project_list` 実行 → `SUCCEEDED` / `narrative` あり / `tool_calls=1`、
  ホワイトリスト外は `AGENT_CAPABILITY_DENIED` で `output=null`。
- `cargo fmt --check` / `clippy -D warnings` / `cargo test --workspace` /
  OpenAPI 一致 / `verify-db.sh`（001〜016）/ i18n / フロント build / UI 検査。

### Known limits

- 外部 MCP Client への実接続、Workflow エディタ、YAML 定義、
  Q-01 への narrative 供給は未実装。実モデルでのツール選択精度は未検証。
- MCP 12 ツールのうち `tools/call` が動くのは 3 つ。他は `CAPABILITY_UNSUPPORTED`。
- AI は http のみ（TLS 非対応）。予算は判定関数のみで、実消費は未計測。

## 2026-11-07 — Phase 6 追補: Playwright Runner 接続

### Added

- `workers/runner/runner.mjs`: JSON DSL の実実行。4 並列、1 case = 1 context、
  `ja-JP` / `Asia/Tokyo`、10 秒ステップ上限、8 分類、`TIMEOUT` / `NETWORK` のみ
  1 回再試行。結果は `execution_source=PLAYWRIGHT` で `executions:import` へ戻す。
- 失敗時のみ screenshot / DOM / console / trace の 4 点と HAR を保存。
  成功時は trace を残さない。HAR は `Cookie` / `Authorization` 等を伏せ字化。
- `samples/sample-web/`: Runner の被テストシステム（合成 SAMPLE 静的サイト）。
- `scripts/phase6-runner-eval.py`: 150 ケース生成 → 100 ケース実行 → API 戻しの受入。
- `derive_assets` のケース ID に plan_code を付与し、複数計画のケースが衝突しないよう修正。
  `max_drafts` の上限を 200 に緩和（Q-01 経由の既定は 20 のまま）。
- `TEST_RUN` job の注意コードを `RUNNER_NOT_CONNECTED` から
  `RUNNER_EXTERNAL_PROCESS` へ変更。画面のバッジと degraded も追従。
- `derive` 時に Requirement / TestCase / `VERIFIES` を PG の `graph_meta` へ投影し、
  被覆の Requirement 指標を辺から計算する。Requirement はルール由来のため
  `INFERRED` とし、`REQUIREMENT_INFERRED_FROM_RULE` を明示する。

### Verification

- 合成 SAMPLE で **100 ケース / 4 並列 / 約 15〜17 秒**、PASSED 99 / FAILED 1、
  初回合格率 99%。失敗 1 件に 4 点成果物、PASSED の `artifacts` は 0 件、
  HAR 100 件の伏せ字化と秘密値残留 0 を確認。
- API に `PLAYWRIGHT` 実行 100 件が受理され、欠陥候補 1 件を生成。
- Requirement 被覆が `VERIFIES` 辺から 0 / 4 → 4 / 4 に算入されることを確認。
- R-01 / 09 / 10 / 11 を実測で PASS、R-17 は条件付き PASS に更新。
- `data/phase6-runner/report.json` / `acceptance.json` に実測値。

### Known limits

- 実案件規模（90 秒/ケース、7,000 ケース全回帰、32 並列）と
  SAMPLE 24 画面 / 300 ルールの正解集は未検証。合成静的 SAMPLE の計測である。
- Collector 分離、複数ノード、`storage_state`、データ seed / reset、
  成果物の保持期限と容量上限、§13 の失敗モード網羅は未実装。
- Runner はローカル受入スクリプト起動のみで、CI ジョブには未登録。
- 実要件からの Requirement 抽出と、テスト資産の Neo4j への outbox 投影は未実装。
  被覆は PG の `graph_meta` だけを読む。

## 2026-11-07 — Phase 6: Test Engineering 縦断実装

### Added

- `015_test.sql`: test schema 11 表（全表 RLS）、環境ロックの部分一意索引、
  `core.project_of` に test_plan / test_case / test_run / test_execution / defect の 5 分支。
  `roles.sql` / `verify-db.sh` / `docker-compose.yml` を同期。
- `engine-test`: ルール抽出（競合は両方残す / 未解析は数える）、
  Scenario / Case / Data の決定論的派生、日本語特有データ、DSL 保存時検証、
  失敗 8 分類と再試行規則、fingerprint、被覆 5 指標。
- `TEST_RUN` worker job。対象ケース数を確定し `READY` で停止する。
  Playwright Runner は接続せず、PASSED を生成しない。
- `executions:import` による実行結果の取込み（`execution_source` 必須）、
  fingerprint による欠陥候補の再発集約、被覆の冪等再計算。
- `/projects/{pid}/test/*` 8 端点と OpenAPI、`TestPage.tsx` と ja / en / zh-CN の 53 キー。
- `scripts/phase6-test-eval.py`、`golden_phase6.rs`、
  `apps/web/scripts/ui-check-phase6.mjs`、`docs/phase6/acceptance.md`。

### Verification

- `cargo fmt --check` / `cargo clippy --workspace --all-targets -- -D warnings` /
  `cargo test --workspace` / OpenAPI 一致 / `scripts/verify-db.sh`（001〜015）全項目緑。
- 単体 27 passed、golden: 合成 300 ルールから 1,200 組のデータ、DSL 合格比 1.0、
  被覆の再現性。
- API + worker E2E: R-02 / 03 / 04 / 05 / 06 / 07 / 08 / 12 / 13 / 14 / 15 / 16 が PASS。
  実測は `data/phase6-eval/report.json`。
- UI: rules 4 / cases 15 / runs 3 / executions 16 / defects 2 / 被覆カード 5、
  607px で 1 列、console・HTTP error 0。
- `detail/02` のマイグレーション順表を実ファイル（014 impact / 015 test）へ修正。

### Known limits

- Playwright Runner、100 ケース / 4 並列の実測、成果物 4 点、trace 容量、
  HAR マスキング、SAMPLE 24 画面の初回合格率は未実施。
- Requirement Coverage はグラフの `VERIFIES` 辺が無いため 0 + `degraded`。
  近似値で埋めない。
- 自然言語 seed、Q-01 の 5 セクション SSE、Markdown / PDF、20 変更の
  recall / precision は Phase 5 から引き継いだ未完了項目。

## 2026-11-07 — Phase 5: Evidence-Bounded Impact Draft 縦断実装

### Added

- `impact.assessment_draft`（`014_impact.sql`）、RLS、`core.project_of('impact_assessment')`。
- `engine-impact` の双方向 BFS、最短経路、hub / depth / budget 打切り、弱い resolution の伝播、
  決定論的 risk と stable sort。
- ENTITY と DIFF seed。DIFF は code revision の hash 差分から Module / Symbol を解決。
- コード Fact evidence と文書 chunk evidence の統合。証拠なしは `unverified` へ分離。
- `IMPACT_ANALYSIS` worker job、Draft の PENDING / RUNNING / SUCCEEDED / PARTIAL / FAILED。
- `POST/GET /projects/{pid}/impacts`、`GET /impacts/{iid}`。
- 影響分析画面を固定プロトタイプから API 接続へ置換。ENTITY / DIFF、risk、path、evidence、
  degraded / truncated / unverified を表示。
- `seed-phase5-doc.sql`、`phase5-impact-eval.py`、`ui-check-phase5.mjs`、
  `docs/phase5/acceptance.md`。

### Verification

- ENTITY: 4 evidence-backed items、同一 request の impact_id / report が一致。
- DIFF: 1 changed file → 4 seeds、18 evidence-backed items、文書 1 件、risk 0.243 LOW。
- 主表 evidence 欠落 0。`approval_state=DRAFT`、`code_modified=false`、`tests_executed=false`。
- UI: ENTITY 35 / evidence 35、DIFF 18 / evidence 18、607px 1 列、HTTP / console error 0。
- DB: 001〜014、impact RLS、worker 更新権限、resource scope 解決を検証。

### Known limits

- 自然言語 seed / `NEEDS_CLARIFICATION`、raw Git diff、line-level / field closure、
  20変更の recall / precision、真の SSE、PDF は当時未実装。同日の Phase 5 正式受入で解消した。
- コード・文書リンクは simple symbol 名の chunk 一致で `INFERRED`。意味リンクではない。
- テスト資産照合は未実装で `tests_not_available` を常に明示する。
- risk は breadth / uncertainty / depth / truncation のみ。業務重要度・欠陥密度・被覆は未接続。

## 2026-11-07 — Phase 4: Code Analysis source-only 縦断実装

### Added

- `code` schema（`013_code.sql`）: repository / revision / source_file / symbol /
  analysis_run / classpath_index / fact / fact_evidence。8 表の RLS、遅延 FK、
  revision + artifact の一意制約。
- `engine-code`: UTF-8 / Shift_JIS / Windows-31J / EUC-JP の元バイト偏移、
  Tree-sitter Java、JSP 等長マスク / scriptlet 合成、Struts / Dicon / MyBatis XML、
  基本 SQL 表参照、Fact evidence。
- Java の project-local 型・メソッド・引数数による `RESOLVED` 呼び出し、複数候補の
  `INFERRED`、外部型の `UNRESOLVED`。反射 / 動的プロキシは常に非 `RESOLVED`。
- `CODE_SYNC` / `CODE_ANALYZE` worker、snapshot、親版との hash 差分、未変更
  Symbol / Fact の再利用、`supersede_then_insert`。
- repository / revision / analysis-run / symbol / fact / source content API。
- コード解析画面: 同期、品質・降格警告、未解決率、Symbol、Fact、byte offset と quote。
- `tests/fixtures/code/golden-phase4.json`、`golden_phase4.rs`、`seed-phase4.sh`、
  `phase4-eval.py`、`ui-check-phase4.mjs`、`docs/phase4/acceptance.md`。

### Verification

- `cargo fmt --check`、`cargo clippy --workspace --all-targets -- -D warnings`、
  `cargo test --workspace`。
- golden source SAMPLE: 41 class / 250 method、200 / 200 project-local CALLS、
  反射 30 / 30 非 `RESOLVED`、1 / 12 ファイル（8.33%）の増分解析。
- API + worker E2E: 84 Facts / 45 Symbols、evidence 84 / 84 を元バイト範囲から再読して一致。
  Struts `HANDLES`、MyBatis `QUERIES`、Shift_JIS class を確認。
- Workflow エディタ: 4 ノード → ドラッグ → 承認追加（5 件）→ 保存で v2、607px 1 列。
- UI: 1440px / 607px。実測値は `metrics=5`、`symbolRows=45`、`factRows=84`、
  降格 2、未解決 33、console / HTTP error 0。
- DB: 001〜013、code RLS 8 表、worker 権限、`code_revision` の project 解決、
  supersede 後 live Fact 1 行を検証。

### Known limits

- OpenRewrite Java 21 worker、顧客 classpath、SVN、TLD 完全対応、JSqlParser、
  1,000+ JSP の実規模受入は未完了。Git、Fact COPY、13,000 Java、2.5M Fact、
  Neo4j 再投影は同日の正式受入エントリで解消した。
- `classpath_complete=false`。source-only のため外部 library の overload / signature は解決しない。
- Neo4j 投影は PG / outbox の生成まで E2E で確認し、Neo4j は無効で検証した。

## 2026-11-07 — Phase 3: Graph + Ontology

### Added

- Ontology v1（56 実体型 / 27 関係型）と `graph_meta` schema（`011_graph.sql`）。
  Ontology の閲覧 / 公開 API と画面。
- PG 正本 + Outbox + Neo4j 投影: `DocumentNodeUpserted` / `RelationshipUpserted` /
  `ProjectionTombstone`、worker の outbox drain、dead letter、`projection_state` / `projection_build`。
- 実体検索 / 隣接 / 経路 / サブグラフ / トレーサビリティ API（深さ 3 ホップ）。
  PgGraphProvider lite は再帰 CTE。Neo4j 障害時も PG API は継続。
- グラフブラウザ（検索・SVG サブグラフ・祖先 / 子孫・統計・再構築）と Ontology 画面。
- `migrations/012_graph_relationship_seq.sql`（関係の `projection_seq`）。
- `apps/web/scripts/ui-check-phase3.mjs`、`scripts/verify-db.sh` の graph_meta 検査。
- `docs/phase3/acceptance.md`、`detail/07` §12、UI / API / DDL / 技術選定の実装差分。
- `docker-compose.yml` に Neo4j 5.26 service と 010〜012 の初期化 SQL。

### Changed

- 実体 / 関係の順序保証を `projection_seq` の条件付き書き込みに統一。再構築は PG の
  `projection_seq` を保持し、関係の `rel_id` は PG の ID を使う（再構築後の重複辺 0）。
- Ontology の公開版を実体の `ontology_version` とグラフ統計に反映。
- グラフ画面のレイアウトからインラインスタイルを除去（`section-gap` / `m-0`）。

### Verification

- `cargo fmt --check` / `cargo clippy -D warnings` / `cargo test --workspace`（147 passed, 0 failed）。
- OpenAPI 一致 / `scripts/verify-db.sh`（001〜012 + graph_meta 検査）全項目緑。
- 受入: PG 5 / Neo4j 5、関係 4 / 4、drift 0.0。再構築 2 回で重複辺 0。
  順序逆転（古い seq）が上書きしないことを実測。投影器 3 秒停止から復旧。
  隣接は 10 万ノードの合成データで P95 5.325ms。
- 文書削除から 4 秒以内に Neo4j の関連ノードが消滅。
- UI ブラウザ受入: グラフ / Ontology / 公開（1440 / 607）。Phase 1 / 2 の UI 受入も再実行し回帰なし。
- 未実施または降格: 影ラベル再構築、日次漂流検出、投影器 10 分停止、複数 projector、
  Neo4j 5.26 LTS、Neo4j 込み compose（`docs/phase3/acceptance.md`）。

## 2026-11-07 — Phase 2: RAG Engine 完成

### Added

- XLSX 解析: 共有文字列、値、数式キャッシュ、日付、コメント、定義名、非表示シート、結合セル、
  図形アンカー。領域検出、ヘッダ判定、12 行 / 800 token チャンク、`locator`。
  親 region chunk と行 evidence（`XLSX_ROW`、子 chunk は保存しない）。
- DOCX / PPTX: OOXML 直読み（見出し / 段落 / 表 / スライド / 図形）。`section_path` / `slide` locator。
- 構造木 `GET /documents/{did}/tree`、文書チャンク一覧 `GET /documents/{did}/chunks`。
- 一括取込 `POST /projects/{pid}/documents:bulk-import`（JSONL、部分成功、失敗一覧、チェックポイント再開）。
- 検索: Lindera 6.2（IPADIC）の単語 + 文字 bigram、RRF（k=60）、ローカル語彙一致 rerank、
  `rerank_score` / `degraded: rerank_local_heuristic`。
- 索引管理: `index_build` の `is_active`、`local-hash-1024-v2`、build API、有効化 API、再埋め込みジョブ。
- UI: Chunk Explorer、索引管理、検索検査モード。XLSX / DOCX / PPTX の文書詳細フォールバック。
- `migrations/010_index_build_active.sql`、`scripts/phase2-eval.py`、`apps/web/scripts/ui-check-phase2.mjs`。
- `docs/phase2/acceptance.md`（実測値と未実施項目）。

### Changed

- upload / register の対応形式を PDF / XLSX / XLSM / DOCX / PPTX に拡張。
- `systivra_scheduler` / `systivra_worker` に子ジョブ投入用の `core.job` INSERT を許可。
- 技術選定の Lindera を 6.2.0、MSRV を 1.88 に更新。
- 一括取込の manifest は `SYSTIVRA_DATA_DIR` 配下に限定。

### Verification

- `cargo fmt --check` / `cargo clippy -D warnings` / `cargo test --workspace`（145 passed, 0 failed）。
- OpenAPI 一致 / `scripts/verify-db.sh`（010 含む）全項目緑。
- 60 問評価: recall@10 1.0（word / partial / identifier・外字 各 1.0）、MRR 0.847、P95 39ms。
- 50 sheet / 30 セル領域の top-3: 100%。一括取込の部分成功と、kill → 再開（30/30、重複 0）。
- UI ブラウザ受入: Chunk Explorer / 索引管理 / 検索検査（1440 / 607）。
- 未実施または降格: docling-serve、pg_bigm、外部 rerank、親子 chunk、partial HNSW、
  1,000 冊 / 1,200 万 chunk の負荷試験（`docs/phase2/acceptance.md`）。

## 2026-11-06 — P1 docs/code alignment

- DDL 文書に `SUCCEEDED_WITH_ERRORS`、`core.job_event`、`core.project_of`、24 か月の audit 分区を反映。
- `doc_node` は Phase 1 非分区であることを DDL / データモデルに明記。
- アーキテクチャ依存表を実際の Cargo 依存（app/worker → engine-rag の Phase 1 例外）に合わせた。
- 技術選定の sqlx 0.8.6 / tree-sitter 0.25.10 / Playwright 1.65.0 / npm lock に更新。
- UI 文書に Phase 1 の 5 画面・3 入口・未実装の Job Center / 仮想スクロール / ICU を明記。
- OpenAPI に一覧 query、監査 limit、parse の Idempotency-Key、SSE の Last-Event-ID を追加。
- 検索に `hnsw.ef_search=80` / `iterative_scan=relaxed_order` を適用し、worker で quote の部分文字列を検証。
- 検索 P95 を 20 回計測して 49ms を記録。

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
