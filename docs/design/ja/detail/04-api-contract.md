# 詳細設計 04：API 契約（実装直結）

対象：Phase 1〜2 で Rust バックエンドとフロントが並行で動くための、決定的な契約。
`06-API.md` の一覧を補足するもので、**この文書がリクエスト/レスポンスの正**とする。

---

## 0. 共通約定

| 項目 | 約定 |
|---|---|
| バージョン | `/api/v1`。破壊的変更時のみ `/v2` |
| 認証 | `Authorization: Bearer <pat>` またはセッション Cookie |
| 認可 | エンドポイントごとの permission。404 を 403 にしない（存在自体を隠す） |
| 形式 | `application/json; charset=utf-8` |
| 時刻 | ISO 8601 UTC。`2026-11-05T12:00:00Z` |
| ID | 全て UUID v4 / v5。人間可読コード（`TC-2026-0118`）は別列 |
| ページング | **カーソルのみ**。`?limit=50&cursor=<opaque>`。最大 100 |
| 冪等 | 書き込みは `Idempotency-Key`。重複送信は同じ `job_id` / 同一結果 |
| 進捗 | 長い処理は `202 + {job_id}`。完了は SSE かポーリング |
| 追跡 | 全レスポンスに `X-Trace-Id` |
| 速度 | 一覧は `include` で列を絞る。既定はサマリのみ |

### 0.1 エラーエンベロープ

```json
{
  "type": "https://systivra.example/errors/VALIDATION_SCHEMA",
  "title": "Invalid request",
  "status": 400,
  "code": "VALIDATION_SCHEMA",
  "detail": "doc_type が不正です",
  "instance": "/api/v1/projects/8f2c.../documents",
  "trace_id": "4bf92f3577b34da6a3ce929d0e0e4736",
  "errors": [
    {"field": "doc_type", "code": "UNKNOWN_VALUE", "message": "doc_type は §0.3 の列挙値ではありません"}
  ]
}
```

| コード | status | 再試行 | 用途 |
|---|---|---|---|
| `VALIDATION_SCHEMA` | 400 | 不可 | 入力型・列挙値の誤り |
| `AUTH_TOKEN_EXPIRED` | 401 | 不可 | 再ログイン要求 |
| `FORBIDDEN_PROJECT_SCOPE` | 403 | 不可 | プロジェクト範囲外 |
| `NOT_FOUND_*` | 404 | 不可 | リソース不存在 |
| `CONFLICT_REVISION_SUPERSEDED` | 409 | 不可 | 改版の競合 |
| `CONFLICT_ALREADY_IMPORTED` | 409 | 不可 | 同一 content_hash |
| `CAPABILITY_UNSUPPORTED` | 409 | 不可 | Provider に能力なし |
| `PROVIDER_TIMEOUT` | 503 | 可 | 上流タイムアウト |
| `PROVIDER_RATE_LIMITED` | 429 | 可（Retry-After） | 上流 429 |
| `BUDGET_*` | 422 | 不可 | 予算超過 |
| `JOB_CANCELLED` | 410 | 不可 | ジョブ中止 |
| `EVIDENCE_QUOTE_MISMATCH` | 422 | 不可 | `quote` が原文の部分文字列でない（2.3） |
| `SCHEMA_VIOLATION` | 422 | 不可 | Agent 出力が `output_schema` に適合しない（6.1） |
| `AGENT_CAPABILITY_DENIED` | 403 | 不可 | 実効権限の積集合で能力が落ちた |
| `POLICY_APPROVAL_REQUIRED` | 423 | 不可 | 承認待ち |

> **エラーコードの 2 系列**：HTTP 境界の業務コードは上表（`VALIDATION_SCHEMA` など、接頭辞は `06-API.md` §1.1 の規約に従う）。Excel パーサー内部の `E-XLSX-001〜007`（`detail/01-excel-ingestion.md` §10）は **SSE / ジョブの `errors[]` にだけ現れる内部コード**であり、HTTP レスポンスの `code` には使わない。HTTP に上げる場合は `VALIDATION_SCHEMA` または `JOB_*` にラップし、`detail.code` に `E-XLSX-xxx` を残す。

### 0.2 ID 表記規約（**実装前に必ず読む**）

| 項目 | 規約 |
|---|---|
| リソース ID | **すべて UUID**（`id` / `*_id`）。DB の主キーと 1 対 1。例では `d1a2...` のように UUID を省略表記してよい |
| 表示用コード | `code` 列を持つ表（`core.workspace` / `core.project` / `test.test_plan` / `test.test_scenario` / `test.test_case` / `ai.agent` / `ai.workflow_def`）に限り、レスポンスで `code` を `id` と併記してよい。**API の参照は UUID で行い、`code` を ID として送らない** |
| 疑似 ID の禁止 | `code` 列の無い表（`kb.document` / `kb.document_revision` / `kb.chunk` / `kb.evidence` / `core.job` / `test.test_run` / `test.test_execution` / `ops.provider` / 影響項目）に `DOC-0012` / `CH-9a1c` / `IMP-2026-0007` のような接頭辞付き ID を使わない（**初期版の本稿がこれに該当していた**） |
| 非リソース ID | `search_id` / `trace_id` / SSE イベント `id` は ULID または単調整数でよい（表の主キーではないため） |
| 環境・Provider 名 | `environment.id`（`test.test_run.environment` 内の jsonb）と `ops.provider.impl_name` は論理名文字列。UUID ではないが、レスポンスの `id` フィールドとは混ぜない |

> **正は本節**。初期版で例示に混在していた接頭辞付き ID は全て UUID 省略表記に置き換えた。前後端ともに UUID を前提に型を生成する（`openapi-typescript`）。

**表示コードとの使い分け（他文書との境界）**：

- `07-UI.md` のワイヤーフレーム、`04-コアエンジン.md` のテストケース YAML、レポート見出しに出る `RUN-2026-0112` / `IMP-2026-0007` / `II-014` / `TC-2026-0118` は **`code`（人間可読の表示キー）** であって、DB 主キーでも API の `id` でもない。
- API レスポンスでは **`id`（UUID）と `code` を併記**する。画面は `code` を表示し、リンク・再取得には `id` を使う。
- DB の `jsonb` に埋め込む相互参照（被覆スナップショット、ImpactReport、Evidence）は **UUID で書く**。`code` を jsonb の参照キーにしない。

### 0.3 `doc_type` 列挙（正は本節）

```
REQUIREMENT / BASIC_DESIGN / DETAILED_DESIGN / UI_SPEC / DB_SPEC /
TEST_SPEC / API_SPEC / MANUAL / OTHER
```

- `kb.document.doc_type` と `documents:bulk-import` のマニフェスト、検索の `filters.doc_types` はすべてこの列挙を使う。
- `DETAILED_SPEC` は**存在しない**（`06-API.md` の旧例示から削除）。詳細設計書は `DETAILED_DESIGN`、画面項目定義は `UI_SPEC`。
- 未知の値は `VALIDATION_SCHEMA`。DB 側も `CHECK` で弾く（`detail/02-database-ddl.md`）。

---

## 1. Document

### 1.1 アップロード URL 発行

```http
POST /api/v1/projects/{pid}/documents:upload-url
Idempotency-Key: upload:user_edit.xlsx:18422
```

```json
{
  "filename": "user_edit.xlsx",
  "size_bytes": 2412356,
  "mime_type": "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
  "source_revision": "svn-18422"
}
```

```json
201
{
  "upload_url": "https://minio/.../presigned",
  "object_key": "raw/8f2c.../9a1b.../user_edit.xlsx",
  "expires_at": "2026-11-05T20:30:00Z",
  "max_size_bytes": 104857600
}
```

**備考**：`object_key` は次の 1.2 で渡す。この API は DB に document を書かない（アップロード未完了を避けるため）。

### 1.2 文書登録

```http
POST /api/v1/projects/{pid}/documents
```

```json
{
  "object_key": "raw/8f2c.../9a1b.../user_edit.xlsx",
  "doc_type": "DETAILED_DESIGN",
  "title": "ユーザ管理機能 詳細設計書",
  "business_domain": "user_management",
  "acl_groups": ["grp-dev"],
  "source_revision": "svn-18422"
}
```

```json
201
{
  "document": {
    "id": "d1a2...",
    "project_id": "8f2c...",
    "doc_type": "DETAILED_DESIGN",
    "title": "ユーザ管理機能 詳細設計書",
    "business_domain": "user_management",
    "acl_groups": ["grp-dev"],
    "current_revision": {
      "id": "r9c3...",
      "source_revision": "svn-18422",
      "content_hash": "sha256:...",
      "status": "CURRENT",
      "acquired_at": "2026-11-05T12:00:00Z"
    }
  },
  "next": {"parse": "/api/v1/documents/d1a2.../revisions/r9c3...:parse"}
}
```

**errors**：`CONFLICT_ALREADY_IMPORTED`（同一 `content_hash`）、`VALIDATION_SCHEMA`（`doc_type`）。

### 1.3 一括取込

```http
POST /api/v1/projects/{pid}/documents:bulk-import
```

```json
{
  "manifest_url": "s3://sep-manifests/2026-11-05.jsonl",
  "mode": "FULL",
  "on_duplicate": "SKIP"
}
```

```json
202
{
  "job_id": "j1...",
  "status": "QUEUED",
  "estimated_files": 60000,
  "links": {"events": "/api/v1/jobs/j1.../events"}
}
```

`mode`:
- `FULL`：全ファイル
- `INCREMENTAL`：`source_revision` が変わったものだけ
- `REINDEX`：解析済みを再チャンク（無視される対象は `skipped_*` に計上）

### 1.4 文書一覧

```http
GET /api/v1/projects/{pid}/documents
  ?doc_type=DETAILED_DESIGN
  &business_domain=user_management
  &status=CURRENT
  &q=住所
  &limit=50
  &cursor=eyJvIjoxMDAwfQ
  &include=summary
```

```json
200
{
  "items": [
    {
      "id": "d1a2...",
      "title": "ユーザ管理機能 詳細設計書",
      "doc_type": "DETAILED_DESIGN",
      "business_domain": "user_management",
      "current_revision_id": "r9c3...",
      "sheet_count": 8,
      "chunk_count": 200,
      "parse_status": "SUCCEEDED",
      "updated_at": "2026-11-05T12:00:00Z"
    }
  ],
  "next_cursor": "eyJvIjoyMDAwfQ",
  "has_more": true
}
```

**速度**：`include=summary` は `sheet_count` / `chunk_count` / `parse_status` のみ。`include=detail` を指定した場合のみ数式・領域数を返す。60,000 件の全フィールドを返さない。

### 1.5 構造木（遅延読み込み）

```http
GET /api/v1/documents/{did}/tree?revision_id=r9c3...&parent=ROOT&limit=100
```

```json
200
{
  "nodes": [
    {"id":"n1","kind":"SHEET","name":"画面項目定義","locator":{"sheet":"画面項目定義"},"child_count":42},
    {"id":"n2","kind":"REGION","name":"R12:C3-F23:C6","locator":{"range":"C12:F23"},"child_count":12}
  ],
  "next_cursor": "eyJvIjoxMDB9",
  "truncated": false
}
```

`ROOT` は最上位。`child_count > 0` のノードのみ次の呼び出しで開く。**全ノードを一度に返さない**。

### 1.6 解析開始

```http
POST /api/v1/documents/{did}/revisions/{rid}:parse
```

```json
202
{
  "job_id": "j2...",
  "status": "QUEUED",
  "mode": "FULL"
}
```

重複送信は `Idempotency-Key` を使えば同じ `job_id` を返す。

---

## 2. Knowledge

### 2.1 検索

```http
POST /api/v1/projects/{pid}/knowledge/search
```

```json
{
  "query": "住所チェックの桁数と文字種",
  "mode": "hybrid",
  "filters": {
    "doc_types": ["DETAILED_DESIGN", "UI_SPEC"],
    "business_domain": "user_management",
    "revision_id": null
  },
  "top_k": 12,
  "rerank": true,
  "graph_assist": true,
  "include": ["locator", "quote", "scores"]
}
```

```json
200
{
  "search_id": "SRH-01J9...",
  "lexical_mode": "BOTH",
  "degraded": [],
  "results": [
    {
      "chunk_id": "c4b1...",
      "document": {"id":"d1a2...","title":"ユーザ管理機能 詳細設計書","doc_type":"DETAILED_DESIGN"},
      "revision": {"id":"r9c3...","source_revision":"svn-18422"},
      "locator": {
        "kind":"xlsx","sheet":"画面項目定義","range":"C12:F12",
        "anchor_cell":"C12","merged_span":"C12:F12"
      },
      "quote": "項目名: 住所 / 入力形式: 全角20文字以内 / 必須: 必須",
      "score": 0.0312,
      "rerank_score": 0.87,
      "sources": ["vector","fulltext","graph"]
    }
  ],
  "total_candidates": 143,
  "latency_ms": 412
}
```

**速度**：`include` に `quote` を含めない場合、`quote` は空で返す（呼び出し側で 2.3 に取りに行く）。1,200 万 chunk 規模で `latency_ms` P95 は **受入上限 2.0s / 目標 1.2s**（`detail/03-performance.md` §1）。内部では 32〜64 分区へ並列 fan-out するが、これはクライアントに見せない。

**errors**：`CAPABILITY_UNSUPPORTED`（`rerank=true` で rerank なし）、`VALIDATION_SCHEMA`（`top_k > 100`）。

### 2.2 チャンク取得

```http
GET /api/v1/chunks/{cid}?include=neighbors
```

```json
200
{
  "chunk_id": "c4b1...",
  "chunk_type": "SHEET_REGION",
  "text": "【画面項目定義】項目名: 住所 ...",
  "locator": {"kind":"xlsx","sheet":"画面項目定義","range":"C12:F23"},
  "token_count": 210,
  "char_count": 480,
  "neighbors": {
    "prev": ["c4b0..."],
    "next": ["c4b2..."],
    "parent": "c3f0..."
  },
  "parse_run": {"parser":"xlsx-native","parser_version":"0.3.1"}
}
```

### 2.3 証拠取得

```http
GET /api/v1/evidence/{eid}
```

```json
200
{
  "evidence_id": "e7f3...",
  "source_type": "EXCEL_CELL_RANGE",
  "document_id": "d1a2...",
  "revision_id": "r9c3...",
  "locator": {"kind":"xlsx","sheet":"画面項目定義","range":"C12:F12"},
  "quote": "項目名: 住所",
  "quote_hash": "sha256:...",
  "context_before": "【画面項目定義】",
  "context_after": "項目名: 氏名",
  "resolution": "RESOLVED",
  "confidence": 1.0
}
```

**約束**：`quote` は原文の完全な部分文字列。サーバーが検証し、一致しなければ 500 を返さずに `EVIDENCE_QUOTE_MISMATCH`（500 系ではなく 422）を返す。

**表現規約**：DB の `quote_hash` は `bytea`（`sha256(NFC(quote))`）。JSON では **`sha256:` を付けた lowercase hex 64 文字**で返す。元のバイト列でも、NFKC 正規化後でもない（`05-データモデル.md` J-03）。`locator` の `sheet` / `range` は原文のまま、`quote` は NFC で保持する。

---

## 3. Job とイベント

### 3.1 ジョブ

```http
GET /api/v1/jobs/{jid}
```

```json
200
{
  "job_id": "j1...",
  "kind": "BULK_IMPORT",
  "status": "RUNNING",
  "progress": {
    "phase": "PARSE",
    "files_done": 1280,
    "files_total": 60000,
    "cells_done": 36864000,
    "cells_total": 1728000000,
    "chunks_done": 256000
  },
  "started_at": "2026-11-05T12:00:00Z",
  "eta_seconds": 36000,
  "errors": [],
  "links": {"events": "/api/v1/jobs/j1.../events"}
}
```

`progress` は 3 指標（ファイル / セル / chunk）を必ず持つ。ファイル数だけでは終盤に止まって見える（`detail/01` §11）。

### 3.2 SSE

```http
GET /api/v1/jobs/{jid}/events
Accept: text/event-stream
Last-Event-ID: 42
```

```text
id: 43
event: progress
data: {"phase":"PARSE","files_done":1281,"chunks_done":256200}

id: 44
event: partial
data: {"failed":[{"path":"/design/01/a.xlsx","code":"E-XLSX-003"}]}

id: 45
event: done
data: {"status":"SUCCEEDED_WITH_ERRORS","files_failed":3,"eta_seconds":0}
```

- `id` は単調増加。再接続時は `Last-Event-ID` から再送する。
- ハートビート：20 秒ごとに空行 `: ping`。プロキシの切断を防ぐ。
- `done` / `error` で終了。以降は接続を閉じる。

---

## 4. Impact

### 4.1 実行

```http
POST /api/v1/projects/{pid}/impacts
Idempotency-Key: impact:abc123..def456
```

```json
{
  "change_set": {"kind":"DIFF","base_revision":"abc123","head_revision":"def456"},
  "options": {"max_depth":6,"include_tests":true,"include_docs":true,"hub_threshold":200}
}
```

```json
202
{
  "impact_id": "i3d7...",
  "job_id": "j3...",
  "status": "QUEUED",
  "links": {
    "self": "/api/v1/impacts/i3d7...",
    "events": "/api/v1/jobs/j3.../events"
  }
}
```

### 4.2 結果サマリ

```http
GET /api/v1/impacts/{iid}
```

```json
200
{
  "impact_id": "i3d7...",
  "status": "SUCCEEDED",
  "base_revision": "abc123",
  "head_revision": "def456",
  "stats": {
    "total": 37,
    "by_level": {"L1":4,"L2":18,"L3":9,"L4":4,"L5":2},
    "resolved": 31, "inferred": 5, "unresolved": 1
  },
  "risk": {
    "score": 0.62,
    "level": "MEDIUM",
    "factors": [
      {"name":"impact_breadth","weight":0.20,"value":0.74},
      {"name":"uncertainty_ratio","weight":0.10,"value":0.16}
    ]
  },
  "truncated": {"hubs":1,"depth":false,"budget":false},
  "engine_version": "impact-0.3.1",
  "ontology_version": "v1.4"
}
```

### 4.3 影響項目

```http
GET /api/v1/impacts/{iid}/items?level=L2&resolution=INFERRED&limit=50&cursor=...
```

```json
200
{
  "items": [
    {
      "item_id": "ii01...",
      "level": "L2",
      "type": "CALLER",
      "direction": "upstream",
      "entity": {"id":"e009...","kind":"Class","name":"UserBatchService"},
      "path": ["e001...","e005...","e009..."],
      "path_edges": [
        {"rel":"CALLS","resolution":"RESOLVED","evidence":"e011..."},
        {"rel":"CALLS","resolution":"INFERRED","evidence":"e012..."}
      ],
      "confidence": 0.92,
      "reason": "間接呼び出し：Batch が CommonValidator 経由で再利用",
      "risk": "medium",
      "related_tests": ["tc01...","tc18..."]
    }
  ],
  "next_cursor": null,
  "has_more": false
}
```

`path` は短い順。`path_edges` は `path` の隣接関係と 1 対 1。

### 4.4 変更影響の単独問合せ

`POST /api/v1/projects/{pid}/change-inquiries` は、`POST /impacts` と同じ ImpactReport を **5 セクションに投影**して返す。`impact_id` を返すので、掘り下げは `/impacts/{iid}` と同一。

| 項目 | 契約 |
|---|---|
| 入力 | `question`（原文、≤2,000 字）+ `input.kind`（`NATURAL_LANGUAGE` / `DIFF` / `ENTITY`）+ `mode`（`QUICK` / `FULL`） |
| 応答 | `202 + {inquiry_id, impact_id, job_id, links}` |
| 進捗 | `202` のジョブ SSE。`partial` の `section` は `summary → screens → documents → code → tests` の順 |
| 取得 | `GET /change-inquiries/{qid}?section=…&cursor=&limit=20`。`documents` は必ずカーソル |
| 冪等 | `Idempotency-Key` 内容ハッシュ。重複送信は同一 `inquiry_id` / `impact_id` |
| 曖昧 | シード確度 < 0.7 は `NEEDS_CLARIFICATION`（200 で候補を返す）。実行しない |
| 失敗 | シード 0 件は `NOT_FOUND_SEED`（404）。未索引・未解析は 200 + `degraded[]` |
| 速度 | `summary` まで受入 8s / 目標 3s。全体は受入 120s / 目標 90s。`QUICK` は受入 15s |

**セクションの必須フィールド**：`items[].why`（なぜ影響するか 1 文）と `quote` + `locator` + `evidence_id`。証拠のない項目は主表に入れず、`unverified` に分ける。

> 完全な契約・4 分類の投影規則・降格・受入 14 ケースは `detail/08-change-inquiry.md`。

---

## 5. Test

### 5.1 実行

```http
POST /api/v1/projects/{pid}/test-runs
```

```json
{
  "name": "影響分析 i3d7 の回帰",
  "case_ids": ["tc18...","tc19..."],
  "environment": {"id":"env-stg-01","base_url":"https://stg.example.internal"},
  "runner": {"type":"playwright","browser":"chromium","headless":true},
  "retry": {"max":1,"on":["TIMEOUT","NETWORK"]}
}
```

> `case_ids` は **UUID**。`TC-2026-0118` のような `test.test_case.code` は UI 表示用で、API の参照には使わない（§0.2）。`case_count` は `case_ids` の件数と一致しなければならない。`environment.id` は jsonb 内の論理名で UUID ではない（§0.2 の例外）。

```json
202
{
  "test_run_id": "t5e6...",
  "status": "QUEUED",
  "case_count": 2,
  "estimated_duration_sec": 40,
  "links": {"events": "/api/v1/jobs/j4.../events"}
}
```

### 5.2 実行結果

```http
GET /api/v1/test-executions/{eid}
```

```json
200
{
  "execution_id": "x8e2...",
  "test_case_id": "tc18...",
  "status": "FAILED",
  "failure_kind": "ASSERTION",
  "duration_ms": 8420,
  "step_results": [
    {"index":1,"action":"goto","status":"PASSED"},
    {"index":4,"action":"assert_text","status":"FAILED",
     "expected":"住所は全角20文字以内で入力してください",
     "actual":"住所を入力してください"}
  ],
  "artifacts": {
    "screenshot": "s3://sep-artifacts/.../failure.png",
    "trace": "s3://sep-artifacts/.../trace.zip",
    "console_log": "s3://sep-artifacts/.../console.log"
  }
}
```

---

## 6. Agent / Provider

### 6.1 Agent 実行

```http
POST /api/v1/agents/{aid}/runs
```

```json
{
  "input": {"impact_id":"i3d7..."},
  "budgets": {"max_tool_calls":40,"max_wall_time_sec":300}
}
```

```json
202
{
  "run_id": "ar01...",
  "status": "RUNNING",
  "links": {"events": "/api/v1/jobs/j5.../events"}
}
```

`done` イベントで `output` を返す。`output` は `output_schema` に適合する。不適合なら `SCHEMA_VIOLATION`。

### 6.2 Provider 切替

```http
PATCH /api/v1/providers/{id}
```

```json
{"enabled": false, "priority": 20}
```

```json
200
{
  "provider": {"id":"p7c2...","impl_name":"native-rag","enabled":false,"priority":20},
  "effective_after": "2026-11-05T12:00:05Z",
  "in_flight_requests": 3
}
```

`in_flight_requests` は切替時に処理中リクエストが残っていることを示す。即時切断はしない。

---

## 7. 実装メモ（速度と安全）

| 約定 | 理由 |
|---|---|
| 一覧は `include` で絞る | 60,000 件の詳細を一度に返さない |
| カーソルは不透明 | 実装変更でクライアントを壊さない |
| `Idempotency-Key` は内容ハッシュ + キー | 重複送信で 2 度解析しない |
| SSE は `id` 付き | 途中切断から再開できる |
| `quote` は常に検証 | AI の引用捏造を防ぐ（P2） |
| 202 + job を標準に | 長い処理で HTTP 接続を握らせない |
| `X-Trace-Id` は全レスポンス | 遅いときの第一歩（`detail/03` §7） |
| `project_id` は**リクエスト本文・クエリで受けず** ctx から注入 | 越権防止（04.7）。パスの `{pid}` は「スコープ宣言」であり、セッションの許可範囲と一致しない場合は 404。一致して初めて ctx の `project_id` として採用する。本文に `project_id` があれば破棄し、乖離していれば `VALIDATION_SCHEMA` |

### 7.1 未確定フィールド

実装時に決めるが、契約として固定してよい：

| 項目 | 候補 |
|---|---|
| カーソル符号化 | base64(JSON `{"o":<offset>,"k":[...]}`) |
| `latency_ms` の定義 | サーバー側処理時間（キュー待ちを除く） |
| `degraded` の値 | `rerank_unavailable` / `graph_assist_off` / `partial_index` |
| 429 の `Retry-After` | 秒。上流 429 をそのまま転送しない |

---

## 8. チェックリスト（実装前に確認）

- [ ] 1.4 の `include` でレスポンスサイズを制御できる
- [ ] 1.5 はノード単位で遅延読み込み。1 呼び出しで 1,000 ノード超を返さない
- [ ] 2.1 は `latency_ms` を返し、P95 をログに残す
- [ ] 3.2 は `Last-Event-ID` で再開できる
- [ ] 4.1 / 5.1 は 202 で返し、完了はイベントで知らせる
- [ ] 全エンドポイントで 404 と 403 を混同しない
- [ ] `quote` の部分文字列検証が 2.3 で動く
