# 詳細設計 06：コード解析エンジン（Java / JSP / XML / Fact）

対象：Phase 4「Code Analysis」（**5 週**）。`04-コアエンジン.md` §4 の設計を、13,000 Java / 1,000+ JSP の実規模で実装できる粒度に落とす。

**正本**：DDL は `detail/02-database-ddl.md`、Fact Schema は `05-データモデル.md` §4、関係型は `04-コアエンジン.md` §3 の Ontology 登録簿、位置情報は `00-全体設計.md` J-02 / J-03、受入は `08-実装計画.md` Phase 4。本稿はその間の**処理手順と契約**だけを定義する。

---

## 1. 入力規模と前提

| 項目 | 値 | 出典 |
|---|---|---|
| Java ファイル | **13,000** | A-02 |
| JSP ファイル | **1,000+** | A-02 |
| 設定 XML | `struts-config.xml` / `dicon` / `web.xml` / `*.tld` / MyBatis Mapper | A-01 |
| Fact | 250 万（上限 500 万） | A-02 |
| 対象フレームワーク | Struts1 / SAStruts / Seasar2 / Dicon / JSP カスタム Tag | A-01 |
| 文字コード | UTF-8 と **Shift_JIS / Windows-31J が混在** | J-02 |
| 実行環境 | 顧客閉域網。外部ネットワークなし | A-03 |
| 改コード | **禁止**（D-04）。出力は Fact と証拠だけ | D-04 |

**前提条件（満たせない場合は明示降格）**：

- `classpath` を顧客が提供できること。不能なら OpenRewrite の型解決を止め、全件を構文レベルに降格し `classpath_complete=false` を帳票に出す（R-03）。
- 解析対象は **SAMPLE + 実案件 1 件が使うフレームワークに限定**。全フレームワーク対応を Phase 4 で目指さない（R-07）。
- アダプタは登録式プラグイン。未知フレームワークは「解析しない」と明示し、黙って `RESOLVED` を出さない。

---

## 2. 全体パイプライン

```text
[1] 同期      git / SVN → code_revision（差分ハッシュ、変更ファイル一覧）
[2] 前処理    文字コード判定（chardetng + encoding_rs）→ UTF-8 内部表現
             元バイトと line_offsets（行開始バイト偏移）を artifact に保存
[3] 構文層    Rust + Tree-sitter：全 14,000 ファイルを全面走査
             → symbol / 構文 CALLS / DECLARES / JSP 構造 / XML 要素
[4] 意味層    Java 21 + OpenRewrite：Java のみ。classpath で型解決
             → シグネチャ確定 CALLS / EXTENDS / IMPLEMENTS
[5] 統合      NDJSON を Fact へ。衝突は OpenRewrite を優先し、
             Tree-sitter 側を INFERRED に降格
[6] アダプタ  Struts1 / SAStruts / Seasar2 / Dicon / Tag / 反射 / 動的プロキシ
             → HANDLES / DISPATCHES / DEPENDS_ON / SUBMITS_TO …
[7] 永続化    supersede_then_insert（1 tx）+ COPY バルク + evidence 添付
[8] 投影      outbox: FactAccepted → Neo4j（detail/07）。chunk は投影しない
```

> **[3] と [4] は別プロセス**。`crates/engine-code`（Rust）と `workers/rewrite`（Java 21 + Gradle）。境目は NDJSON だけで、共有メモリも RPC も使わない。片方が落ちても他方の成果は残る。

---

## 3. プロセス間契約（NDJSON）

`workers/rewrite` が 1 ファイルつき 1 行を stdout に書く。Rust 側は行単位で読み、途中で落ちた行は捨ててそのファイルだけ再投入する。

### 3.1 Rust → Java（入力）

```json
{"v":1,"job_id":"j8...","revision":"svn-18422","file":"src/main/java/.../UserEditAction.java","encoding":"WINDOWS_31J","classpath_key":"cp-9f2...","byte_size":18422}
```

| フィールド | 型 | 規約 |
|---|---|---|
| `v` | int | 契約版。不一致は拒否 |
| `revision` | string | `code.code_revision` と一致 |
| `file` | string | リポジトリ相対パス。**UTF-8** |
| `encoding` | enum | `UTF_8` / `SHIFT_JIS` / `WINDOWS_31J` / `EUC_JP` / `UNKNOWN` |
| `classpath_key` | string | `code.classpath_index` のキー。無い場合は `null` |
| `byte_size` | int | 元ファイルのバイト数。位置検証に使う |

`encoding = UNKNOWN` または判定信頼度 < 0.75 のときは **推測せず `ENCODING_UNDETECTED`** でスキップし、未解析一覧に出す（R-22）。

### 3.2 Java → Rust（出力）

```json
{"v":1,"file":"src/main/java/.../UserEditAction.java","ok":true,"classpath_complete":true,"facts":[{"relation":"CALLS","subject":{"kind":"Method","key":"jp.co...UserEditAction.update"},"object":{"kind":"Method","key":"jp.co...UserService.update(UserDto)"},"resolution":"RESOLVED","confidence":0.99,"evidence":[{"kind":"source","path":"src/main/java/.../UserEditAction.java","byte_start":8120,"byte_end":8164,"line":142,"quote":"userService.update(dto);"}]}]}
```

| フィールド | 規約 |
|---|---|
| `facts[]` | `05-データモデル.md` §4 の Fact Schema に適合。必須 12 項目 |
| `relation` | Ontology 登録簿の固定集合のみ。`^[A-Z_]+$` |
| `resolution` | `RESOLVED / INFERRED / PARTIAL / UNRESOLVED / CONFLICT` |
| `confidence` | 0〜1。`RESOLVED` は ≥ 0.95、`INFERRED` は 0.5〜0.9 を目安 |
| `evidence[]` | **1 件以上**。`quote` は原文の完全な部分文字列（NFC） |
| `byte_start / byte_end` | **元ファイルのバイト偏移**。UTF-8 変換後の偏移ではない |
| `line` | 元ファイルの行番号（1 始まり）。`line_offsets` から算出 |
| `classpath_complete` | false なら全 Fact を `PARTIAL` 以下に降格 |

**失敗行**：

```json
{"v":1,"file":".../Broken.jsp","ok":false,"error":{"code":"E-JSP-002","message":"scriptlet placeholder mismatch"}}
```

1 行の失敗でジョブを止めない。失敗は `code.analysis_run.error_summary` に集計し、UI に未解析率として出す。

---

## 4. 位置情報（日本語コーパスで最も壊れやすい箇所）

### 4.1 二つの偏移を混ぜない

| 種別 | 単位 | 用途 |
|---|---|---|
| `byte_start / byte_end` | **元ファイルのバイト** | 証拠の同一性、差分検出、`quote_hash` の検証 |
| `line / column` | 行番号 + 表示桁 | UI のジャンプ、ハイライト |
| `token_count / char_count` | UTF-8 正規化後 | chunk 予算（`detail/01` §6） |

`line_offsets: Vec<u64>` を `document_revision` と同じ要領で **解析 artifact（Parquet / オブジェクトストレージ）に保存**し、DB の `code.source_file` には `line_offsets_key` だけ持たせる。Shift_JIS の 2 バイト文字を 1 文字として数えて行をずらすと、証拠を開いても該当箇所が出ない（R-22）。

### 4.1.1 偏移の作り方（往復比較では足りない）

**「復号して再符号化し、元のバイト列と一致したら偏移を出す」では実案件で破綻する。**

CP932 には同じ Unicode に写像される重複バイト列がある。`髙`（U+9AD9）は `0xEEE0` と `0xFBFC` の両方が有効で、`encoding_rs` の再符号化は片方に寄せる。すると**中身が正しいのに往復が一致しない**ファイルが出る。`髙` は日本人名で頻出するので、これは例外ではなく通常ケースである。

正しい作り方は **元バイト列を先頭から歩く**こと。

```text
pos = 0
for ch in decoded_text.chars():
    len = 元ファイルで ch が占めるバイト数
    char_offset[ch] = pos
    if ch == '\n': line_offset.push(pos + len)
    pos += len
if pos != 元ファイル長: 偏移を出さない（exact = false）
```

`len` の決め方は 2 段：

1. **速い道**：`encode(ch)` の結果が `body[pos..]` の先頭と一致すれば、その長さ。
2. **遅い道**：一致しない場合（重複写像・外字）、`body[pos..pos+n]` を復号して `ch` 1 文字になる `n` を 1〜4 の範囲で探す。

速い道で大半が処理でき、遅い道に入るのは重複写像の文字だけ。1 文字でも決められなければ**そのファイルの偏移は一切出さない**（`exact = false`）。もっともらしい数字を返すと、証拠が別の場所を指したまま静かに通る。

実測（Phase 0、`crates/worker/src/text/`）：CP932 / EUC-JP / UTF-8 / UTF-8 BOM の 4 種、外字・半角カナ・JSP scriptlet 混在、914KB / 9,761 行まで **全件 `exact = true`**。壊れたバイト列のみ `ENCODING_UNDETECTED`。debug ビルドで約 89ms / 914KB。

### 4.2 正規化の二軌

- **保存・証拠**：原文を **NFC** で保持。`quote_hash = sha256(NFC(quote))`。
- **照合・マッチング**：NFKC + 旧字体マップ（全角英数、全角空格、舊字 → 新字）。
- `quote` は原文 NFC の部分文字列でなければならない。NFKC 後の文字列を `quote` に入れない（`detail/04` §2.3）。

### 4.3 JSP の等長プレースホルダ

JSP は HTML + 指令 + scriptlet の混在。Tree-sitter の HTML grammar で全体を解析する際、scriptlet を**同じバイト長・同じ行数**の空白 + 改行に置換してから HTML に渡す。

```text
原文  <% if (user != null) { %>      ← 28 バイト、1 行
置換  "                            " ← 28 バイト、1 行（改行は元の位置に残す）
```

- 置換で行番号もバイト偏移も変わらないことを**プロパティテスト**で検証：scriptlet をランダム挿入して位置が不変であること（R-02）。
- 位置が 2 行超ずれた場合は**ファイル単位の位置に退避**し、`resolution = PARTIAL`。精度より正確性を選ぶ。
- scriptlet 内の Java は別途 Java grammar で解析し、得た Fact の位置を元バイト偏移へ戻す。

---

## 5. フレームワーク別アダプタ

| # | 対象 | 入力 | 生成する関係 | 既定 resolution | 注意 |
|---|---|---|---|---|---|
| A-01 | `struts-config.xml` | `<action path="/userEdit" type="…UserEditAction">` | `HANDLES`（Action → path）、`DISPATCHES`（Action → JSP） | RESOLVED | `path` は画面 ID の正規化キー。大文字小文字を保持 |
| A-02 | `ActionForm` | フィールド + `validation.xml` | `VALIDATES`（Form → Rule）、`DEFINES` | RESOLVED | 日本語項目名は原文 NFC で保持 |
| A-03 | SAStruts | POJO Action + 注釈 / 規約 | `HANDLES`、`CALLS` | RESOLVED | 規約ベースの暗黙 URL は `INFERRED` に留める |
| A-04 | Seasar2 `*.dicon` | `<component name="userService" class="…">`、`<property>` | `DEPENDS_ON`（注入）、`DECLARES` | RESOLVED | 文字列で書かれたクラス名は FQN に解決できない場合 `PARTIAL` |
| A-05 | JSP カスタム Tag (`*.tld`) | taglib 宣言 + 使用箇所 | `REFERENCES`（JSP → Tag）、`DECLARES` | RESOLVED | TLD 未添付なら `UNRESOLVED`。推測しない |
| A-06 | JSP form | `<form action="…">` / Struts `<html:form>` | `SUBMITS_TO` | **PARTIAL** | 実行時依存。forward 先は config と突き合わせて昇格 |
| A-07 | 反射 | `Class.forName("…")` / `Method.invoke` | `CALLS`（候補集合） | UNRESOLVED / INFERRED | **`RESOLVED` と誤記しない**。候補は `fact.candidates[]` に全件残す |
| A-08 | 文字列連結 SQL | `"SELECT … " + tbl` | `QUERIES`（Table 候補） | UNRESOLVED | テーブル名が定数なら RESOLVED に昇格可 |
| A-09 | 動的プロキシ | `Proxy.newProxyInstance` / AOP 設定 | `DEPENDS_ON` | INFERRED | 実装クラスを確定させない |
| A-10 | MyBatis Mapper XML | namespace + `<select id>` + SQL | `QUERIES`（Mapper → Table） | RESOLVED | 動的 `<if>` は分岐を列挙し `candidates` へ |
| A-11 | 標準 Java | 構文 + 型解決 | `CALLS / EXTENDS / IMPLEMENTS / DECLARES / IMPORTS` | 下表 | — |

### 5.1 Fact 生成規則（`04-コアエンジン.md` §4 と同一）

| アナライザー | 関係 | 既定 resolution |
|---|---|---|
| Tree-sitter シンボル | `DECLARES` | RESOLVED（構文確定） |
| Tree-sitter 呼び出し | `CALLS`（名前一致のみ） | INFERRED |
| OpenRewrite メソッド解決 | `CALLS`（シグネチャまで） | RESOLVED |
| OpenRewrite 継承 | `EXTENDS` / `IMPLEMENTS` | RESOLVED |
| Struts config | `HANDLES` | RESOLVED |
| 注釈マッピング | `HANDLES` | RESOLVED |
| Dicon DI | `DEPENDS_ON` | RESOLVED |
| 反射 / 文字列連結 | `CALLS`（候補集合） | UNRESOLVED / INFERRED |
| JSP form → Action | `SUBMITS_TO` | PARTIAL |
| MyBatis XML | `QUERIES` | RESOLVED |

**衝突解決**：同じ `(subject, relation, object)` に複数アナライザーが出した場合、OpenRewrite > XML/config > Tree-sitter の順で採る。落とした側は削除せず `fact.candidates` / `analysis_run` に残し、影響分析で「なぜ RESOLVED か」を辿れるようにする。

**関係型の書き込み**：Cypher 5 では関係型をパラメータ化できない。Ontology 登録簿で `^[A-Z_]+$` を検証した識別子だけを文字列連結する。動的生成は禁止（`05` §3.3）。`SUBMITS_TO` は登録簿に追加済み。

---

## 6. 永続化規約

### 6.1 `supersede_then_insert`（必須）

`code.fact` には `fact_unique_live` の partial unique index がある。**増分再解析では 1 トランザクション内で順序を守る**。

```text
BEGIN
  1. UPDATE code.fact
        SET valid_to = now(), superseded_by = $new_id
      WHERE project_id=$1 AND revision_id=$2
        AND subject_key=$3 AND relation=$4 AND object_key=$5
        AND analysis_engine=$6 AND valid_to IS NULL;
  2. INSERT INTO code.fact (... 12 必須列 ...) VALUES (...);
  3. INSERT INTO code.fact_evidence (fact_id, evidence_id, role) ...;  -- 1 件以上
COMMIT
```

- repository 層の `supersede_then_insert(tx, facts)` **以外から `code.fact` へ INSERT させない**。
- 統合テスト：同一キーで 2 回再解析 / 並列 2 セッション / ロールバック後の live 行数が 1。
- 順序を逆にすると unique 違反で増分が止まる（`review-2026-11-05.md` C-2 の解決）。

### 6.2 バルクと性能

| 項目 | 規約 |
|---|---|
| 書き込み | `COPY` 5,000 行ごと。行 INSERT は使わない（`detail/03` §6.1） |
| Fact 250 万行 | 単一表・非分区。`(project_id, revision_id)` 索引 |
| 古い revision | `valid_to` 付きは export 後にアーカイブ。削除は論理 → 非同期 |
| `kb.chunk` との対応 | **コード Fact から chunk を作らない**。chunk は設計書由来だけで 1,200 万 |
| グラフ投影 | Fact は outbox 経由で `graph_meta.entity` / `relationship` と Neo4j へ。Fact 自体を Neo4j に置かない |

### 6.3 `classpath_index`

- 顧客提供の jar / 依存ツリーを `code.classpath_index` に登録し、`classpath_key` で参照。
- 構築失敗率 > 30% で Phase 4 のゲートを「全面構文レベル」に降格（R-03 / `08` §8）。
- 降格時は `classpath_complete=false` を `analysis_run` と帳票の両方に出す。**黙った精度低下を禁止**。

---

## 7. 増分解析

```text
1. code_revision を同期。前版との content_hash 差分で変更ファイル一覧を作る
2. 変更ファイル + その直接参照元（逆依存 1 層）だけを再解析対象にする
3. 依存する classpath / dicon / struts-config が変わった場合は影響ファイルを追加
4. 対象外は既存 Fact をそのまま残す（再計算しない）
5. 再解析分だけ supersede_then_insert
```

- 目標：1 ファイル変更で再解析が全件の **10% 未満**（`08` Phase 4 受入）。日次増分全体は `detail/03` §1 の < 40 分 / 目標 < 20 分に収める。
- `source_file.content_hash` が同じならスキップ。日本語ファイル名の正規化差（NFC / NFD）で偽差分を出さないよう、パスは **NFC で統一**してからハッシュする。
- 変更検出はバイト単位。Shift_JIS → UTF-8 変換後の比較ではない（J-02）。

---

## 8. 性能（13,000 + 1,000 ファイル）

| 段 | 速度目安 | 13,000 ファイル | 対策 |
|---|---|---|---|
| 文字コード判定 | 全ファイル | 数分 | 先頭 64KB で判定、信頼度 < 0.75 は保留 |
| Tree-sitter 構文 | 200 ファイル/秒/コア | 1〜2 分（8 コア） | ファイル単位で並列、grammar 版固定 |
| OpenRewrite 意味 | 20 ファイル/秒 | 約 11 分 | classpath を 1 回構築して使い回す |
| JSP 等長置換 | 300 ファイル/秒 | 数秒 | 位置は置換前のバイトで保持 |
| Fact 統合 + COPY | 50,000 行/秒 | 50 秒（250 万行） | NDJSON → COPY |
| 合計（初回） | — | **30〜60 分**（実測で校正） | Phase 4 中に 1,000 ファイルで較正 |

> この数字は Phase 4 の SAMPLE 較正値で置き換える。**60,000 冊 Excel の 24〜48 時間とは別工程**であり、両者を足して「初回取込」と呼ばない（`detail/03` §3）。

---

## 9. 失敗モードと降格

| # | 失敗 | 検知 | 挙動 | UI / 帳票 |
|---|---|---|---|---|
| F-01 | 文字コード判定不能 | 信頼度 < 0.75 | そのファイルをスキップ | 未解析一覧、`ENCODING_UNDETECTED` |
| F-02 | classpath 構築失敗 | 失敗率 > 30% | 全面構文レベル | `classpath_complete=false` |
| F-03 | OpenRewrite が 1 ファイルで例外 | `ok=false` 行 | そのファイルだけ Tree-sitter 結果を採用 | 部分失敗数 |
| F-04 | JSP 行位置が 2 行超ずれ | プロパティテスト | ファイル単位位置に退避 | `PARTIAL` 明示 |
| F-05 | 反射を RESOLVED と誤判定 | CI の SAMPLE 検査 | ビルド失敗 | — |
| F-06 | 未知フレームワーク | アダプタ未登録 | 解析しない | 「未対応」を明示。沈黙しない |
| F-07 | `fact_unique_live` 衝突 | unique 違反 | tx ロールバック + 警告 | 増分失敗として再実行 |
| F-08 | evidence が 0 件 | Schema 検証 | Fact を拒否 | `VALIDATION_SCHEMA` |

---

## 10. SAMPLE と golden fixture

`08-実装計画.md` の SAMPLE（24 画面、`is_sample=true`、UI 透かし）に以下を必ず含める。

| # | fixture | 検証 |
|---|---|---|
| C-01 | Struts1 `struts-config.xml` + Action + ActionForm | `HANDLES` / `DISPATCHES` が RESOLVED |
| C-02 | SAStruts の規約ベース URL | 暗黙 URL が INFERRED に留まる |
| C-03 | Seasar2 `*.dicon` の DI | `DEPENDS_ON` が RESOLVED、プロパティ名保持 |
| C-04 | JSP カスタム Tag（TLD あり / なし） | ありは RESOLVED、なしは UNRESOLVED |
| C-05 | scriptlet 混在 JSP 100 行 | 等長置換後も行番号・バイト偏移が不変 |
| C-06 | Shift_JIS の Java（全角文字 + 外字） | 文字化け 0、`quote` が原文部分文字列 |
| C-07 | Windows-31J の JSP（サロゲートペア含む） | `line_offsets` が正しい |
| C-08 | 反射 15 件 + 動的プロキシ 15 件 | **RESOLVED が 0**、すべて INFERRED / UNRESOLVED |
| C-09 | 文字列連結 SQL + MyBatis `<if>` | 定数は RESOLVED、動的分岐は candidates |
| C-10 | 同一キーの再解析 2 回 + 並列 2 セッション | live Fact が 1 行、`projection_seq` が後退しない |

各 fixture は `tests/fixtures/code/` に置き、Fact の NDJSON スナップショットを CI で比較する。**スナップショットに `confidence` の小数第 3 位以降を含めない**（環境差でフレークする）。

---

## 11. 受入（Phase 4）

`08-実装計画.md` Phase 4 をそのまま使う：

- SAMPLE 案件でクラス / メソッド認識率 **98% 以上**、メソッド呼び出し（`RESOLVED` 辺）の precision **98% 以上**。
- 反射 / 動的プロキシ 30 件がすべて `INFERRED` または `UNRESOLVED`。`RESOLVED` 誤記 **0**。
- 増分：1 ファイル変更で再解析が全件の **10% 未満**。
- classpath なし：構文レベルへ降格し `classpath_complete=false` が帳票に出る。
- 任意の Fact から `source_file` の元バイト位置へ戻れ、`quote` が原文の部分文字列として検証できる。
- **世界中のどの画面でも「解析不足」が隠れない**。未解析率・部分失敗数・降格理由を `analysis_run` から必ず表示する。

---

## 12. 他詳細設計との境目

| 境目 | 本稿の担当 | 相手の担当 |
|---|---|---|
| Fact → グラフ | outbox に `FactAccepted` を積むまで | `detail/07-graph-projection.md`（投影・再構築・漂流検出） |
| 位置情報の保存先 | `line_offsets_key` を渡す | `detail/01-excel-ingestion.md` §8（artifact 形式） |
| DDL / 索引 | 参照するだけ | `detail/02-database-ddl.md` |
| 速度 SLO | Phase 4 の較正値 | `detail/03-performance.md` |
| API | `code/analysis-runs` の job 返却 | `06-API.md` §2.4 / `detail/04-api-contract.md` §3 |
| 影響分析 | Fact の `resolution` / `confidence` を提供 | `04-コアエンジン.md` §5 |
