# Phase 6 受入記録（Test Engineering）

実施日: 2026-11-08  
対象: `08-実装計画.md` Phase 6 / `detail/09-test-runner.md` / `detail/08-change-inquiry.md` §6・§7  
環境: macOS arm64、PostgreSQL 18.4 + pgvector + pg_trgm、Rust 1.99.1、
Node.js 26.0.0、Neo4j 2026.05 Community（投影の実測時のみ）、Chrome headless

> 判定: **Phase 6 は正式受入に到達**。
> ルール抽出 → シナリオ / ケース / データ派生 → 承認 → run 起票 →
> ブラウザ実行（Playwright Runner）→ 被覆・欠陥候補、に加えて
> SAMPLE **24 画面 / 300 ルール**の合成 fixture と正解集、**1,200 組**のデータ派生、
> **100 ケース / 4 並列 12.3 秒**、初回合格率 **100%**、`detail/09` §13 の失敗モード、
> 成果物の容量上限 / 保持期限、`storage_state` の鮮度確認、データ seed / reset、
> `TestCase -[:VERIFIES]-> Requirement` の outbox → Neo4j 投影、Runner 単体 CI を実測した。
> **Collector 分離 / 16 ワーカー複数ノード / 実案件 90 秒/ケース / 7,000 ケース全回帰は
> 環境・顧客データ依存として未検証**であり、§3 に明記する。

---

## 1. 実装済み範囲

| 領域 | 実装 |
|---|---|
| DDL | `015_test.sql`。test schema 11 表、全表 RLS、`core.project_of` の分支、環境ロック部分一意索引 |
| ルール | コード片 / 設計書記述から `REQUIRED / LENGTH / RANGE / PATTERN / CROSS_FIELD` を抽出。競合は両方残し `CONFLICT`。読めない記述は `unparsed` に数える |
| 派生 | ルール × 影響項目から Scenario / DRAFT Case / データを決定的に生成。`base_path` を API から指定でき、画面ごとの計画に分けられる |
| DSL | v=1 語彙、保存時検証（未知 action、`wait_for.ms>5000`、脆い CSS、外部オリジン、判定 0 件、秘密参照）、`LOCAL` / `STAGING` 以外は破壊的ケースを `SKIPPED` |
| 承認 | `test:approve` を持つ人間だけ。DSL 検証は保存時と承認時の両方で走る |
| Runner | `workers/runner/runner.mjs`（Node + playwright-core）。1 case = 1 context、4 並列、ステップ 10 秒 / ケース 180 秒上限、8 分類、`locale=ja-JP` / `Asia/Tokyo` |
| §13 失敗モード | 先頭 3 件全滅で `ENV` へ切替えて run 停止（残りは `SKIPPED`）、flaky 集計、`DATA` は reset 強制、成果物 0 件は `UPLOAD_FAILED`、環境ロック待ち、run 24 時間上限 |
| storage_state | `STORAGE_STATE` の mtime と `STORAGE_STATE_MAX_AGE_MIN`（既定 480 分）で鮮度確認。期限切れ / 欠落は実行前に `ENV` で中断し run を作らない |
| データ | `DATA_MODE=NONE/SEED/RESET_ENDPOINT` + `SEED_URL` / `RESET_URL`。run 開始時に投入 / 初期化し、`DATA` 失敗時は reset を強制 |
| 成果物 | 失敗時のみ screenshot / DOM / console / trace の 4 点 + HAR（伏せ字）。trace 20 MB 上限（超過は `TRUNCATED`）、run 合計 20 GB 上限（超過後は trace 無効） |
| 保持 | 画像 / DOM / log 90 日、trace 30 日。期限超過は実体を削除して `EXPIRED` を立てる（リンク切れにしない）。worker が日次で適用、API に手動適用 / run 単位削除 |
| 実行結果 | `executions:import`。8 分類、再試行規則（TIMEOUT / NETWORK のみ 1 回）、`attempt` 別行、fingerprint、欠陥候補の再発集約 |
| 被覆 | 5 指標。分子分母を残し、同じ `test_run` からは常に同値。Requirement は PG グラフの `VERIFIES` 辺から取る |
| グラフ投影 | `derive` が同じ transaction で PG `graph_meta` と `core.outbox_event` に書く。worker の projector が Requirement / TestCase / `VERIFIES`（resolution=INFERRED）を Neo4j へ MERGE する |
| 環境ロック | 同一 `environment_id` の活性 run は 1 つ。2 つ目は `QUEUED` で待ち、Runner は `READY` になるまで待ってから実行する |
| API | `/projects/{pid}/test/*` 10 端点（`artifacts:expire` / `runs/{rid}/artifacts:cleanup` を含む） |
| UI | テスト工学画面。被覆 5 指標、成果物チップ（`TRUNCATED` / `EXPIRED` / `UPLOAD_FAILED`）、run 単位の成果物削除、保持期限の適用 |
| CI | Runner 単体テスト（`workers/runner/runner.test.mjs`）を `.github/workflows/ci.yml` の `runner` ジョブで実行 |

---

## 2. 受入結果

| # | 受入 | 結果 | 証拠 |
|---|---|---|---|
| R-01 | 100 ケースを 4 並列で 60 分以内 | **PASS** | 100 件 / 4 並列 / **12.32 秒**（24 画面 SAMPLE、`data/phase6-sample/report.json`）。成果物収集込みの 100 件は 12.57 秒 |
| R-02 | 同じ `test_run` の被覆を 2 回計算し 5 指標すべて同値 | **PASS** | `phase6-test-eval.py` が 2 回の応答完全一致を確認 |
| R-03 | `DRAFT` ケースが被覆に算入されない | **PASS** | 承認 120 / DRAFT 1,128。`excluded` に草稿を数える |
| R-04 | 「生成したが未実行」は被覆 0 | **PASS** | 取込み前 `rule.num=0`、`never_executed` を表示 |
| R-05 | `ASSERTION` 失敗が再試行されない | **PASS** | attempt=2 を `retry_not_allowed:ASSERTION` で拒否 |
| R-06 | `TIMEOUT` は 1 回だけ再試行され attempt=2 が残る | **PASS** | flaky 実測で attempt=2 の PASSED 行を受理 |
| R-07 | `PRODUCTION` で `@destructive` が `SKIPPED` | **PASS** | 二層（API は LOCAL / STAGING 以外を 400、取込は許可リストで `SKIPPED`） |
| R-08 | `allowed_origins` 外への `goto` を保存時に拒否 | **PASS** | 422 + `origin_not_allowed` |
| R-09 | 失敗時に画像 / DOM / console / trace の 4 点 | **PASS** | 失敗 1 件に 4 点 + HAR。run 合計上限超過後は trace を落とす |
| R-10 | 成功時に trace を保存しない | **PASS** | PASSED 99 件の `artifacts` は 0 件 |
| R-11 | HAR の `Cookie` / `Authorization` 伏せ字 | **PASS** | HAR 100 件を伏せ字化、秘密値の残留 0 |
| R-12 | 同じ環境に 2 run を投入すると 2 つ目が `QUEUED` | **PASS** | Runner 2 本で実測。2 本目は **6,038 ms** 待って起動し、両方 `DONE` |
| R-13 | 同一 fingerprint の失敗が欠陥候補 1 件にまとまる | **PASS** | 欠陥候補 1 件（fingerprint 付き、再発集約） |
| R-14 | `wait_for.ms=6000` を保存時に拒否 | **PASS** | 422 + `wait_for_ms_exceeds_5000` |
| R-15 | `v=1` の未知 action を保存時に拒否 | **PASS** | 422 + `unknown_action` |
| R-16 | Q-01 から派生した計画は `DRAFT` で、`test_run` を作らない | **PASS** | `test_run_created=false`。計画指定 run は当該計画のケースだけを見る |
| R-17 | SAMPLE 24 画面 / 300 ルールで初回合格率 60% 以上 | **PASS** | 300 ルール、1,200 組、承認 120、**初回合格率 100% / 正解集一致 100%**。画面被覆 24 / 24 |

### 2.1 API + worker E2E

独立 DB `systivra_phase6_verify`、API `18086/18088`。Q-01 の Impact Draft は
fixture を直接投入し、実コード解析のやり直しを避けた。

ルール抽出:

```json
{"parsed":4,"conflicts":1,"unparsed":1,
 "degraded":["rules_not_parsed","rules_conflict","rules_without_evidence"]}
```

計画派生（R-16）:

- シナリオ 4、草稿ケース 15、`drafts_truncated=0`、`test_run_created=false`。
- 承認 14 件、DRAFT 1 件を意図的に残す。

環境ロック（R-12）:

- 1 つ目: `job_id` あり、`queued_behind=false`。
- 2 つ目: `queued_behind=true`、`job_id=null`。
- `TEST_RUN` job は `SUCCEEDED_WITH_ERRORS`（`RUNNER_EXTERNAL_PROCESS`）。
  Runner は別プロセスであることを示す注意であり、未接続の意味ではない。
- 1 つ目完了後、2 つ目が `READY` / `lock_acquired=true` で起動。

被覆（取込み前 → 後）:

| 指標 | 取込み前 | 取込み後 |
|---|---|---|
| rule | 0 / 4 | 4 / 4 |
| screen | 0 / 1 | 0 / 1 |
| impact | 0 / 2 | 1 / 2 |
| regression | 0 / 14 | 13 / 14 |
| requirement | 0 / 4 + `REQUIREMENT_INFERRED_FROM_RULE` | 4 / 4 + 同上 |

`screen` が 0 のままなのは、画面由来のケースを意図的に DRAFT で残したため。
`requirement` は `derive` が PG グラフへ投影した
`TestCase -[:VERIFIES]-> Requirement` 辺から計算する。Requirement 自体は
検証ルールから推定したものなので、`REQUIREMENT_INFERRED_FROM_RULE` を付けて
「実要件を検証した」と誤読されないようにする。

実行結果の取込み:

```json
{"first":{"accepted":14,"rejected":["...: case_not_approved_or_unknown"]},
 "second":{"accepted":2,"rejected":["...: retry_not_allowed:ASSERTION"]}}
```

実測結果: `data/phase6-eval/report.json`。

### 2.2 Runner の実測（`phase6-runner-eval.py`）

SAMPLE 静的サイトを配信し、承認 150 ケースから 100 件を 4 並列で実行。

| 項目 | 実測 |
|---|---|
| 実行 | 100 件 / 4 並列 / **12.57 秒** |
| 結果 | PASSED 99 / FAILED 1（意図的に古い期待値を注入） |
| 失敗分類 | `ASSERTION` 1 |
| 成果物 | 失敗 1 件に 4 点 + HAR、成功時の trace 0 |
| HAR | 100 件を伏せ字化、`phase6-secret-token-do-not-leak` の残留 0 |
| API 戻し | `execution_source=PLAYWRIGHT` で 100 件受理、拒否 0 |
| 欠陥候補 | 1 件（fingerprint 付き） |

### 2.3 SAMPLE 24 画面 / 300 ルール（`phase6-sample-eval.py`）

`samples/sample-web/sample/s/01..24/edit/index.html` を生成（各画面は同一の
検証語彙を実装し、ラベルは DSL から渡る）。300 ルールを抽出し、
24 計画（画面ごとに `base_path` を指定）で **1,200 組**のデータを派生。
各画面 5 件（ルール 4 + 画面確認 1）の **120 件**を承認して実行した。

| 項目 | 実測 |
|---|---|
| 画面 / ルール | 24 / 300（`unparsed=0`） |
| データ派生 | **1,200 組**（受入 900 組以上） |
| 承認ケース | 120（DRAFT 1,128 は未承認のまま） |
| 100 ケース / 4 並列 | **12.32 秒**（< 60 分） |
| 120 ケース / 4 並列 | 14.81 秒、失敗 0 |
| 初回合格率 | **100%** |
| 正解集（`samples/phase6/golden.json`）一致 | **100%**、不一致 0 |
| 被覆 | 画面 **24 / 24**、回帰 120 / 120、影響 24 / 48、ルール 2 / 14 |

ルール被覆が 2 / 14 なのは、1 計画あたりの草稿上限（200、実測は 50）と
**プロジェクト全体のルール順**で派生するため、各計画の先頭 4 件が同じ
2 ルールに集中するからである（分母 14 = 草稿が触れたルール）。
300 ルール全体の派生はエンジン単体の golden（`golden_phase6.rs`、
900 組以上のデータと DSL 合格比 1.0）で検証しており、**ルール単位の派生スコープは
未実装の制約**として §3 に残す。

### 2.4 §13 失敗モード（`phase6-failure-eval.py`）

| シナリオ | 実測 |
|---|---|
| `storage_state` 期限切れ | run を作らず `env_abort.reason=storage_state_expired`（age 32,400 秒 > 28,800 秒） |
| `storage_state` 欠落 | run を作らず `storage_state_missing` |
| 先頭 3 件全滅（接続不能） | 3 件を **ENV** に再分類して停止、残り 4 件は `SKIPPED`、run は `DONE`、却下 0 |
| DATA 失敗 | 失敗 1 件で reset を 1 回実行（`reset=ok`）して停止、残りは `SKIPPED` |
| run 開始時 reset / seed | `data_preparation.reset=ok` / `seed=ok`、全件合格 |
| 成果物 0 件 | 書込不能ディレクトリで失敗させ、`UPLOAD_FAILED` を記録（合否は FAILED のまま）し run に警告。API の `artifacts` にも残る |
| flaky | 初回のみ遅延させ、`TIMEOUT` → attempt=2 `PASSED` を **3 件**集計。ENV 中断は誤発火しない |
| 環境ロック | 2 本目の Runner が **6,038 ms** 待機して起動、両 run `DONE` |

### 2.5 成果物の保持と容量上限（`phase6-retention-eval.py`）

評価では上限を縮小（trace 2 KB / run 4 KB）して機構を実測し、既定値は
trace 20 MB / run 20 GB / 保持 90 日（trace 30 日）。

| シナリオ | 実測 |
|---|---|
| trace 上限 | 2,048 バイトへ切り詰め、`status=TRUNCATED` を記録 |
| run 合計上限 | 4,096 バイト超過後の失敗は trace を保存しない（`trace_disabled=true`、3 点 + HAR） |
| 保持期限 | 100 日前の画像と 40 日前の trace を削除し `EXPIRED` / `bytes=0`、新しい画像は残す（`expired=2` / `deleted_files=2`） |
| 冪等性 | 2 回目の適用は `expired=0` |
| run 単位削除 | `artifacts:cleanup` で残り 1 件を削除し、監査 `test.artifacts.cleanup` を記録 |
| worker 日次 | 起動時に保持を適用し `test artifacts retention applied` を出力、`EXPIRED` を確認 |

### 2.6 `VERIFIES` の outbox → Neo4j 投影（`phase6-graph-eval.py`）

| 項目 | PG | Neo4j |
|---|---|---|
| Requirement | 3 | 3（`natural_key=rule:<uuid>`） |
| TestCase | 13 | 13 |
| VERIFIES 辺 | 12 | 12（`resolution=INFERRED`） |
| outbox イベント | 28（dead letter 0） | — |

`derive` と同じ transaction で `EntityUpserted` / `RelationshipUpserted` を積み、
worker の projector が MERGE する。Neo4j の件数・解決状態は PG と一致した。
被覆計算は PG を読み、Neo4j の有無に依存しない（`GRAPH_VERIFIES_UNAVAILABLE` の縮退は維持）。

### 2.7 Runner 単体 CI

`workers/runner/runner.test.mjs`（`node --test`、8 件）を追加し、
`.github/workflows/ci.yml` の `runner` ジョブで実行する。
分類（SELECTOR / TIMEOUT / NETWORK / DATA）、`buildUrl` のクエリ、
ロケータ優先順位、HAR 伏せ字、成果物記録、run 予算の trace 無効化を検査する。
ブラウザ実行の E2E は API / DB / Chrome を要するため CI には載せず、
本記録の受入スクリプトで再現する。

### 2.8 UI（`npm run check:ui:phase6`）

```json
{"rules":6,"cases":25,"runs":2,"executions":10,"defects":4,"coverageCards":5,
 "artifactUi":{"cleanupButtons":2,"expiredChips":4,"truncatedChips":1},
 "columns":1,"errors":[]}
```

- 被覆 5 指標、`runner_external_process`、`REQUIREMENT_INFERRED_FROM_RULE` を表示。
- 成果物チップ `EXPIRED` / `TRUNCATED` と run 単位の削除ボタン、保持期限の適用ボタン。
- 1440px / 607px。607px で被覆グリッドが 1 列へ縦積み。
- スクリーンショット: `/tmp/systivra-phase6-tests-1440.png`、
  `/tmp/systivra-phase6-tests-607.png`。

### 2.9 ゲート

| ゲート | 結果 |
|---|---|
| `cargo fmt --all --check` | 緑 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 緑 |
| `cargo test --workspace` | **233 passed**、0 failed |
| `cargo run -p api --bin openapi -- --check` | `openapi.json is up to date` |
| `npm run check:i18n` / `npm run build` | 569 keys / build 成功 |
| `scripts/verify-db.sh` | 001〜027 全項目通過（拡張は plpgsql / vector / pg_trgm のみ） |
| Runner 単体（`node --test`） | 8 passed |

---

## 3. 未実施・降格

| 項目 | 状態 | 扱い |
|---|---|---|
| Collector の分離 / 16 ワーカー複数ノード | **未実装** | Runner は単一プロセス、context 4 並列。ブラウザ障害時の再配置は未検証。成果物はローカル FS（S3 ではない） |
| 実案件規模の性能（90 秒/ケース、7,000 ケース全回帰、32 並列） | **未検証** | 合成静的 SAMPLE は 120〜125 ms/ケース。実顧客の画面・待機・帳票を含む 90 秒/ケースの裏付けにはならない |
| ルール単位の派生スコープ | **未実装** | `derive` はプロジェクト全体のルールを順に使い、草稿上限 200 で切る。24 画面実測のルール被覆が 2/14 なのはこのため。300 ルール全体はエンジン golden で検証 |
| `NEEDS_CLARIFICATION` / 自然言語 seed | **未実装** | Phase 5 側の未完了を引継ぎ |
| Q-01 の 5 セクション SSE / Markdown / PDF | **未実装** | Phase 5 側の未完了を引継ぎ |
| 20 変更の recall / precision | **未実施** | 人手正解集がまだ無い |
| 2.5M Fact / 6 ホップ性能 | **未実施** | Phase 4 の専用 DB 実測に委譲 |
| テスト資産照合 | **未実装** | `tests_not_available` を明示 |
| 30 分目標（`detail/03`） | **未検証** | 失敗 0・P95 2 件以下の条件で 30 分に入るかは実案件でしか分からない |

---

## 4. 既知の制約

- `core.project_of` は `CREATE OR REPLACE` で関数全体を置き換える。015 は 014 の
  10 分支をすべて保持し、test の 5 分支を追加した。`verify-db.sh` に回帰検査がある。
- 設計正本の `detail/02` では子表に `project_id` が無いが、実装では全表に付与して
  RLS を敷いた。スコープ設定漏れを「0 件」にするためである。
- `test_run.status` に `READY` を追加した。実行前の状態を `DONE` と偽らないため。
- `test_execution` に `execution_source` と `PENDING` を追加した。取込み元を隠さないため。
- 環境ロックは `pg_try_advisory_lock` ではなく、部分一意索引による永続化した
  「活性 run は 1 つ」制約で表現した。セッションを跨ぐロックを保持できないため。
- 被覆の分母は登録済みテスト資産（`covers_*` の和集合）から作る。外部の
  要件台帳や図の母集合ではない。分母が変われば率も変わるので分子分母を残す。
- `INFERRED` / `UNRESOLVED` を `RESOLVED` に格上げしない。ルールの競合は
  両方を残し、片方を勝たせない。
- 草稿は `status=DRAFT` 固定。承認は `test:approve` を持つ人間だけ。
  `derive` は `test_run` を作らず、Q-01 を見ただけで実行が始まることはない。
- Requirement 被覆の Requirement は、現時点では検証ルールから推定した実体である
  （`natural_key = rule:<uuid>`、resolution は規則の解決状態を引き継ぐ）。
  実要件の抽出に置き換えるときは投影元だけを差し替え、`VERIFIES` の読み方は変えない。
- `derive` は PG の `graph_meta` を正本とし、同じ transaction で outbox に
  `EntityUpserted` / `RelationshipUpserted` を積む。worker の projector が
  Neo4j へ MERGE する。被覆計算は PG を読むため、Neo4j 未接続でも完結する。
- Runner の `ENV` 中断は、実行済みケースの結果だけを残し、残りを `SKIPPED` として
  取り込んで run を閉じる（run 24 時間上限と同じ扱い）。run 状態は `CANCELLED`
  ではなく `DONE` になる。中断理由は Runner の報告（`env_abort` / `data_abort` /
  `run_timeout`）に残る。
- 保持期限はファイルの mtime で判定する。オブジェクトストレージの Last-Modified に
  相当する値が無いため、コピーすると期限が延びる。日次適用は worker の起動時 +
  24 時間間隔で、cron / systemd timer からの外部起動はしていない。
- 成果物の実体は `data/` 配下のローカル FS。`ARTIFACT_ROOT` で基点を変えられるが、
  S3 / 署名付き URL は未実装。期限切れでも実体を消すまで `artifacts` は残り、
  `EXPIRED` として表示する。

---

## 5. 再現

```bash
# 空 DB に 001〜027 と roles を適用（全件を順に適用し RLS / 権限を検査する）
PGHOST=localhost PGUSER=<admin> ./scripts/verify-db.sh

createdb systivra_phase6_verify
for f in migrations/0*.sql; do psql -d systivra_phase6_verify -v ON_ERROR_STOP=1 -f "$f"; done
psql -d systivra_phase6_verify -f deployment/postgres/roles.sql
PGDATABASE=systivra_phase6_verify ./scripts/seed-dev.sh

# API（18088）と worker を起動
PORT=18088 DATABASE_URL=postgres://systivra_app@localhost:5432/systivra_phase6_verify \
  SYSTIVRA_DATA_DIR=$PWD/data NEO4J_DISABLED=1 SESSION_SECRET=<32 bytes> \
  SESSION_COOKIE_SECURE=false ./target/debug/systivra-api
DATABASE_URL=postgres://systivra_scheduler@localhost:5432/systivra_phase6_verify \
  WORKER_TICK_SEC=1 NEO4J_DISABLED=1 ./target/debug/systivra-worker

export PGHOST=localhost PGUSER=<admin> PGDATABASE=systivra_phase6_verify \
  SYSTIVRA_BASE_URL=http://127.0.0.1:18088

# Phase 6 の実測（それぞれ独立に DB を初期化する）
python3 scripts/phase6-test-eval.py
python3 scripts/phase6-runner-eval.py      # 100 ケース / 4 並列 / 成果物 / HAR
python3 scripts/phase6-failure-eval.py     # §13 / storage_state / seed / reset / ロック
python3 scripts/phase6-retention-eval.py   # trace 上限 / 保持期限 / 削除 / worker 日次
python3 scripts/phase6-sample-eval.py      # 24 画面 / 300 ルール / 正解集
python3 scripts/phase6-graph-eval.py       # VERIFIES の outbox → Neo4j（Neo4j 必需）

cargo test -p engine-test --test golden_phase6
cd workers/runner && node --test

cd apps/web && VITE_API_PROXY=http://127.0.0.1:18088 npx vite --port 15200
cd apps/web && WEB_BASE=http://localhost:15200 npm run check:ui:phase6
```

UI 検査は `phase6-retention-eval.py` の後に実行する（`EXPIRED` / `TRUNCATED` の
成果物が必要）。`phase6-sample-eval.py` は `samples/sample-web/sample/s/` と
`samples/phase6/golden.json` を再生成する。

---

## 6. 次ゲート

Phase 6 の残りは環境・顧客データ依存。

1. 実案件の画面で 90 秒/ケースの裏付けを取り、30 分目標（`detail/03`）を再判定する。
2. Collector を分離し、16 ワーカー / 複数ノードでの再配置と 7,000 ケース全回帰を実測する。
3. ルール単位の派生スコープ（計画ごとの対象ルール指定）を実装し、24 画面実測の
   ルール被覆を 300/300 に近づける。
4. 成果物をオブジェクトストレージへ移し、保持期限を Last-Modified で判定する。
