# 詳細設計 08：変更影響問合せ（単独問合せ / Change Inquiry）

対象：**「XX を変更したら何が起きるか」を 1 画面で単独に問える入口**。変更のたびに影響分析の主ワークベンチ（A-04）を開かずに、決まった 5 つの答えを返す。

```text
入力  「XX の変更」
  ↓
① どんな影響か     → summary   変更の効き方、規模、リスク、確度、打切り
② 影響する画面     → screens   影響する画面・フォーム・JSP
③ 影響する文書     → documents 影響する設計書と該当箇所（Sheet!Range / ページ）
④ 影響するコード   → code      影響する Java / JSP / SQL / 設定
⑤ どうテストするか → tests     既存ケース、追加すべき草稿ケース、実行手順、データ境界
```

> **本稿の最重要判断**：これは**新しい影響分析エンジンを作らない**。`04-コアエンジン.md` §4.5 の ImpactEngine に **`profile = CHANGE_INQUIRY`** を渡し、得られた `ImpactReport` を 4 分類へ投影したもの。追加するのは「入力の正規化」「分類投影」「テスト提案の編成」だけ。第二の影響ロジックを作ると、A-04 と Q-01 で答えが食い違う。

---

## 1. 位置づけと正本

| 項目 | 正本 |
|---|---|
| 影響の算出 | `04-コアエンジン.md` §4.5、`detail/03-performance.md` §4 |
| ImpactReport の形 | `04-コアエンジン.md` §4.5「出力」、`detail/04-api-contract.md` §4.2 / §4.3 |
| 証拠 / quote | `detail/04-api-contract.md` §2.3、`00-全体設計.md` J-03 |
| API 共通規約 | `detail/04-api-contract.md` §0（UUID / エラー / SSE / 冪等 / `latency_ms`） |
| 画面 | `07-UI.md` Q-01 |
| 改コード禁止 | D-04。本機能は**一切コードを書かない** |
| 速度 | `detail/03-performance.md` §1 / §4 |

**やらないこと**：

| やらない | 理由 |
|---|---|
| ソースコードの自動変更 | D-04（首期禁止、将来も承認必須） |
| テストの自動実行 | 環境と承認を伴う別操作。§7 で計画に渡すだけ |
| AI に結論を書かせる | 主表と `risk.score` は決定論的。AI は `narrative` と `hypotheses[]` のみ |
| 文書を全件一覧で返す | 60,000 冊。サーバー側ページング必須 |
| 独自の影響スコア | `ImpactReport.risk.score` をそのまま使う |

### 1.1 表示キーの規約

画面の表示キーは `graph_meta.entity.natural_key`（例 `SCR-USER-EDIT`）。`code` 列ではないため、API では `code` ではなく **`natural_key`** として返す。`test.test_case` だけは `code` 列を持つので `code` を返す（`detail/04` §0.2）。

---

## 2. 入力

### 2.1 3 つの入力形

| kind | 例 | 解決方法 | 確度 |
|---|---|---|---|
| `NATURAL_LANGUAGE` | 「住所の入力桁数を全角 20 文字から 30 文字に変えたい」 | 日本語クエリ正規化 → RAG 検索 + `entity_search` で**シード候補**を作る | 要確認（§2.2） |
| `DIFF` | `git diff abc123..def456` | 変更行 → `code.symbol`、フィールド変更 → 読み書きメソッド閉包 | 高 |
| `ENTITY` | `{"entity_ids":["e009..."],"doc_types":["UI_SPEC"]}` | 指定実体をそのままシード | 高 |

> 自然言語は**シードを探すためだけ**に使う。結論・リスク・証拠を AI が生成しない。日本語は NFKC + 旧字体マップで照合し、保存と `quote` は原文 NFC（`00` J-03）。全角数字・全角空格・外字・サロゲートペア・和暦を入力してもシードが落ちないことを golden テストで担保する。

### 2.2 シード確認（自然言語のときだけ）

シード候補が複数、または確度 < 0.7 の場合は**推測で進めず** `NEEDS_CLARIFICATION` を返し、候補を提示する。

```json
{
  "inquiry_id": "q7b2...",
  "status": "NEEDS_CLARIFICATION",
  "candidates": [
    {"entity_id":"e009...","natural_key":"SCR-USER-EDIT","label":"ユーザ編集画面","score":0.81,"reason":"「住所」「入力桁数」が UI_SPEC の C12:F12 に一致"},
    {"entity_id":"e014...","natural_key":"SCR-USER-REG","label":"ユーザ登録画面","score":0.66,"reason":"同じ住所項目が別画面にもある"}
  ],
  "hint": "対象を 1 つ選ぶか、両方を選んでください。選ばないと分析を開始しません。"
}
```

勝手に 1 つへ絞って「影響は 3 件です」と答えるのが、**本機能で最も危険な失敗**。候補には必ず `reason`（どの語がどの locator に当たったか）を付ける。

### 2.3 入力の正規化

| 段 | 処理 | 失敗時 |
|---|---|---|
| 1 | 原文保存（NFC、前後空白のみ除去） | — |
| 2 | 長さ検査（2,000 字） | `VALIDATION_SCHEMA` |
| 3 | 日本語正規化（NFKC + 旧字体 + 全角→半角は**照合時のみ**） | そのまま照合 |
| 4 | 期間・版の抽出（「2024 年度版」「v1.4」） | 指定なしとして扱う |
| 5 | シード検索（RAG top 20 + entity_search top 20 → RRF） | `NEEDS_CLARIFICATION`（候補 0 なら `NOT_FOUND_SEED`） |

---

## 3. API 契約

`detail/04-api-contract.md` §0 の共通規約に従う。`project_id` は ctx から注入、パス `{pid}` はスコープ宣言として照合。全リソース ID は UUID。

### 3.1 実行（非同期）

```http
POST /api/v1/projects/{pid}/change-inquiries
Idempotency-Key: sha256:9f1c...
Content-Type: application/json
```

```json
{
  "question": "住所の入力桁数を全角20文字から30文字に変えたい",
  "input": {"kind": "NATURAL_LANGUAGE"},
  "scope": {"max_depth": 6, "hub_threshold": 200, "include_tests": true},
  "mode": "FULL",
  "include": ["screens", "documents", "code", "tests", "evidence"]
}
```

| フィールド | 規約 |
|---|---|
| `question` | 原文を保持。監査と再実行のキー。最大 2,000 字 |
| `input.kind` | `NATURAL_LANGUAGE` / `DIFF` / `ENTITY` |
| `mode` | `QUICK`（2 ホップ、テストは既存ケースのみ）/ `FULL`（6 ホップ） |
| `scope` | `detail/04` §4.1 の `options` と同じ意味 |
| `include` | 不要な section を省ける。既定は全て |
| `Idempotency-Key` | 内容ハッシュ。重複送信は同一 `inquiry_id` / 同一結果 |

```json
202
{
  "inquiry_id": "q7b2...",
  "impact_id": "i3d7...",
  "job_id": "j6...",
  "status": "QUEUED",
  "links": {
    "self": "/api/v1/change-inquiries/q7b2...",
    "events": "/api/v1/jobs/j6.../events",
    "workbench": "/ui/analysis/impact/i3d7..."
  }
}
```

> `impact_id` を必ず返す。Q-01 の答えは A-04 / A-05 と**同じ ImpactReport**。掘り下げたくなったら主ワークベンチへそのまま遷移できる。

### 3.2 分割 SSE（5 段を順に出す）

全部が出るまで待たせない。各 section が確定した時点で `partial` を流す。

```text
id: 51
event: progress
data: {"phase":"SEED","done":2,"total":2}

id: 52
event: partial
data: {"section":"summary","data":{"risk":{"score":0.62,"level":"MEDIUM"},"counts":{"screens":3,"documents":7,"code":12,"tests":9},"degraded":[]}}

id: 53
event: partial
data: {"section":"screens","data":{"items":[],"total":3,"has_more":false}}

id: 54
event: partial
data: {"section":"documents","data":{"items":[],"total":7,"has_more":false}}

id: 55
event: partial
data: {"section":"code","data":{"items":[],"total":12,"has_more":false}}

id: 56
event: partial
data: {"section":"tests","data":{"existing":[],"drafts":[],"steps":[]}}

id: 57
event: done
data: {"inquiry_id":"q7b2...","impact_id":"i3d7...","status":"SUCCEEDED","latency_ms":8420}
```

- `id` 単調増加、`Last-Event-ID` で再開、20 秒ごとに `: ping`（`detail/04` §3.2）。
- `summary` を**最初**に出す。以降が遅れても「何が起きるか」だけ先に読める。
- 1 つの section が失敗しても他は出す。失敗は `section_errors[]` と `degraded[]` に明示し、**空配列で誤魔化さない**（空と失敗は違う）。
- `items` は SSE では先頭 20 件まで。続きは §3.3 のカーソルで取る。

### 3.3 取得（再訪・共有）

```http
GET /api/v1/change-inquiries/{qid}?include=screens,documents,code,tests&cursor=<opaque>&limit=20
```

```json
200
{
  "inquiry_id": "q7b2...",
  "impact_id": "i3d7...",
  "question": "住所の入力桁数を全角20文字から30文字に変えたい",
  "status": "SUCCEEDED",
  "engine_version": "impact-0.3.1",
  "ontology_version": "v1.4",
  "summary": {
    "headline": "入力規則の変更です。画面 3 件、設計書 7 箇所、コード 12 箇所、テスト 9 件に波及します。",
    "risk": {"score": 0.62, "level": "MEDIUM", "factors": [{"name":"impact_breadth","weight":0.20,"value":0.74}]},
    "counts": {"screens": 3, "documents": 7, "code": 12, "tests": 9},
    "truncated": {"hubs": 1, "depth": false, "budget": false},
    "degraded": []
  },
  "screens": {
    "total": 3, "has_more": false, "next_cursor": null,
    "items": [{
      "entity_id": "e009...",
      "natural_key": "SCR-USER-EDIT",
      "label": "ユーザ編集画面",
      "level": "L1",
      "why": "住所項目の入力規則を直接持つ",
      "quote": "項目名: 住所 / 入力形式: 全角20文字以内 / 必須: 必須",
      "evidence_id": "e7f3...",
      "locator": {"kind":"xlsx","sheet":"画面項目定義","range":"C12:F12","document_id":"d1a2..."}
    }]
  },
  "documents": {
    "total": 7, "has_more": false, "next_cursor": null,
    "items": [{
      "document_id": "d1a2...",
      "doc_type": "UI_SPEC",
      "title": "ユーザ管理機能 画面設計書",
      "locator": {"kind":"xlsx","sheet":"画面項目定義","range":"C12:F12"},
      "quote": "全角20文字以内",
      "evidence_id": "e7f3...",
      "why": "桁数の定義元。ここを直さないと設計書と実装が食い違う"
    }]
  },
  "code": {
    "total": 12, "has_more": false, "next_cursor": null,
    "items": [{
      "symbol_id": "s011...",
      "qualified_name": "jp.co...UserEditAction#validate",
      "file": "src/main/java/.../UserEditAction.java",
      "line": 142,
      "resolution": "RESOLVED",
      "confidence": 0.99,
      "relation": "VALIDATES",
      "quote": "if (addr.length() > 20) { ... }",
      "evidence_id": "e011..."
    }]
  },
  "tests": {
    "existing": [{"test_case_id":"tc18...","code":"TC-2026-0118","title":"住所に半角英字を入力した場合","status":"APPROVED"}],
    "drafts": [{"draft_id":"dr01...","title":"全角29文字の境界が通ること","status":"DRAFT","covers":["ii01..."],"evidence":["e7f3..."]}],
    "steps": ["ユーザ編集画面を開く","住所に全角29文字を入力する","登録を押す","エラーが出ないこと"],
    "data_boundaries": [
      {"rule":"全角20文字以内","values":["全角19文字","全角20文字","全角21文字","全角30文字"],"note":"上限変更後は 29/30/31 を追加"}
    ],
    "not_covered": ["半角カナの扱い（既存ルールに定義なし）"]
  }
}
```

**errors**：`NOT_FOUND_SEED`（シード 0 件）、`VALIDATION_SCHEMA`（`question` 長すぎ / `kind` 不正）、`CAPABILITY_UNSUPPORTED`（`include` に `tests` を指定したがテスト資産が未投入）、`EVIDENCE_QUOTE_MISMATCH`（証拠の再検証に失敗）。

### 3.4 一覧・再実行

| メソッド | パス | 説明 |
|---|---|---|
| GET | `/projects/{pid}/change-inquiries?cursor=&limit=` | 自分の問合せ履歴（質問・状態・件数サマリ） |
| POST | `/change-inquiries/{qid}:rerun` | 同じ `question` と `input` で再実行。`impact_id` は新規 |
| GET | `/change-inquiries/{qid}/report.md` | Markdown 出力（共有・チケット添付用） |

`rerun` は**監査に残す**。設計書やコードが更新された後に同じ質問を投げ直し、差分を比較できるようにする。

---

## 4. 出力 5 セクションの意味

| section | 答える問い | 必須フィールド | 空の意味 |
|---|---|---|---|
| `summary` | 何がどれだけ効くか | `headline` / `risk` / `counts` / `truncated` / `degraded` | 起きない（必ず 1 つ返す） |
| `screens` | どの画面が変わるか | `items[].natural_key` / `why` / `quote` / `locator` | 「影響する画面は無い」と断言 |
| `documents` | どの設計書のどこか | `document_id` / `doc_type` / `locator` / `quote` | 同上。**未投入なら `degraded` に `documents_not_indexed`** |
| `code` | どのコードか | `qualified_name` / `file` / `line` / `resolution` | 同上。**未解析なら `degraded` に `code_not_analyzed`** |
| `tests` | どう確かめるか | `existing` / `drafts` / `steps` / `data_boundaries` | 「既存ケースなし。草稿も作れない」を明示 |

### 4.1 `summary.headline` の作り方（決定論的）

AI に書かせない。テンプレートに件数を差し込む。

```text
{変更種別}です。画面 {s} 件、設計書 {d} 箇所、コード {c} 箇所、テスト {t} 件に波及します。
+ 打切りがある場合：「うち hub {h} 件は展開を打ち切りました。」
+ 未解析がある場合：「コード {u} 件は未解析のため含まれていません。」
```

変更種別は `input.kind` とシードの実体型から決める（`入力規則` / `画面項目` / `メソッド` / `テーブル` / `帳票` / `その他`）。**形容詞を足さない**（「重大な」など、根拠のない強調をしない）。

### 4.2 各項目の `why`

「なぜ影響するか」を 1 文で。関係型から機械的に作る。

| 関係 | `why` の型 |
|---|---|
| `VALIDATES` | 「{項目}の入力規則を直接持つ」 |
| `HANDLES` | 「{path} の処理を担当する」 |
| `SUBMITS_TO` | 「{form} からの送信先」 |
| `QUERIES` | 「{table} を参照する SQL を持つ」 |
| `CALLS`（INFERRED） | 「{name} への呼び出し候補（名前一致のみ。型未解決）」 |
| `DESCRIBES` | 「{実体}を記述する設計箇所」 |

`RESOLVED` と `INFERRED` を**同じ見た目で出さない**。`INFERRED` はバッジを付け、`why` に「候補」「未解決」を必ず含める。

---

## 5. 4 分類への投影アルゴリズム

ImpactEngine が返す `items[]`（`level` / `type` / `entity` / `path` / `resolution` / `confidence`）を、以下の規則で 4 分類に振り分ける。**新しい走査はしない**。

```text
for item in report.items:
    t = item.entity.kind
    if t in {Screen, Form, Page, Jsp, Html}:
        screens.add(item)
    elif t in {Document, Sheet, Section, Region}:
        documents.add(item)
    elif t in {Class, Method, Field, Sql, Table, Column, Config, Tag}:
        code.add(item)
    else:
        summary.other += 1        # 画面・文書・コードのどれでもない実体
```

| section | 追加条件 | 並び順 |
|---|---|---|
| `screens` | `entity.kind` が画面系、または `relation ∈ {HANDLES, DISPATCHES, SUBMITS_TO, RENDERS}` の終点 | `level` 昇順 → `confidence` 降順 → `natural_key` |
| `documents` | `entity.kind` が文書系、または証拠の `locator.kind ∈ {xlsx, pdf, docx}` | `doc_type` の業務順（要件→基本→詳細→画面→DB→テスト）→ `relevance` 降順 |
| `code` | `entity.kind` がコード系、または `relation ∈ {CALLS, IMPLEMENTS, EXTENDS, QUERIES, VALIDATES, READS, WRITES}` | `resolution`（RESOLVED → INFERRED → PARTIAL → UNRESOLVED）→ `confidence` 降順 |
| `tests` | §6 で編成 | 既存 → 草稿 → 手順 |

**重複排除**：同じ `entity_id` が複数経路で出たら 1 件にまとめ、`path` は最短、`why` は最も具体的な関係を採る。件数は**重複排除後**を `counts` に入れる（A-04 の `stats.total` と一致させる）。

**分類できない実体**（`other`）は捨てずに `summary.other` に件数だけ出す。「画面でも文書でもコードでもない影響が N 件ある」ことは誠実さの一部。

### 5.1 文書セクションのページング

60,000 冊前提。`documents` は**必ず**カーソルで返す。

- `limit` 既定 20、最大 100。
- `total` は重複排除後の全件数。`has_more` / `next_cursor` を返す。
- ソートは `doc_type` → `relevance` → `document_id`。`relevance` が同点でも順序が揺れないよう `document_id` でタイブレーク。
- 全文書をメモリに載せない。SQL は `project_id` + `is_current = true` + `deleted_at IS NULL` で絞る。

---

## 6. 「どうテストするか」の編成

テストは**提案**であり、実行ではない。実行側は `detail/09-test-runner.md`。ここでは 4 層で出す。

### 6.1 既存ケース（`existing`）

```sql
SELECT tc.id, tc.code, tc.title, tc.status
  FROM test.test_case tc
 WHERE tc.project_id = $1
   AND tc.covers_impacts && $2::uuid[]
   AND tc.status IN ('APPROVED','DRAFT')
 ORDER BY tc.status, tc.code;
```

`covers_impacts`（`uuid[]`、`detail/02-database-ddl.md`）を持たないケースは**出さない**。被覆の根拠がないケースを「既存テスト」と呼ばない。

### 6.2 草稿ケース（`drafts`）

ルール × 影響項目から、境界値の草稿を作る。`status = DRAFT` 固定。承認は人間（`test:approve`）。

| 生成元 | 作るケース |
|---|---|
| `validation_rule` の上限変更 | 新上限 −1 / 上限 / 上限 +1 |
| 全角・半角の規則 | 全角のみ / 半角混在 / 全角空格 |
| 外字・サロゲートペア | 外字 1 字 / サロゲート 2 字 / 結合文字 |
| 和暦・日付 | 明治・昭和・平成・令和の境界、うるう日 |
| 必須・任意の変更 | 未入力 / 空白のみ / 前後空白 |

草稿は**最大 20 件**。それ以上は「他 N 件の候補」と件数だけ出し、T-04 で詳細生成する（UI を埋め尽くさない）。

### 6.3 手順（`steps`）

Playwright で実行可能な粒度で書くが、**実行はしない**。

```text
1. ユーザ編集画面を開く
2. 住所に全角29文字を入力する
3. 登録を押す
4. エラーが出ないこと
5. 住所に全角31文字を入力し、エラーが出ること
```

各ステップは `test_case` の DSL（`steps[]`）と同じ語彙（`goto` / `fill` / `click` / `assert_text` / `field_error`）に合わせる。Q-01 で作った手順をそのまま T-03 に取り込めるようにする。

### 6.4 未カバー（`not_covered`）

「ルールが無いのでテストできない」ものを明示する。例：半角カナの扱いが設計書に無い、外字の許容が決まっていない。**沈黙して通さない**。

---

## 7. テスト計画への受け渡し

Q-01 から `POST /impacts/{iid}/test-plans:derive` を呼び、`drafts` を `test_plan` に落とす。**実行は別操作**。

```text
Q-01「テスト計画に入れる」→ 202 + {plan_id}
  → T-01 に DRAFT の計画が現れる
  → 人間が承認（T-02 / T-03）
  → T-05 で実行（test:execute + 環境指定）
```

- Q-01 は `test:write` が無いユーザーには「計画に入れる」を出さない（閲覧のみ）。
- 計画に入れても `test_run` は作らない。誤って CI を走らせない。
- 実行は正式環境では `X-Confirm-Action` + 承認（`06-API.md` §4.2）。

---

## 8. UI（Q-01）

`07-UI.md` §2.2 の Q-01。**1 画面・5 ブロック・折り畳み**。情報密度を上げすぎない。

```text
┌───────────────────────────────────────────────────────────────────────────────┐
│ 変更影響をたずねる                                      [履歴] [Markdown 出力] │
├───────────────────────────────────────────────────────────────────────────────┤
│ 例: 住所の桁数を全角20→30 / UserEditAction.update を変更 / 画面 SCR-USER-EDIT │
│ ┌─────────────────────────────────────────────────────────────────┐ [問合せる]│
│ │ 住所の入力桁数を全角20文字から30文字に変えたい                   │           │
│ └─────────────────────────────────────────────────────────────────┘           │
│ 対象: ● 自動判定   ○ 画面   ○ 文書   ○ コード       詳細: [QUICK|FULL]        │
├───────────────────────────────────────────────────────────────────────────────┤
│ ■ どんな影響か                                          リスク 中 (0.62)      │
│   入力規則の変更です。画面 3 件、設計書 7 箇所、コード 12 箇所、テスト 9 件。  │
│   うち hub 1 件は展開を打切り。未解析 2 件は含まれていません。                 │
│   [影響グラフで見る →]                                                        │
├───────────────────────────────────────────────────────────────────────────────┤
│ ▸ 影響する画面 (3)                                                             │
│   SCR-USER-EDIT ユーザ編集画面   L1  直接この項目を持つ      [原文 C12:F12]    │
│   SCR-USER-REG  ユーザ登録画面   L2  同じ住所項目を持つ      [原文 D08:D08]    │
│   SCR-USER-LIST ユーザ一覧画面   L3  編集画面から遷移        [原文 B02]        │
├───────────────────────────────────────────────────────────────────────────────┤
│ ▾ 影響する設計書 (7)                                              [もっと見る] │
│   UI_SPEC  ユーザ管理機能 画面設計書    画面項目定義 C12:F12   「全角20文字以内」│
│   DETAILED_DESIGN ユーザ管理機能 詳細設計書  §4.2 入力チェック  「20 桁」       │
├───────────────────────────────────────────────────────────────────────────────┤
│ ▸ 影響するコード (12)   RESOLVED 8 / INFERRED 3 / UNRESOLVED 1                 │
│   UserEditAction#validate   UserEditAction.java:142  RESOLVED  [該当行]        │
│   UserService#update        UserService.java:88       INFERRED  [候補]         │
├───────────────────────────────────────────────────────────────────────────────┤
│ ▸ どう確かめるか (既存 9 / 草稿 4)                                             │
│   既存: TC-2026-0118 住所に半角英字を入力した場合（承認済）                    │
│   追加: 全角29文字が通る / 全角31文字でエラー / 全角空格のみ / 外字1字         │
│   手順: 画面を開く → 29文字入力 → 登録 → エラーなし → 31文字 → エラー          │
│   [テスト計画に入れる]  ※実行はしません                                        │
├───────────────────────────────────────────────────────────────────────────────┤
│ 未カバー: 半角カナの扱い（設計に定義なし）                                     │
└───────────────────────────────────────────────────────────────────────────────┘
```

| 原則 | 内容 |
|---|---|
| 5 ブロック固定 | 順番を変えない。ユーザーが毎回同じ位置で読める |
| 折り畳み | `screens` 以外は既定で折り畳み。件数は見出しに出す |
| 空と失敗の区別 | 「0 件」と「未解析で出せない」を別の文言・色で出す |
| 証拠 | 各項目に `[原文]`。押すと K-06 ドロワーが該当 locator を開く |
| 実行しない | テストのボタンは「計画に入れる」まで。実行ボタンを置かない |
| 日本語 | 禁則処理、全角括弧、和暦、日文字体。エラーは 2 行目に `code` と `trace_id` |
| コピー | 見出し横に「この 5 ブロックをコピー」。チャット・チケットに貼れる |

---

## 9. 速度

| 段 | 目標 | 受入 | 備考 |
|---|---|---|---|
| シード解決（自然言語） | 300ms | 1.5s | RAG top 20 + entity_search top 20 + RRF |
| `summary` まで | 3s | 8s | ここが体感の勝負。先に出す |
| `screens` | 2s | 8s | 逆依存 3 層まで |
| `documents` | 4s | 20s | 証拠検索を並列 4 |
| `code` | 3s | 15s | Fact + symbol |
| `tests` | 2s | 10s | 既存 + 草稿生成 |
| `done` 全体（FULL） | **90s** | **120s** | `detail/03` §1 の影響分析と同じ上限 |
| `done` 全体（QUICK） | **8s** | **15s** | 2 ホップ、草稿なし |

**速度の作り方**：

- 5 セクションを**並列に走らせ、確定順に流す**。直列にすると `summary` が最後になる。
- `documents` の証拠検索は並列 4、top_k 20。全件取得しない。
- rerank は 1 回のバッチ（40 件）。セクションごとに呼ばない。
- 同じ `question` + 同じ revision の結果を 10 分キャッシュ。`rerun` は明示的にキャッシュを無視する。
- `QUICK` は深さ 2・hub 展開なし・草稿生成なし。**「まず QUICK、必要なら FULL」**を UI の既定動線にする。

---

## 10. 降格と失敗モード

| # | 失敗 | 検知 | 挙動 | 表示 |
|---|---|---|---|---|
| D-01 | グラフが落ちている | 接続失敗 | PG の `graph_meta` だけで 1 ホップに降格 | `degraded:["graph_unavailable"]`、`truncated.depth=true` |
| D-02 | 文書が未索引 | `kb.chunk` 0 件 | documents を空にし、理由を出す | `degraded:["documents_not_indexed"]` |
| D-03 | コード未解析 | `code.fact` 0 件 | code を空にし、理由を出す | `degraded:["code_not_analyzed"]` |
| D-04 | テスト資産なし | `test_case` 0 件 | tests は手順だけ | `degraded:["tests_not_available"]` |
| D-05 | rerank 不調 | 429 / timeout | RRF のみ | `degraded:["rerank_unavailable"]` |
| D-06 | シード 0 件 | 検索 0 件 | 実行しない | `NOT_FOUND_SEED` + 言い換え例 |
| D-07 | シード曖昧 | 確度 < 0.7 | 実行しない | `NEEDS_CLARIFICATION` + 候補 |
| D-08 | 予算超過 | `max_wall_time_sec` | コアだけ返す | `truncated.budget=true` |
| D-09 | AI 叙述だけ失敗 | Provider エラー | 叙述を出さない | `degraded:["ai_unavailable"]`（主表は無傷） |

**原則**：空配列を返して「影響なし」と誤読させない。**「出せない」と「無い」を必ず区別する。**

---

## 11. 段階提供（過剰な約束をしない）

| Phase | Q-01 でできること | できないこと |
|---|---|---|
| 5 | `summary` / `screens` / `documents` / `code` / 既存テスト。`QUICK` と `FULL` | 草稿ケース生成、AI 叙述 |
| 6 | `drafts` / `steps` / `data_boundaries`、テスト計画への受渡し | AI 叙述 |
| 7 | AI 叙述（`narrative` / `hypotheses[]`） | コード変更、テスト実行 |

> Phase 5 の受入では `tests` を「既存ケースのみ」と明記する。草稿が無いことを不具合と呼ばない。**Phase 5 のデモで「どうテストするか」を完全に見せたいなら、Phase 6 の一部を前倒しする**（`08-実装計画.md` §5 の前倒し表に追記）。

---

## 12. 受入テスト

| # | 手順 | 期待 |
|---|---|---|
| Q-01 | 「住所の桁数を 20→30」で問合せ | 5 ブロックが出る。`counts` と各 `total` が一致 |
| Q-02 | 同じ内容を 2 回（同じ `Idempotency-Key`） | 同一 `inquiry_id`、`impact_id` も同一、ジョブ 1 回 |
| Q-03 | 曖昧な質問「住所を直したい」 | `NEEDS_CLARIFICATION`。候補 2 件以上、`reason` 付き。**勝手に実行しない** |
| Q-04 | 存在しない対象「ZZZ 機能」 | `NOT_FOUND_SEED`。言い換え例を出す |
| Q-05 | 全角数字・全角空格・外字・和暦を含む質問 | シードが落ちない。`quote` は原文 NFC |
| Q-06 | 文書未投入のプロジェクトで実行 | `documents` 空 + `degraded:["documents_not_indexed"]`。「影響なし」と言わない |
| Q-07 | コード未解析で実行 | `code` 空 + `degraded:["code_not_analyzed"]` |
| Q-08 | 60,000 冊のプロジェクトで documents を取得 | 20 件 + `next_cursor`。P95 < 20s。全件を返さない |
| Q-09 | 途中で SSE を切断 → `Last-Event-ID` で再接続 | 欠番なし。`summary` を二重に出さない |
| Q-10 | 影響項目 200 件超（hub 含む） | `truncated.hubs > 0` が UI に出る。件数は重複排除後 |
| Q-11 | `test:write` 無しのユーザー | 「テスト計画に入れる」が非表示。閲覧は可能 |
| Q-12 | 同一質問を設計書更新後に `rerun` | 新しい `impact_id`。差分（増減）を比較できる |
| Q-13 | 5 ブロックの各項目を原文クリック | K-06 が正しい locator を開く。`quote` がハイライト |
| Q-14 | AI Provider 停止 | 主表 5 ブロックは完全。`degraded:["ai_unavailable"]` |

---

## 13. 他詳細設計との境目

| 境目 | 本稿 | 相手 |
|---|---|---|
| 影響の算出 | `profile=CHANGE_INQUIRY` を渡すだけ | `04-コアエンジン.md` §4.5 |
| グラフ走査 | 結果を受け取る | `detail/07-graph-projection.md` |
| Fact / 証拠の位置 | `quote` / `locator` を整形 | `detail/06-code-analysis-engine.md`、`detail/01-excel-ingestion.md` |
| テストケース生成 | 草稿の編成のみ | `04-コアエンジン.md` §4.6 |
| テストの**実行** | 一切やらない。計画に入れるまで | `detail/09-test-runner.md`（DSL / Runner / 被覆 / 欠陥） |
| API 共通規約 | 従う | `detail/04-api-contract.md` §0 |
| 画面 | Q-01 のワイヤー | `07-UI.md` §2.2 / §3 |
| 速度 | セクション別の上限 | `detail/03-performance.md` §1 |
