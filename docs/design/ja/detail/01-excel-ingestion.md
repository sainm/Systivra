# 詳細設計 01：Excel 取込・解析・チャンク化

対象：`engine-rag` の XLSX 取込、`ParserProvider` のうち XLSX 部分、`doc_node` / `chunk` 生成。
前提：設計書 60,000 冊、大半が Excel。ADR-011（XLSX は自作解析）に従う。

---

## 1. 範囲

| 含む | 含まない |
|---|---|
| `.xlsx` / `.xlsm` の解析 | `.xls`（LibreOffice で `.xlsx` に変換してから同じ経路） |
| セル値・数式・結合・図形アンカー・定義名・リンク・コメント | ピボットテーブルの再計算、マクロ実行、外部リンクの解決 |
| 領域検出、ヘッダ判定、12 行単位のチャンク生成 | グラフ投影（chunk は投影しない） |
| 圧縮 artifact の生成と参照 | 帳票レンダリング（UI 側） |

---

## 2. 入力前処理

```text
受信（presigned URL または バッチ manifest）
  → サイズ判定：上限 100 MB / 500,000 セル / 50 シート（超過は E-XLSX-003）
  → 拡張子と MIME の一致確認（不一致は E-XLSX-004）
  → 暗号化判定：OLE の EncryptedPackage なら E-XLSX-002（復号はしない）
  → ZIP 妥当性：`[Content_Types].xml` と `xl/workbook.xml` の存在
  → content_hash（SHA-256）で重複排除
  → `document_revision` 作成（source_encoding は XLSX 内部 XML のため UTF-8）
  → parse job enqueue（priority, project_id, revision_id）
```

**やらないこと**：パスワード解除、破損ファイルの修復、`.xls` の自力パース。失敗は明示し、部分成功で止めない。

---

## 3. 内部モデル

```rust
pub struct Workbook {
    pub sheets: Vec<Sheet>,
    pub defined_names: Vec<DefinedName>,
    pub date1904: bool,
}

pub struct Sheet {
    pub name: String,
    pub index: u32,
    pub visibility: Visibility,     // Visible / Hidden / VeryHidden
    pub dimension: Option<Range>,   // 使用範囲（信頼しすぎない）
    pub cells: CellGrid,
    pub merged: Vec<MergedRange>,
    pub drawings: Vec<Drawing>,
    pub hyperlinks: Vec<Hyperlink>,
    pub tables: Vec<TableDef>,
    pub col_widths: Vec<(u32, f64)>,
    pub row_heights: Vec<(u32, f64)>,
}

pub struct Cell {
    pub row: u32,           // 1 始まり
    pub col: u32,           // 1 始まり（A=1）
    pub value: CellValue,   // Empty / String / Number / Bool / Date / Error
    pub formula: Option<Formula>,
    pub style_id: u32,
    pub comment: Option<String>,
}

pub struct Formula {
    pub text: String,             // "=SUM(C12:C20)"
    pub cached: Option<CellValue>,// キャッシュ値。なければ None
    pub shared_index: Option<u32>,
}

pub struct MergedRange {
    pub range: Range,       // C12:F12
    pub anchor: (u32, u32), // 値を持つ左上セル
}

pub struct Drawing {
    pub kind: DrawingKind,  // Picture / Chart / Shape
    pub anchor: Anchor,     // TwoCell / OneCell / Absolute
    pub rel_id: String,
    pub name: Option<String>,
}
```

### 3.1 OOXML の読み先

| 情報 | パート |
|---|---|
| シート一覧・定義名 | `xl/workbook.xml` |
| シート関係 | `xl/_rels/workbook.xml.rels` |
| セル・結合・リンク・図形参照 | `xl/worksheets/sheetN.xml` |
| 共有文字列 | `xl/sharedStrings.xml` |
| スタイル | `xl/styles.xml` |
| 図形・画像・グラフ | `xl/drawings/drawingN.xml` |
| 表（ListObject） | `xl/tables/tableN.xml` |
| コメント | `xl/commentsN.xml` |
| メディア | `xl/media/*` |

`calamine` で値と結合を取得し、`quick-xml` で drawings / tables / definedNames を補完する。calamine が返さない要素を推測で埋めない。

> **Phase 2 実装状況**：`crates/worker/src/xlsx/` は calamine を使わず、ZIP + XML を直読みする。共有文字列、数値、真偽、日付（styles の numFmt）、数式キャッシュ、コメント、定義名、非表示シート、結合セル、図形アンカーを読み、領域検出・ヘッダ判定・12 行（800 token 上限）チャンクまで実装済み。図形の media は SHA-256 を保持する。

### 3.2 値の扱い

| 型 | 扱い |
|---|---|
| 文字列 | 共有文字列を解決。前後空白は保持（設計書は空白に意味がある） |
| 数値 | `f64`。表示形式は保持しない（必要なら style_id から後段で解決） |
| 日付 | シリアル値を `date1904` で補正し `YYYY-MM-DD` |
| 真偽 | `TRUE` / `FALSE` |
| エラー | `#REF!` などをそのまま保持し、`ERROR` として記録 |
| 数式 | `formula.text` と `cached` を両方保持。`cached` がなければ `NO_CACHED_VALUE` |
| 空セル | 生成しない（メモリ節約）。ただし結合範囲の展開時は論理セルを作る |

---

## 4. 領域検出

### 4.1 使用範囲の決定

`dimension` は壊れていることがある（書式だけの列を含む）。以下で補正する。

```text
1. dimension を初期値とする
2. 値・数式・コメント・結合のいずれかを持つセルの最大行/列を走査
3. dimension が実体より大きい場合、実体に合わせる
4. 実体が dimension より大きい場合、実体を採用し警告を記録
```

「書式だけのセル」は値ではない。`style_id` だけで使用範囲を広げない。

### 4.2 領域（region）分割

```text
入力：occupancy[row][col] = 値/数式/コメント/結合あり
1. 空行・空列で分割する。ただし連続 1 行/1 列の空隙は same region とみなす
2. 各ブロックの外接矩形を region とする
3. 面積 < 4 セルの region はノイズとして結合（近傍 1 セル以内）
4. 同一 region 内で、全列が空の行が 3 行以上続く場合はサブ region に分割
5. region に ID を振る：`{sheet}:{r1}:{c1}-{r2}:{c2}`
```

**空隙 1 行を許容する理由**：設計書は見出しと明細の間に空行を 1 行入れることが多い。厳密に切ると 1 論理表が 2 つに割れる。

### 4.3 ヘッダ判定

region の先頭 3 行を採点する。

| 信号 | 点数 |
|---|---|
| 行内の非空セルがすべて文字列 | +2 |
| 結合セルが行を横断する | +2 |
| 次行の型が文字列→数値/日付に変わる | +2 |
| セルが太字（styles.xml の font.bold） | +1 |
| 背景色が白以外（fill） | +1 |
| 列数と一致する非空セルがある | +1 |

最大点の行をヘッダとする。最高点 < 3 なら「ヘッダなし」とし、キーは `R{row}C{col}` にする。

---

## 5. 結合セル

```text
for merged in sheet.merged:
    anchor_value = cells[merged.anchor]
    for (r, c) in merged.range:
        if (r, c) != merged.anchor:
            cells[r][c] = LogicalCell {
                value: anchor_value,
                merged_from: merged.anchor,
                is_merged_child: true,
            }
```

- 描画時は子セルに値を繰り返さない。`merged_from` を持つ子は「親の値を参照」と表示する。
- チャンク生成時は、親の値を 1 回だけ使う。子を展開すると同じ文字が 6 回出て検索ノイズになる。
- ヘッダ行が結合されている場合、結合幅を列数とみなしてヘッダを展開する。

---

## 6. 数式

| 状況 | 扱い |
|---|---|
| `cached` あり | 値を表示・検索に使用。`formula.text` も保持 |
| `cached` なし | 値を空にし、`NO_CACHED_VALUE` を記録。検索対象にしない |
| 共有数式（`t="shared"`） | マスターの式を復元し、各セルに展開 |
| 外部参照 | 式文本のみ保持。値は `UNRESOLVED` |
| 循環参照 | `ERROR` として記録 |

数式を評価しない。評価器を作ると Excel 互換性の沼に入る。

---

## 7. 図形・画像・グラフ

```text
drawings/drawingN.xml
  → twoCellAnchor / oneCellAnchor / absoluteAnchor を読む
  → from(row, col, rowOff, colOff) をアンカーセルに変換
  → pic / graphicFrame / sp を分類
  → 画像は xl/media から取り出し、SHA-256 を付与
  → グラフは kind=Chart、内容は UNSUPPORTED_CONTENT として記録
```

- 図形は `doc_node` の `FIGURE` ノードにする。`locator` は `{sheet, anchor_cell, offset}`。
- 画像の OCR は初期は行わない。図中の文字が必要なら Phase 6 以降で OCR を追加する。
- グラフの系列データは取得しない。タイトルとアンカーだけ記録する。

**実装メモ（Phase 0 の検証で確定）**

- **セル値を読むライブラリに図形は取れない。** 多くのライブラリは `xl/drawings/` を無視する。図形がどのセルに貼られているかが分からないと「この項目の説明図」を証拠に結び付けられないので、`xl/drawings/drawingN.xml` を直接読む（`crates/worker/src/xlsx/`）。
- `xdr:from` の `col` / `row` は **0 始まり**。A1 に直すときは +1 する。
- `absoluteAnchor` はセルに紐づかない。左上（A1）として扱い、`resolution = PARTIAL` にする。
- `graphicFrame` はグラフとは限らない（表・SmartArt も同じ要素）。`c:chart` の有無で分ける。
- 画像の同一性は `xl/media/*` の SHA-256。同じ画像が複数箇所に貼られるので、パスではなくハッシュで束ねる。
- 図形が 1 つも無いブックには `drawing` パーツが存在しない。**「drawing が無い」は異常ではない**（空を返す）。


---

## 8. チャンク生成

### 8.1 単位

- **親 chunk**：region を 12 行ごとに分割したブロック。ヘッダ行を各ブロックの先頭に繰り返す。
- **子（evidence unit）**：1 行。保存しない。`locator.row` で参照する。
- region が 12 行以下なら 1 chunk。
- 1 chunk の token が 800 を超える場合、6 行に分割する。それでも超える場合はセル単位で切る。

### 8.2 描画形式

ヘッダあり：

```text
【ユーザ編集画面 / 画面項目定義】
項目名: 住所
入力形式: 全角20文字以内
必須: 必須
エラー時メッセージ: 住所を正しく入力してください
---
項目名: 氏名
...
```

ヘッダなし：

```text
【Sheet1 / R12:C3-F18】
R12C3: 住所
R12C4: 全角20文字以内
...
```

- 1 行を 1 ブロックにし、`key: value` を改行で並べる。CSV にしない。
- 12 行を超える塊でも、行の区切りは `---` で明示する。
- 空値は出力しない（`必須: ` のような空行を作らない）。

### 8.3 locator

```json
{
  "kind": "xlsx",
  "sheet": "画面項目定義",
  "range": "C12:F23",
  "rows": [12, 23],
  "anchor_cell": "C12",
  "merged_span": "C12:F12",
  "region_id": "画面項目定義:12:3-23:6"
}
```

`range` は chunk の範囲、`rows` は開始/終了行。子の引用は `range` を 1 行に狭める。

### 8.4 chunk ID

```text
chunk_id = uuid_v5(project_id,
  f"{revision_id}:{sheet}:{range}:{content_hash[:16]}")
```

内容が変われば ID が変わる。同じ内容の再取込は同一 ID になり冪等。

---

## 9. 圧縮 artifact

セル格子は PostgreSQL に置かない。Parquet（zstd）で object storage に保存する。

| 列 | 型 | 備考 |
|---|---|---|
| `sheet` | string | シート名 |
| `row` | int32 | 1 始まり |
| `col` | int32 | 1 始まり |
| `value_type` | string | STRING / NUMBER / BOOL / DATE / ERROR / EMPTY |
| `value` | string | 文字列表現。数値は丸めない |
| `formula` | string | nullable |
| `cached_value` | string | nullable |
| `style_id` | int32 | nullable |
| `merged_from_row` | int32 | nullable |
| `merged_from_col` | int32 | nullable |
| `comment` | string | nullable |

保存先：

```text
s3://sep-parse/{project_id}/{document_id}/{revision_id}/cells.parquet
s3://sep-parse/{project_id}/{document_id}/{revision_id}/drawings/{sha256}.png
```

`document_revision.parse_artifact_key` に Parquet のキーを保存する。

---

## 10. エラー分類

| コード | 意味 | 再試行 | ユーザー向け説明 |
|---|---|---|---|
| `E-XLSX-001` | ZIP/OOXML 破損 | 不可 | ファイルが壊れています。元のアプリで開き直してください |
| `E-XLSX-002` | 暗号化 | 不可 | パスワード保護を解除してください |
| `E-XLSX-003` | サイズ/セル数超過 | 不可 | ファイルを分割してください（上限 100MB / 50 万セル） |
| `E-XLSX-004` | 拡張子と中身の不一致 | 不可 | 拡張子を確認してください |
| `E-XLSX-005` | シート 0 件 | 不可 | 可視シートがありません |
| `E-XLSX-006` | メモリ超過 | 可（分割後） | ファイルが大きすぎます |
| `E-XLSX-007` | 数式キャッシュなし（警告） | — | 一部の数式は値が取れませんでした |

エラーは `parse_run.error` に JSON で保存し、ファイル単位で失敗させる。バッチ全体は止めない。

---

## 11. バッチ取込

### 11.1 マニフェスト

JSON Lines。1 行 1 ファイル。

```json
{"path":"/design/01/user_edit.xlsx","doc_type":"DETAILED_DESIGN","business_domain":"user","acl_groups":["grp-dev"],"source_revision":"svn-18422"}
{"path":"/design/01/order_list.xlsx","doc_type":"UI_SPEC","business_domain":"order","acl_groups":["grp-dev"]}
```

- `doc_type` は `detail/04-api-contract.md` §0.3 の列挙に従う（`DETAILED_DESIGN` / `UI_SPEC` など）。`DETAILED_SPEC` は存在しない。
- `path` は共有ストレージ上のパス、または presigned URL。
- 同一 `content_hash` はスキップし、`skipped_duplicate` に計上する。
- 1 行でも不正なら、その行だけ失敗させる。

### 11.2 実行

```text
bulk_import job
  → manifest を読み、10,000 件ずつ子ジョブに分割
  → 子ジョブはファイル単位で並列実行（8〜16）
  → 進捗：files_done / files_total、cells_done / cells_total、chunks_done
  → 失敗は failure_list に追記（path, code, message）
  → 全子ジョブ完了後、サマリを確定
```

- 途中再開：`job_checkpoint` に処理済み manifest 行番号を保存。
- 部分成功を許す。`files_failed > 0` でも `status=SUCCEEDED_WITH_ERRORS`。
- 1 ファイルのタイムアウトは 5 分。超過は `E-XLSX-006`。

---

## 12. 性能とメモリ

| 指標 | 目標 | 備考 |
|---|---|---|
| 解析速度 | 5,000 セル/秒/コア | 結合・数式込み |
| 1 冊（28,800 セル） | 5.8 秒 | 理論値。実測で校正 |
| メモリ/冊 | セル数 × 120 B 以下 | 50 万セルで 60 MB |
| 同時実行 | 8〜16 ファイル | CPU コア数に合わせる |
| 60,000 冊（**解析のみ**） | 12〜24 時間 | 16 ワーカー |

> この「12〜24 時間」は **Excel 解析工程だけ**の値。Embedding / HNSW / DB 書き込みを含む **初回取込 end-to-end は 16 ワーカーで 24〜48 時間（受入 <48h）、32 ワーカーで 13〜25 時間（目標 <24h）**。正本は `detail/03-performance.md` §3。本稿の数字を顧客への取込時間として使わない。

メモリ上限を超えそうな場合は、シート単位でストリーム処理し、`CellGrid` を解放してから次へ進む。全シートを同時に保持しない。

---

## 13. 受入テスト（ゴールデンファイル）

| # | ファイル | 検証 |
|---|---|---|
| X-01 | 結合セルだらけの画面項目定義 | 結合が展開され、値の重複がない |
| X-02 | 数式 + キャッシュあり | 値が検索でき、式も保持される |
| X-03 | 数式 + キャッシュなし | `NO_CACHED_VALUE`、検索対象外 |
| X-04 | 図形アンカー + 画像 3 枚 | locator がアンカーセルを指す |
| X-05 | 非表示シート / VeryHidden | 既定では取込まない（設定で切替） |
| X-06 | 1,000 行 × 30 列 | 12 行チャンクに分割され、ヘッダが繰り返される |
| X-07 | ヘッダなしの表 | `R12C3` 形式のキーになる |
| X-08 | 結合ヘッダ | ヘッダが列数分に展開される |
| X-09 | 50MB / 40 万セル | 5 分以内、メモリ < 60MB |
| X-10 | 暗号化ファイル | `E-XLSX-002`、再試行しない |

各ファイルは `tests/fixtures/xlsx/` に置き、CI で解析結果のスナップショットを比較する。
