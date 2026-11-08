# Phase 4 進捗・受入記録（Code Analysis 縦断実装）

実施日: 2026-11-07  
対象: `08-実装計画.md` Phase 4 / `detail/06-code-analysis-engine.md`  
環境: macOS arm64、PostgreSQL 18.4、Rust 1.99.1、Node.js 26.0.0、Neo4j 2026.05 Community

> 判定: **Phase 4 の今回指定受入範囲は PASS。**
> source-only の縦断実装、12 ファイル E2E、Git の clone/fetch/branch/SHA 同期、
> 13,000 Java の解析・増分、250 万 Fact の COPY / supersede / Neo4j 再投影を実測した。
> SVN は未導入、OpenRewrite / 顧客 classpath と「解析による 250 万 Fact 生成」は
> 外部依存または未接続であり、本受入の PASS には含めない。顧客案件の意味解析品質を
> 証明するものではない。

---

## 1. 実装済み範囲

| 領域 | 実装 |
|---|---|
| リポジトリ | `DIR` / `SNAPSHOT` / `GIT` を同期。Git はローカル / 閉域 bare を clone/fetch し、branch / tag / SHA を commit SHA に解決。版ごとの snapshot、tree digest、file hash |
| 文字コード | UTF-8 / Shift_JIS / Windows-31J / EUC-JP の判定、復号、元バイト偏移、BOM |
| Java | Tree-sitter AST から package / import / class / interface / enum / method / field、継承、実装、呼び出し |
| JSP | 等長プレースホルダ、scriptlet 合成、HTML 構造、form action。原文件の行・バイト位置 |
| XML | Struts action / forward、Dicon component、MyBatis namespace と SQL |
| SQL | 基本の `FROM / JOIN / UPDATE / INTO` 表参照。Java 文字列リテラルと `.sql` |
| Fact | `RESOLVED / INFERRED / PARTIAL / UNRESOLVED`、候補、未解決理由、Fact evidence |
| 永続化 | `code` schema、RLS、Fact / evidence の 5,000 行 COPY、`supersede_then_insert`、同一 tx |
| 増分 | 親版との path + content hash 差分。変更ファイルだけ Fact を再生成し、未変更の Symbol / Fact をバッチ再利用・再投影 |
| グラフ | Fact から PG entity / relationship と outbox を生成。outbox と全件 rebuild は `UNWIND` 1,000 件バッチ、Neo4j は有効で実測 |
| API | repository、revision sync、analysis run、symbols、facts、source content |
| UI | リポジトリ同期、解析状態、未解析・降格理由、Symbol、Fact、元バイト位置と quote |

---

## 2. 受入結果

| # | 受入 | 結果 | 証拠 |
|---|---|---|---|
| 1 | SAMPLE のクラス / メソッド認識率 ≥ 98% | **PASS（source-only 合成 SAMPLE）** | `golden_phase4.rs`: 41 class / 250 method、期待 Symbol recall 100% |
| 2 | `RESOLVED` 呼び出し precision ≥ 98% | **PASS（プロジェクト内の一意な型・メソッド・引数数）** | 200 / 200 が正解。誤解決 0、recall 100% |
| 3 | 反射 / 動的プロキシ 30 件が `RESOLVED` でない | **PASS** | 30 / 30 が `UNRESOLVED`、`RESOLVED` 0 |
| 4 | 1 ファイル変更の再解析が 10% 未満 | **PASS** | API E2E: 12 ファイル中 1 ファイル = 8.33%。未変更 78 Fact / 41 Symbol を再利用 |
| 5 | classpath なしの降格が帳票に出る | **PASS** | `classpath_complete=false`、UI に 2 件の降格理由 |
| 6 | Fact から元ファイルのバイト位置へ戻れる | **PASS** | 84 / 84 evidence の byte range を source API で再読し、quote と一致 |
| 7 | 「解析不足」が画面で隠れない | **PASS** | 未解決 33、失敗 0、classpath 欠落、OpenRewrite 未接続を常時表示 |
| 8 | Git の同期・SHA・失敗処理 | **PASS** | `phase4-git-eval.py` 18/18。path / `file://`、clone/fetch、branch/tag/SHA、tree digest、parent、増分、失敗終端 |
| 9 | 13,000 Java を解析できる | **PASS（合成）** | 同期 4.06s、解析 21.79s（596.65 files/s）、Symbol 91,000 / Fact 104,000 |
| 10 | 大規模でも増分が 10% 未満 | **PASS** | 13,000 中 1 ファイル = 1 / 13,000（0.0077%）。増分解析 203.73s |
| 11 | 250 万 Fact を COPY 永続化できる | **PASS** | 17.498s（142,870.07 rows/s）、5,000 行 × 500 文、live 2,500,000 |
| 12 | supersede 増分が live 一意性を保つ | **PASS** | 25,000 件を 2.014s。live 2,500,000 / historical 25,000 / total 2,525,000 |
| 13 | 250 万 Fact 相当のグラフを Neo4j 再投影できる | **PASS** | 5,506 entity / 2,500,005 relationship、Neo4j 173.117s、PG と件数一致 |

### 2.1 Golden source SAMPLE

`crates/engine-code/tests/golden_phase4.rs` は次を毎回検査する。

- 41 classes / 250 methods。
- プロジェクト内で一意に解決できる 200 CALLS の precision / recall。
- 30 反射呼び出しの `RESOLVED` が 0。
- 41 ファイル中 1 ファイルを選んだ増分解析が 10% 未満。
- 未変更 Java 型を解決文脈として保持し、変更ファイルの呼び出し精度を落とさない。

この 98% は**型解決 worker のない source-only SAMPLE**の値である。13,000 Java の顧客案件や
外部 library の signature precision を証明しない。

### 2.2 API + worker E2E

専用 fixture の 12 ファイル SAMPLE を API + worker で実行。

初回:

```json
{"files_analyzed":12,"files_skipped":0,"symbols":45,"facts":84,
 "resolved":51,"unresolved_or_partial":33,"classpath_complete":false}
```

1 Java ファイル変更後:

```json
{"files_analyzed":1,"files_skipped":11,"copied_symbols":41,"copied_facts":78,
 "new_symbols":4,"new_facts":6,"symbols":45,"facts":84,
 "resolved":51,"unresolved_or_partial":33,"incremental_fraction":0.08333333333333333}
```

その他:

- 30 reflection CALLS（`Class.forName` 15 / `Proxy.newProxyInstance` 15）/ `RESOLVED` 0。
- 84 evidence すべてで `byte_end > byte_start`。
- 84 evidence を `/api/v1/code/files/{id}/content` の元バイト範囲と照合し、84 / 84 一致。
- Shift_JIS の `LegacyNameService` を class symbol として認識。
- Struts `HANDLES` と MyBatis `QUERIES` を確認。
- 未変更 78 Fact / 41 Symbol は新 revision へバッチコピーし、変更ファイルの
  6 Fact / 4 Symbol だけを追加生成した。コピー Fact も current revision の
  graph relationship と outbox を再生成する。PG / Neo4j の `r1 -> r2` は
  relationship 84 / 84 で一致した。
- 再現: `scripts/phase4-eval.py`、ローカル結果 `data/phase4-eval/report.json`。

### 2.3 UI

`npm run check:ui:phase4`:

```json
{"metrics":5,"revisionRows":4,"symbolRows":45,"factRows":84,
 "degraded":2,"unresolved":33,"quoteLength":4,"gitKind":true,"errors":[]}
```

- 1440px: 解析状態、品質警告、Symbol 表、Fact 表、evidence 詳細を表示。
- 607px: `code-layout` が 1 列へ縦積み。
- 登録済みリポジトリの選択肢と選択中 chip の双方で `GIT` kind を表示する。
- スクリーンショット: `/tmp/systivra-phase4-code-1440.png`、
  `/tmp/systivra-phase4-code-607.png`。

### 2.4 Git 同期 E2E

`scripts/phase4-git-eval.py`（API + worker、ローカル bare リポジトリ）:

```json
{"first_revision":"1b1bb72f...","second_revision":"144a9767...",
 "symbols":4,"facts":4,"incremental_files_analyzed":1,"passed":18,"failed":0}
```

- `kind=GIT` で登録し、`revision=main` を clone/fetch → commit SHA に解決。
- clone 後に bare へ追加した tag も fetch で取得し、tag を最初の commit SHA に解決する。
- 2 回目の `main` 同期は新しい SHA の revision を作り、parent が前の版を指す。
- 1 ファイル変更後の解析は `files_analyzed=1`。
- 同じ commit SHA を直接同期しても revision / tree digest を重複作成しない。
- 存在しない branch は `CODE_SYNC_FAILED` で `FAILED` 終端し、エラーをジョブに残し、
  revision を追加しない。
- remote URL は `GIT_ALLOW_REMOTE=1` のときだけ許可。URL に資格情報は含めない。
- ローカル bare パスと `file://` URL は E2E 済み。HTTP(S) / SSH の実 remote は
  閉域サーバーが無いため未実測。
- **SVN は未導入**。`kind=SVN` は HTTP 409 `CAPABILITY_UNSUPPORTED` で明示的に拒否し、
  UI にも未対応を表示する。

### 2.5 大規模性能

`scripts/phase4-scale-eval.py`（13,000 Java、DIR）:

```json
{"files":13000,"file_count":13000,"generate_seconds":3.77,
 "sync_seconds":4.06,"analysis_seconds":21.79,
 "analysis_files_per_second":596.65,"symbols":91000,"facts":104000,
 "incremental_sync_seconds":4.57,"incremental_analysis_seconds":203.73,
 "incremental_files_analyzed":1,"incremental_ratio":0.000077,"status":"SUCCEEDED"}
```

`scripts/run-phase4-fact-scale.sh` は専用一時 DB と API を起動し、
`scripts/phase4-fact-scale-eval.py` を実行して DB を破棄する。実測:

```json
{"fact_count_persisted":2500000,"copy_batch_rows":5000,
 "copy_seconds":17.498,"copy_rows_per_second":142870.07,"copy_statements":500,
 "supersede_requested":25000,"supersede_seconds":2.014,
 "live_facts":2500000,"historical_facts":25000,
 "superseded_by_links":25000,"total_fact_rows":2525000,
 "pg_graph_build_seconds":107.954,
 "pg_graph_entities":5506,"pg_graph_relationships":2500005,
 "neo4j_reprojection_seconds":173.117,
 "neo4j_entities":5506,"neo4j_relationships":2500005,"passed":true}
```

- 13,000 Java の初回解析は 60 分上限に対し 21.79 秒。増分は 1 / 13,000 で、
  型文脈を作るため全ファイルを読み直すが Fact / Symbol の再生成は 1 ファイルだけ。
- Fact / evidence は application の解析経路でも 5,000 行 COPY。25,000 件の
  supersede 後も live は 2,500,000、`superseded_by` 25,000 件、総行 2,525,000。
- PG の 5,506 node / 2,500,005 live edge を、production の `/graph/rebuild` API が
  keyset 走査と `UNWIND` 1,000 件で Neo4j に再投影した。PG / Neo4j / API 応答の件数は一致。
- outbox drain も 5,000 件取得・最大 1,000 件 `UNWIND` に変更し、1 件ずつ投影しない。
- 再現: `data/phase4-scale/report.json`、`data/phase4-scale/fact-report.json`。
  専用 DB はスクリプト終了時に削除する。

### 2.6 最終ゲート

| ゲート | 結果 |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS |
| `cargo test --workspace` | **213 passed / 0 failed** |
| OpenAPI 生成物一致 | PASS |
| i18n | **544 keys**、3 言語一致 |
| frontend `tsc -b && vite build` | PASS（chunk size warning のみ） |
| `scripts/verify-db.sh` | 001〜024、RLS / 権限 / graph / code / supersede 全通過 |
| Phase 4 API E2E | 12 files、1 file 増分、84 / 84 evidence |
| Git E2E | **18 / 18** |
| UI `check:ui:phase4` | 1440 / 607px、HTTP / console error 0 |

---

## 3. 未実施・降格

| 項目 | 状態 | 理由 / 次の扱い |
|---|---|---|
| Java 21 + OpenRewrite LST / classpath 型解決 | **外部依存・未接続** | source-only の Tree-sitter でプロジェクト内型だけ解決。顧客 classpath と OpenRewrite 実行環境は本リポジトリ外 |
| 旧 Java 意味解析 worker | **不採用（ADR-017）** | 独立 Java プロセスと classpath 型解決は最終設計に含めない |
| classpath_index の jar 構築 | **未実装** | 表だけで、Maven/Gradle 依存解決は無い |
| SVN clone・branch / tag | **未導入** | `svn` を前提にせず API は unsupported。Git はローカル bare / `file://` で E2E 済み |
| SAStruts / Seasar annotation 全般 | **一部** | Dicon class/name は実装。規約 URL、property 注入の完全解決は未実装 |
| TLD とカスタム Tag | **一部** | JSP HTML は解析。TLD 有無による `RESOLVED / UNRESOLVED` は未実装 |
| JSqlParser | **未使用** | 基本 token 走査。複雑な SQL、CTE、DB 方言は未検証 |
| 解析による 250 万 Fact 生成 | **外部依存・未実施** | 250 万 Fact 相当の永続化・graph 再投影は受入済み。13,000 Java の解析は 104,000 Fact |
| 1,000+ JSP の実規模解析 | **未実施** | 13,000 Java は合成、JSP は 12 ファイル SAMPLE と単体 fixture |
| 増分 Fact の自動再投影 | **PASS（小規模 E2E）** | 未変更 Fact の新 revision graph/outbox を生成。2.5M は全件 rebuild API で投影 |
| JSP 200 行・全 W-31J fixture | **未実施** | 基本 scriptlet と Shift_JIS Java は検証。大規模 JSP は未検証 |
| UI の Source 行ハイライト | **未実装** | quote と byte range を表示。原文を該当行へスクロールする viewer は未接続 |
| 増分の逆依存 1 層 | **未実装** | 変更ファイルだけを再解析。service signature 変更から呼び出し元へ広げる処理は無い |
| JSP scriptlet の CALLS Fact | **未実装** | scriptlet は構文検査まで。HTML form は Fact 化するが Java 呼び出しは未生成 |

---

## 4. 既知の制約

- `code.fact` の `superseded_by` は遅延 FK。同一 tx で旧行を supersede して新行を
  INSERT する順序を守る。`scripts/verify-db.sh` が live Fact 1 行を検査する。
- `files_skipped` は「Fact を再生成しないファイル」である。Java 型解決文脈としては
  読み込んでおり、ファイル IO が 0 という意味ではない。
- source-only の `RESOLVED` は「同一 snapshot 内で型・メソッド名・引数数が一意」の意味であり、
  classpath を含む JVM の完全な overload 解決ではない。
- 未変更ファイルから再利用した Fact は、親版の evidence を共有する。ファイル hash が同じなので
  byte 位置は有効だが、evidence locator の revision は親版のままである。
- quote は NFC で保存する。元ファイルが NFC でない場合、位置は保持できても
  quote の完全一致を別途検証する必要がある。
- `DIR` / `SNAPSHOT` とローカル Git は `SYSTIVRA_DATA_DIR` 配下に限定する。
  `GIT_ALLOW_REMOTE=1` のときだけ許可する HTTP(S) / SSH / `git://` は、URL に
  資格情報を埋め込まず Git credential helper に委ねる。
- 一時検証 DB は完了後に破棄する。永続する証拠は本記録、golden fixture、
  `data/phase4-*/report.json`、スクリーンショット。

---

## 5. 再現

```bash
./scripts/seed-phase4.sh
PGHOST=localhost PGUSER=sainm ./scripts/verify-db.sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p api --bin openapi -- --check

# API / worker を起動後
SYSTIVRA_PROJECT_ID=22222222-2222-4222-8222-222222222222 python3 scripts/phase4-eval.py
SYSTIVRA_PROJECT_ID=22222222-2222-4222-8222-222222222222 python3 scripts/phase4-git-eval.py
PHASE4_SCALE_FILES=13000 python3 scripts/phase4-scale-eval.py
cd apps/web && npm run check:i18n && npm run gen:api && npm run build
cd apps/web && npm run check:ui:phase4

# 専用一時 DB + API + 2.5M Fact / Neo4j 再投影
./scripts/run-phase4-fact-scale.sh
```

各評価は `SYSTIVRA_BASE_URL`、`SYSTIVRA_LOGIN_ID`、`SYSTIVRA_PASSWORD` を共通で使う。
`phase4-eval.py` は `PHASE4_TAG`、性能評価は `PHASE4_SCALE_FILES` / `PHASE4_SCALE_TAG`、
Fact 評価は `PHASE4_FACT_COUNT` / `PHASE4_SUPERSEDE_COUNT` で変更できる。

---

## 6. 受入境界

今回の受入で閉じた範囲:

1. Git のローカル / `file://` clone・fetch、branch / tag / commit SHA、tree digest、
   scan / diff / analysis 接続、同期失敗の終端化、UI kind 表示。
2. 13,000 Java 相当の解析時間、Symbol / Fact 件数、1 ファイル増分率。
3. 250 万 Fact 相当の COPY、supersede、PG graph 生成、Neo4j 全件再投影。

明示的な外部依存 / 未導入:

1. OpenRewrite LST、Java 21 semantic worker、顧客 classpath の取得と再現。
2. 解析そのもので 250 万 Fact を生成する顧客案件。本記録の性能値は
   Fact-equivalent synthetic data と synthetic source に限定する。
3. SVN。`svn` を導入していないため `kind=SVN` は HTTP 409 `CAPABILITY_UNSUPPORTED` とする。
4. HTTP(S) / SSH Git remote の実閉域サーバー E2E。ローカル bare / `file://` は実測済み。
5. 顧客案件の 24 画面・難読 JSP・外部 library を含む意味解釈。source-only の
   `RESOLVED` はプロジェクト内で一意な型・メソッド・引数数の範囲に限定する。
