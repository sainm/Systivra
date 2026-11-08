# Phase 2 受入記録（RAG Engine 完成）

実施日: 2026-11-07  
対象: `08-実装計画.md` Phase 2 / `detail/01`〜`04`  
環境: macOS arm64、PostgreSQL 18.4 + pgvector 0.8.2、Rust 1.88 以上、Node.js 24

---

## 1. 受入項目

| # | 受入 | 結果 | 証拠 |
|---|---|---|---|
| 1 | 50 sheet の Excel で任意の 30 セル領域を top-3 で取得（80% 以上） | **100%**（30/30） | `scripts/phase2-eval.py` の `cells.top3` |
| 2 | 任意の chunk から `parse_run` と parser 版に遡れる | **PASS** | `GET /chunks/{cid}` が `parser=xlsx-native / docx-native / pptx-native`、`parser_version=0.1.0` を返す |
| 3 | embedding モデル切替時、旧索引も検索可能（`index_build` 分離） | **PASS（ローカルハッシュ 2 版で実測）** | v1 / v2 の build を READY にし、相互に activate。検索は有効 build の `model_id` を使い、旧 build は READY のまま残る |
| 4 | 3 分類それぞれ recall@10 ≥ 0.85 | **PASS** | word 1.0 / partial 1.0 / identifier・外字 1.0（各 20 問、計 60 問）。MRR 0.847、P95 39ms |
| 5 | 1,000 冊の一括取込が部分失敗を含めて完走し再開できる。1,200 万 chunk で P95 ≤ 2.0s | **一部未実施** | 部分成功・再開は 30 ファイルで実測（下記）。1,000 冊 / 1,200 万 chunk はこの環境では未実施 |

再現:

```bash
# 50 sheet の評価 corpus 生成 → bulk import → 60 問 + 30 セル領域
python3 scripts/phase2-eval.py

# DB 不変条件（migration 010 含む）
./scripts/verify-db.sh

# UI（Vite と API を起動した状態）
cd apps/web && npm run check:ui:phase2
```

---

## 2. 実測値

### 2.1 60 問評価（2026-11-07、run 001130）

| 分類 | 問数 | recall@10 | MRR |
|---|---:|---:|---:|
| word（日本語の語） | 20 | 1.000 | 0.542 |
| partial（部分一致） | 20 | 1.000 | 1.000 |
| identifier（識別子・外字） | 20 | 1.000 | 1.000 |
| **合計** | **60** | **1.000** | **0.847** |

- P95 latency: 39ms（API 応答の `latency_ms`）
- 30 セル領域 top-3: 100%
- 評価 corpus: 50 sheet（各 1 region、A1:C5）、DOCX 5 section、PPTX 5 slide。
  identifier の 5 問は外字 `髙橋` を含み、検索時は旧字体マップで `高橋` に正規化する。
- word の MRR が低いのは、汎用語（全 sheet 共通の「全角20文字以内」）を含むクエリで、
  正解 chunk が 1 位にならず 2〜5 位に入るため。recall@10 は満たす。

### 2.2 一括取込

- 5 行の manifest（正常 2 / 重複 1 / 不正 JSON 1 / 存在しないパス 1）:
  `SUCCEEDED_WITH_ERRORS`、`files_done=5`、`files_failed=2`、重複は登録されず。
- 再開: 30 ファイルの manifest を処理中に worker を kill（checkpoint line=6）。
  `WORKER_STALE_SEC=2` で再起動後、同じジョブが line 6 から再開し `files_done=30`、
  登録 document 30 件、重複 0 件。
- 子の `DOCUMENT_PARSE` ジョブは親の完了後に非同期で実行される。

### 2.3 UI

`npm run check:ui:phase2` の実測:

```json
{"chunkCount":50,"rootCount":50,"regionCount":1,"buildRows":2,"inspectChips":36,"errors":[]}
```

- Chunk Explorer: 構造木（SHEET → REGION）、チャンク一覧、locator / 本文表示。
- 索引管理: build 一覧、READY の有効化、モデル選択。
- 検索検査: score / rerank_score / sources / degraded / lexical_mode。
- 1440px と 607px のスクリーンショットで確認。607px では構造木と詳細が縦積み。
- Phase 1 の `ui-check.mjs`（検索 1 リクエスト、原文ハイライト、モーダル）も再実行し、回帰なし。

### 2.4 revision フィルタ

`filters.revision_id` を XLSX E2E の revision に固定して `U001` を検索し、
返却 2 件がともに同じ XLSX document / `kind=xlsx` であることを確認した。

---

### 2.5 改版管理（2026-11-08 / `scripts/phase2-revision-eval.py`）

同一文書に PDF の v1 / v2 を登録し、解析後に切替と版指定検索を実測。

| 項目 | 実測 |
|---|---|
| 改版登録 | `POST /documents/{did}/revisions` で v2 を登録、v1 は `SUPERSEDED` / v2 は `CURRENT` |
| chunk の現行性 | v1 の chunk は `is_current=false` / `superseded_at` 設定、v2 のみ `true` |
| 既定検索 | 現行版（v2）の chunk のみ 20 件 |
| 版指定検索 | `filters.revision_id = v1` で 20 件（非現行版でも引ける） |
| 切替 | `:activate` で v1 を `CURRENT` へ。既定検索が v1 に切り替わる |
| グラフ | 切替後、旧 v2 の Neo4j ノード **0**・PG `deleted_at` **101 件**、切替先 v1 のノード **101** |
| イベント / 監査 | `DocumentRevisionSuperseded` を publish、監査 `document.revision.activate` 1 行 |
| 原本 | 旧版 `GET /documents/{did}/revisions/{rid}/content` が v1 とバイト一致（1,053,531） |
| UI | 改版 2 行、CURRENT 1 行、切替ボタンで反転、1440 / 607 で pageerror 0 |

---

## 3. 実装済み範囲

| 領域 | 実装 |
|---|---|
| XLSX | 共有文字列、値、数式キャッシュ、日付（styles）、コメント、定義名、非表示シート、結合セル、図形アンカー。領域検出、ヘッダ判定、12 行 / 800 token チャンク。親 chunk（region block）+ 行 evidence（`source_type=XLSX_ROW`、子 chunk は保存しない） |
| DOCX | 見出し階層、段落、表。`section_path` + `paragraph` / `table` locator |
| PPTX | スライド、図形テキスト、表、図形アンカー。`slide` + `shape_ids` / `table` locator |
| DocNode | DOCUMENT / SHEET / SECTION / REGION / PARAGRAPH / TABLE / FIGURE。PPTX のスライドは SECTION として格納 |
| 検索 | Lindera(IPADIC) の単語 + 文字 bigram を FTS に併用。旧字体（髙→高、﨑→崎 など）を検索時に正規化。RRF（k=60）。ローカル語彙一致 rerank。revision / doc_type / business_domain フィルタ |
| 索引 | `index_build`、`embedding_model` 2 版、`is_active`、モデル切替、再埋め込みジョブ |
| 一括取込 | JSONL manifest（データディレクトリ内）、部分成功、失敗一覧、チェックポイント再開、子ジョブ |
| UI | Chunk Explorer、索引管理、検索検査、文書詳細の XLSX/DOCX/PPTX フォールバック表示 |
| 契約 | OpenAPI に bulk-import / tree / chunks / index-builds を追加。`npm run gen:api` で型生成 |

---

## 4. 未実施・降格（偽装しない）

| 項目 | 状態 | 理由 / 次の扱い |
|---|---|---|
| docling-serve による DOCX/PPTX | 未使用 | Docker / compose provider が無い。OOXML 直読みへ降格。docling のレイアウト推定は未検証 |
| pg_bigm | 未導入 | 開発 PostgreSQL に拡張が無い。Lindera + bigram フォールバックで運用し、`pg_bigm` 導入環境では L2 を有効化する設計を維持 |
| モデル rerank | 未実装 | 外部 reranker が無い。ローカル語彙一致のみで `rerank_local_heuristic` を返す |
| 親子 chunk | 一部実装 | XLSX は親 region chunk + 行 evidence（子は保存しない）。DOCX / PPTX の表は chunk 単位で、行 evidence は未実装 |
| 部分 HNSW（`WHERE model_id`） | 未実装 | モデル別 embedding は共存するが、HNSW は分区単位の通常索引。モデル切替は `index_build` の active build で行う |
| 改版履歴 UI / 版指定検索 | **実装** | 文書画面の改版履歴（CURRENT / SUPERSEDED、切替、原本、この版で検索）。API は改版登録 / 一覧 / 原本 / 切替。切替時に旧版のグラフ実体を tombstone し Neo4j からも削除（`DocumentRevisionSuperseded`）。検索は `filters.revision_id` 指定時のみ非現行版を対象にする |
| 1,000 冊 / 1,200 万 chunk の負荷試験 | 未実施 | ローカル環境に 12M chunk のデータセットが無い。P95 は 109〜1,509 chunk で 40〜43ms |
| S3 / MinIO | 未使用 | ローカル FS のまま。`upload_url` / `object_key` の契約は不変 |
| 和暦 / 縦書き / ルビ | 未実装 | Phase 2 の範囲外 |
| A-01（`docker compose up`） | 未実行 | 開発機に Docker / compose provider が無い。Podman の同一イメージ検証は Phase 1 で実施済み |

---

## 5. 既知の制約

- `embedding_count` は current chunk のみを数える。過去 revision の embedding は残るが、build の `chunk_count` には含めない。
- 一括取込の `manifest_url` は `SYSTIVRA_DATA_DIR` 配下に限定する。`path` はマニフェストからの相対パス。
- 一括取込の進捗は親ジョブが登録完了時点で `DONE` になる。子の解析進捗は各 `DOCUMENT_PARSE` ジョブを見る。
- Lindera の辞書は `embed-ipadic` でバイナリに同梱する。索引とクエリで同じ crate バージョンを使う。
- **`tokenized` は保存済みの値で固定**され、`fts` はその生成列。形態素解析器を更新しても
  既存 chunk には自動で効かない。日本語語彙（`住所` など）が古い DB で 0 件になるのは
  このためで、形態素分割そのものは実装済み。
  `POST /api/v1/projects/{pid}/knowledge/retokenize` が**古い行だけ**最大 5000 件
  ずつ再計算する（本文に和文があるのに `tokenized` に和文が無い行を抽出）。
  `updated` / `examined` / `remaining` を返し、`remaining=0` まで繰り返す。
  更新済み行を再読しないので `remaining` は必ず減る（冪等）。
  実測 `scripts/knowledge-retokenize-eval.py`: 住所 `0 件 → 1 件`、
  `fts` 命中 `0 → 1`、再実行の `updated=0`。返却は `DocumentIngest` の write 権限が必要。
- `cargo` の MSRV は 1.88（Lindera 6.2 の要求）。
