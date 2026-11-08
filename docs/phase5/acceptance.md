# Phase 5 進捗・受入記録（Evidence-Bounded Impact Draft）

実施日: 2026-11-07  
対象: `08-実装計画.md` Phase 5 の ENTITY / DIFF 縦断切片  
環境: macOS arm64、PostgreSQL 18.4、Rust 1.99.1、Node.js 26.0.0、Neo4j 2026.05 Community

> 判定: **Phase 5 の今回指定受入範囲は PASS。**
> ENTITY / DIFF は、既存の PG コード Fact・文書 evidence・グラフから
> 決定論的な `ImpactAssessmentDraft` を生成する。Git unified diff の行シード、
> Field の読み書き閉包、frontier 限定 BFS、20 変更 golden set を実測した。
> 2026-11-07 の追補で **Q-01 変更影響問合せ**（`detail/08`）を実装した。
> 自然言語は決定的な部分一致でシードを探し、曖昧なら `NEEDS_CLARIFICATION` で止まる。
> 5 セクションは ImpactReport からの投影で、`summary` は件数テンプレート。
> Markdown / PDF、真の SSE（`Last-Event-ID`）、250 万辺の 6-hop 性能も実測済み。
>
> OpenRewrite / `javac` / 顧客 classpath は**取得も実行もせず、今回の受入対象外**とする。
> 「外部境界」は利用可能という意味ではなく、未導入・未検証として PASS から除外する意味である。
> 外部型や動的挙動は既存 Fact の
> `INFERRED / UNRESOLVED` を維持し、静的解析結果を型解決済みと偽装しない。

---

## 0. 受入境界

| 区分 | 取扱い |
|---|---|
| 実行する | Rust の source-only Fact、文書 evidence、PG グラフ、deterministic ImpactEngine |
| 実行しない | OpenRewrite LST、`javac`、独立 Java semantic worker、顧客 JAR / classpath 解決 |
| 正解データ | 合成 SAMPLE の 20 変更。顧客案件固有の意味的正解は未提供 |
| 主張できること | 合成 SAMPLE における recall / precision、経路、evidence、性能 |
| 主張しないこと | 外部 library overload、実行時 DI、動的プロキシの完全解決、顧客案件での意味精度 |

OpenRewrite / `javac` / 顧客 classpath が無い状態を「影響なし」と丸めない。
解決できない型・動的呼び出しは `INFERRED / UNRESOLVED` と `degraded` に残す。

---

## 1. 実装済み範囲

| 領域 | 実装 |
|---|---|
| 入力 | `ENTITY`（1〜100 個の graph entity）と `DIFF`（base/head code revision） |
| 種子解決 | ENTITY は project-scope で検証。DIFF は Git unified diff / 原文行差分から変更行を求め、最小包含 Symbol を解決 |
| 閉包 | `WRITES / READS` Fact により変更 Field から読み書きメソッドへ到達。CALLS の逆方向で呼び出し元へ到達 |
| 走査 | DB frontier を 6 ホップまで取得する双方向 BFS、hub 非展開、item budget、最短経路。全件ロードしない |
| 解決状態 | 経路上の最も弱い `RESOLVED / PARTIAL / INFERRED / UNRESOLVED / CONFLICT` を引き継ぐ |
| 証拠 | コード Fact evidence、文書 chunk / evidence、locator。証拠のない graph node は `unverified` へ分離 |
| 文書 | 影響コードの simple symbol 名を文書 chunk と照合。**名前一致の INFERRED** と明示 |
| 出力 | `ImpactAssessmentDraft`。why、path、path_edges、confidence、risk、truncated、degraded、unverified |
| 安全性 | `approval_state=DRAFT`、`code_modified=false`、`tests_executed=false` |
| API | `POST/GET /projects/{pid}/impacts`、`GET /impacts/{iid}`、Markdown / PDF |
| worker | `IMPACT_ANALYSIS` job。失敗時は Draft を FAILED へ更新 |
| UI | ENTITY / DIFF 選択、unified diff 入力、変更行、Draft、分類別 item、evidence、降格・打切り・未検証、PDF 表示 |

---

## 2. 受入結果

| # | 受入 | 結果 | 証拠 |
|---|---|---|---|
| 1 | ENTITY から影響を算出 | **PASS** | UserEditAction Action から 4 items / evidence 4 |
| 2 | DIFF から seed を解決 | **PASS** | 1 changed file → Module + Class + 2 Method の 4 seeds |
| 3 | 各主表 item に why / resolution / evidence | **PASS** | DIFF 18 / 18 items に evidence、why 空 0 |
| 4 | 文書 evidence とコード影響を統合 | **PASS** | UserService 呼び出し仕様が 1 document item として到達 |
| 5 | 証拠なしを黙って主表へ入れない | **PASS** | unverified は別配列。今回 0 件 |
| 6 | 同一入力で同一 Draft | **PASS** | 同じ ENTITY request が同じ impact_id、report JSON 完全一致 |
| 7 | 降格・打切りを隠さない | **PASS** | `tests_not_available`、hubs / depth / budget を API / UI に表示 |
| 8 | コード変更・テスト実行をしない | **PASS** | `code_modified=false`、`tests_executed=false` |
| 9 | Git unified diff から line-level seed | **PASS** | `UserService.save` の new line 11 を Method seed に解決 |
| 10 | Field 読み書き閉包 | **PASS** | `UserEditAction.userService` → `READS` → `UserEditAction.update()` |
| 11 | 20 変更 Golden set | **PASS** | recall **1.000000**、precision **0.833333**、証拠欠落 0 |
| 12 | 全影響項目に経路・why・quote・locator | **PASS** | Golden 20 変更 / 66 actual 項目を検査 |
| 13 | 真の SSE と再開 | **PASS** | `text/event-stream`、`partial` 5 件 + `done`、`Last-Event-ID: 3` → code/tests/done |
| 14 | summary 受入 8 秒 | **PASS** | ENTITY 0.018s、自然言語 0.086s、Q-01 DIFF 0.012s |
| 15 | Markdown / PDF | **PASS** | PDF は日本語テキスト抽出、Impact 117,073B / Q-01 133,554B |
| 16 | 250 万辺の 6-hop | **PASS** | **1.070 秒**、6 items、depth 0〜5、frontier 限定 |
| 17 | 逆依存探索 | **PASS** | `UserService.save` seed から呼び出し元 `UserEditAction.update` に到達 |

### 2.1 API + worker E2E

Phase 4 の 12 ファイル SAMPLE と文書 chunk で実行。

ENTITY:

```json
{"entity_items":4,"entity_evidence":4,
 "entity_degraded":["tests_not_available"],
 "approval_state":"DRAFT","code_modified":false,"tests_executed":false}
```

DIFF（Git 形式の変更行を含む）:

```json
{"diff_items":21,"diff_evidence":21,"document_items":1,
 "changed_hunks":[{"path":"src/main/java/jp/example/service/UserService.java",
                   "new_start":11,"new_end":11,"old_start":11,"old_end":11}],
 "diff_unverified":0,"risk":{"score":0.295,"level":"LOW"},
 "truncated":{"hubs":0,"depth":false,"budget":false,"no_logical_change":false}}
```

DIFF seeds:

- `jp.example.service.UserService.save(UserDto)`（Method）
- `src/main/java/jp/example/service/UserService.java`（Module）

変更 Field の統合テストでは、`UserEditAction.userService` の line 6 を Field seed とし、
`READS` 関係で `UserEditAction.update()` に到達した。

実測結果: `data/phase5-eval/report.json`。

### 2.2 UI

`npm run check:ui:phase5`:

```json
{"entityItems":5,"entityEvidence":4,"diffItems":21,"diffEvidence":21,
 "changedHunks":1,"pdfLinks":1,"seedColumns":1,"errors":[]}
```

- ENTITY は default seed の監査済み 5 項目、DIFF は 21 / 21 項目に evidence。
- 607px で seed 入力が 1 列へ縦積み。
- unified diff 入力、変更行、DRAFT、risk factors、path、evidence quote / locator、
  Markdown / PDF ダウンロードを表示。
- スクリーンショット: `/tmp/systivra-phase5-impact-1440.png`、
  `/tmp/systivra-phase5-impact-607.png`。

### 2.3 20変更 Golden set

`tests/fixtures/impact/golden-phase5.json` に、Entity 20 変更、期待影響、
各期待の意味的理由を固定した。`scripts/phase5-golden-eval.py` の実測:

```json
{"changes":20,"true_positive":55,"expected_total":55,"actual_total":66,
 "recall":1.0,"precision":0.833333,"evidence_failures":[],"passed":true}
```

全 main item について `path`、`why`、`quote`、`locator` を検査し、欠落 0。
反射呼び出しは対象クラスを `RESOLVED` にせず、未解決候補として現れることを許容する。

### 2.4 大規模グラフ性能

Phase 4 の専用一時 DB を使い、2,500,005 relationships の中に低次数 6-node chain を
埋め込んだ。impact worker は seed から frontier だけを DB 検索する。

```json
{"facts":2500000,"relationships":2500005,
 "impact_six_hop":{"seconds":1.07,"items":6,"max_depth":5,"passed":true}}
```

全 `graph_meta.entity / relationship` のメモリロードは行わない。背景の高次数 node は
hub として展開を打ち切り、性能 chain は depth 0〜5 の 6 層を全件 evidence 付きで返した。
再現: `./scripts/run-phase4-fact-scale.sh`、
結果 `data/phase4-scale/fact-report.json`。

---

## 2.5 Q-01 変更影響問合せ（2026-11-07 追補）

最小 fixture（グラフ実体 4 件 + CALLS 1 本 + code fact + evidence）と
実 SAMPLE の Git diff で実測。

```json
{"entity":{"status":"SUCCEEDED",
  "headline":"メソッド変更です。画面 0 件、設計書 1 箇所、コード 21 箇所、テスト 40 件に波及します。",
  "counts":{"screens":0,"documents":1,"code":21,"tests":40,"other":0},
  "code_first":{"label":"UserEditAction","why":"変更のシードとして直接指定された",
                "quote":"class UserEditAction {}"},
  "existing_tests":20,"draft_tests":20,"summary_seconds":0.018},
 "events":{"content_type":"text/event-stream","ids":[1,2,3,4,5,6],
           "sections":["summary","screens","documents","code","tests"],
           "resume_from_3":["partial:code","partial:tests","done"]},
 "natural_language":{"status":"SUCCEEDED","summary_seconds":0.086},
 "needs_clarification":{"status":"NEEDS_CLARIFICATION","candidates":2},
 "not_found_seed":"NOT_FOUND_SEED",
 "idempotent":true,
 "diff":{"status":"SUCCEEDED","summary_seconds":0.012,"code_items":20},
 "pdf":{"inquiry_bytes":133554,"impact_bytes":117073},
 "derive_plan":{"draft_case_count":20,"test_run_created":false}}
```

- `counts` は各セクションの `total` と一致。`code` の先頭項目に `why` / `quote` /
  `locator` が付く。
- 自然言語は一意なら自動で進み、同名 2 件なら `NEEDS_CLARIFICATION` で候補を返す。
  候補が 0 件なら `NOT_FOUND_SEED`。**推測で 1 件に絞らない。**
- 同じ質問 + 同じ入力は同じ `inquiry_id`（冪等）。`DIFF` は unified diff と
  head revision を受け付け、行 seed から 5 セクションを生成する。
- `/events` は真の `text/event-stream`。id 1〜6 が増加し、5 つの `partial`
  （summary / screens / documents / code / tests）の後に `done` を返す。
  `Last-Event-ID: 3` の再接続は code / tests の partial と done のみを返した。
- 「テスト計画に入れる」で `POST /change-inquiries/{iid}/test-plans:derive` を呼ぶと、
  DRAFT 計画（草稿 20 件）が T-01 に現れる。**`test_run` は作らない。**
- 計画に紐づく `test_run` は、**その計画のケースだけ**を対象にする。
  計画のケースは承認前 DRAFT なので `case_count=0`。全件 run は従来どおり
  全プロジェクトの承認済みケースを対象にする。
- AI 叙述: `POST /change-inquiries/{iid}/narrate`。AI 未設定なら
  `narrative=null` / `hypotheses=[]` + `ai_unavailable`（5 セクションは残る）。
  設定時は `narrative` と、Q-01 の既知 evidence を basis に持つ
  `hypotheses[]` を返す。各仮説は `INFERRED` で、主表・counts・risk は変更しない。
- Impact 叙述は 22 items を変えず、`narrative=null` / `ai_unavailable`。mock LLM では
  `model=mock-1` / `ai_inferred` で保存され、GET で `narrative_source=AI` を確認。
- Markdown 出力: `GET /change-inquiries/{iid}/report.md` と
  `GET /impacts/{iid}/report.md`。`text/markdown` + `Content-Disposition`、
  見出し・表・証拠・降格を含む。AI 叙述がある場合は「## AI の要約（INFERRED）」を
  先頭に置き、数値・証拠は下の表が正であると明記する。画面からはダウンロードボタン。
- PDF 出力: 同じ `report.pdf` endpoint。日本語フォントは `SYSTIVRA_PDF_FONT`
  または既知のローカルフォントを使い、サブセット埋め込みする。PDF から
  「影響するコード」と symbol 名を再抽出して検証した。
- UI（`InquiryPage`）で 5 セクション、候補選択、イベント順、計画作成、
  テスト画面への遷移（`?plan=`）、Markdown / PDF ダウンロードまで通した。
  概要には `approval_state=DRAFT`、`code_modified=false`、`tests_executed=false` を常時表示する。
  1440px / 607px。スクリーンショット: `/tmp/systivra-q01-1440.png`、
  `/tmp/systivra-q01-607.png`。

実測結果: `data/phase5-inquiry/report.json`。

### 2.6 最終ゲート

| ゲート | 結果 |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS |
| `cargo test --workspace` | **222 passed / 0 failed** |
| OpenAPI 生成物一致 | PASS（SSE / Markdown / PDF を含む） |
| i18n | **549 keys**、3 言語一致 |
| frontend `tsc -b && vite build` | PASS（chunk size warning のみ） |
| `scripts/verify-db.sh` | 001〜024 全項目 PASS |
| Phase 5 API E2E | DIFF 21 / evidence 21、Field closure PASS |
| Golden set | 20変更、recall 1.0、precision 0.833333 |
| UI | Impact / Q-01 1440 / 607px、HTTP / console error 0 |
| 2.5M edge performance | 6-hop 1.07s、Neo4j 再投影 173.117s |

## 3. 未実施・境界

| 項目 | 状態 | 扱い |
|---|---|---|
| 自然言語 seed / `NEEDS_CLARIFICATION` | **実装（部分一致）** | DB の名前言及候補を最大500件取得。RAG 併用は未実装。曖昧なら止まる |
| Git unified diff / line-level seed | **PASS** | unified diff と原文 LCS の両方から行範囲を求め、最小包含 Symbol を seed にする |
| フィールド読み書き閉包 | **PASS** | Java 解析の `READS / WRITES` Fact と双方向グラフで読み書き method に閉包 |
| コードと文書の意味リンク | **一部** | 名前一致 chunk を `INFERRED` で返す。Ontology の `DESCRIBES` 実装ではない |
| テスト資産照合 | **一部** | 既存ケースは `covers_impacts` で照合。草稿は最大 20 件を提案（保存しない） |
| 20 変更 recall / precision | **PASS（合成 SAMPLE）** | recall 1.0 / precision 0.833333。顧客案件の正解ではない |
| PDF | **PASS** | `report.pdf`、日本語フォントサブセット。CI では font 未配置時に明示 409 |
| SSE `partial → done` | **PASS** | `text/event-stream`、`Last-Event-ID` 再開。5 段の順序と欠番を実測 |
| 250 万 Fact / 6 ホップ性能 | **PASS（合成）** | 2,500,005 edge、frontier 限定、1.07s。全件ロードなし |
| 本番 DB の impact report 保存形式 | **暫定** | `impact.assessment_draft.report` JSONB。大規模時は artifact へ退避 |
| RAG 併用 seed | **未接続** | 自然言語はグラフ名の決定的部分一致。外部 rerank / RAG 統合は境界外 |

---

## 4. 既知の制約

- 「影響なし」と「未解析」を区別する。`tests_not_available`、`documents_not_indexed...`、
  `unverified_items` は 200 + degraded / unverified で返す。
- 文書 item はコード Symbol 名との一致であり、コード境界や実行時データフローの証明ではない。
- `INFERRED` / `UNRESOLVED` は経路上の最も弱い辺を引き継ぐ。より近い `RESOLVED` 経路が
  別に存在しても、採用した最短経路の不確実性を消さない。
- graph node の degree が hub threshold を超える場合、その node は返すが先へ展開しない。
- 現時点の risk は breadth / uncertainty / depth / truncation の決定論的近似で、
  業務重要度・欠陥密度・テスト被覆は未接続。
- 応答は DRAFT であり、承認・コード変更・テスト実行の権限を持たない。
- 影響分析の冪等キーは依頼内容の SHA-256。**草稿行が作り直された**（復元・検証用
  リセット等）状態で同じ依頼を投げると、完了済みの旧 job が返って草稿が `PENDING`
  のまま残る問題があった。いまは「job が終態 かつ 草稿が `SUCCEEDED` でない」場合に
  別キーで再投入する。Phase 6 → Phase 7 の順番で流す回帰で検出・修正済み。

---

## 5. 再現

```bash
# 空 DB に 001〜027 と roles を適用
PGHOST=localhost PGUSER=<admin> ./scripts/verify-db.sh

./scripts/seed-dev.sh
psql -d <db> -f scripts/seed-phase5-doc.sql
./scripts/seed-phase4.sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p api --bin openapi -- --check

# API / worker 起動後
python3 scripts/phase4-eval.py     # コード Fact / graph を準備
SYSTIVRA_PROJECT_ID=22222222-2222-4222-8222-222222222222 PHASE5_MAX_ITEMS=498 \
  python3 scripts/phase5-impact-eval.py
SYSTIVRA_PROJECT_ID=22222222-2222-4222-8222-222222222222 \
  python3 scripts/phase5-golden-eval.py
SYSTIVRA_PROJECT_ID=22222222-2222-4222-8222-222222222222 \
  PGDATABASE=systivra \
  SYSTIVRA_PDF_FONT="/System/Library/Fonts/Supplemental/Arial Unicode.ttf" \
  python3 scripts/phase5-inquiry-eval.py

cd apps/web
npm run gen:api
npm run check:i18n
npm run build
npm run check:ui:phase5
npm run check:ui:phase5-inquiry

# 2.5M Fact / edge + Neo4j + 6-hop frontier 性能
./scripts/run-phase4-fact-scale.sh
```

Phase 5 evaluator は `SYSTIVRA_BASE_URL`、`SYSTIVRA_PROJECT_ID`、ログイン情報を
環境変数で変更できる。PDF は `SYSTIVRA_PDF_FONT` で日本語 TTF/OTF を指定する。
