# 第 6 部：API 設計

## 1. 共通規約

| 項目 | 規約 |
|---|---|
| 基底パス | `/api/v1`。破壊的変更時のみ版を上げる |
| Content-Type | `application/json; charset=utf-8`（アップロードは presigned URL。API プロセスを通さない） |
| 認証 | ブラウザは `HttpOnly + SameSite=Lax` のセッション Cookie。プログラムは `Authorization: Bearer <pat>`（不透明トークン。ハッシュ保存、即時失効可） |
| 認可 | RBAC + プロジェクト範囲。エンドポイントごとに必要 permission を宣言し、ミドルウェアで検証 |
| エラー | RFC 9457 `application/problem+json` + 業務 `code` |
| ページング | カーソル式 `?cursor=&limit=50`。`next_cursor` を返す。offset は使わない（深いページで性能劣化・漂流） |
| 冪等 | 書き込みは `Idempotency-Key` を受ける。未指定でも「分析/実行の作成」は内容ダイジェストで生成 |
| 非同期 | 時間のかかる操作は `202 + {job_id, links}`。進捗は `GET /jobs/{id}` または SSE |
| ストリーム | `text/event-stream`。イベントは `progress / partial / done / error` |
| 追跡 | `X-Trace-Id`。OTel と一致。エラー本文にも含める |
| 時刻 | ISO 8601 UTC（`2026-11-05T12:00:00Z`）。表示は locale |
| 命名 | リソースは複数名詞。動作は子リソースか `:verb`（CRUD で表せない場合のみ） |

### 1.1 エラーモデル

```json
{
  "type": "https://systivra.example/errors/CAPABILITY_UNSUPPORTED",
  "title": "Capability not supported by provider",
  "status": 409,
  "code": "CAPABILITY_UNSUPPORTED",
  "detail": "Provider 'external-rag-a' does not support 'rerank'. Configure a fallback provider.",
  "instance": "/api/v1/projects/8f2.../knowledge/search",
  "trace_id": "4bf92f3577b34da6a3ce929d0e0e4736",
  "errors": [
    {"field": "mode", "code": "UNSUPPORTED_VALUE", "message": "hybrid is not available"}
  ]
}
```

| 接頭辞 | 意味 | 例 |
|---|---|---|
| `AUTH_` | 認証 | `AUTH_TOKEN_EXPIRED` |
| `FORBIDDEN_` | 認可 | `FORBIDDEN_PROJECT_SCOPE` |
| `VALIDATION_` | 入力 | `VALIDATION_SCHEMA` |
| `NOT_FOUND_` | 資源 | `NOT_FOUND_DOCUMENT` |
| `CONFLICT_` | 状態競合 | `CONFLICT_REVISION_SUPERSEDED` |
| `CAPABILITY_` | 能力不可 | `CAPABILITY_UNSUPPORTED` |
| `PROVIDER_` | 上流障害 | `PROVIDER_TIMEOUT` |
| `BUDGET_` | 上限超過 | `BUDGET_TOKENS_EXCEEDED` |
| `JOB_` | ジョブ | `JOB_CANCELLED` |
| `POLICY_` | 方針による拒否 | `POLICY_APPROVAL_REQUIRED` |

`PROVIDER_*` かつ `retryable=true` の場合は `Retry-After` を付ける。

---

## 2. エンドポイント一覧

### 2.1 Workspace / Project

| メソッド | パス | 説明 | 権限 |
|---|---|---|---|
| GET | `/workspaces` | 一覧 | `workspace:read` |
| POST | `/workspaces` | 作成 | `system:admin` |
| GET | `/projects?workspace_id=` | 一覧 | `project:read` |
| POST | `/workspaces/{wid}/projects` | 作成 | `project:create` |
| GET | `/projects/{pid}` | 詳細（統計含む） | `project:read` |
| PATCH | `/projects/{pid}` | 設定変更 | `project:admin` |
| GET | `/projects/{pid}/overview` | ホーム集約 | `project:read` |

### 2.2 Document / Knowledge

| メソッド | パス | 説明 | 権限 |
|---|---|---|---|
| POST | `/projects/{pid}/documents:upload-url` | presigned URL 発行 | `document:write` |
| POST | `/projects/{pid}/documents` | 登録（アップロード後） | `document:write` |
| GET | `/projects/{pid}/documents` | 一覧（type/domain/status） | `document:read` |
| GET | `/documents/{did}` | 詳細 + 現行改版 | `document:read` |
| GET | `/documents/{did}/revisions` | 改版履歴 | `document:read` |
| POST | `/documents/{did}/revisions/{rid}:parse` | 解析開始（非同期） | `document:write` |
| GET | `/documents/{did}/tree?revision_id=` | 構造木 | `document:read` |
| GET | `/chunks/{cid}` | chunk 詳細 + 前後 | `document:read` |
| DELETE | `/documents/{did}` | 論理削除 + 索引失効 | `document:delete` |
| POST | `/projects/{pid}/knowledge/search` | 検索 | `knowledge:read` |
| POST | `/projects/{pid}/knowledge/index-builds` | 索引再構築 | `knowledge:admin` |
| GET | `/evidence/{eid}` | 証拠原文 + 文脈 | `knowledge:read` |

### 2.3 Graph / Ontology

| メソッド | パス | 説明 | 権限 |
|---|---|---|---|
| POST | `/projects/{pid}/graph/query` | 実体 / 隣接 | `graph:read` |
| GET | `/projects/{pid}/graph/entities/{eid}` | 実体詳細 + 次数 | `graph:read` |
| POST | `/projects/{pid}/graph/paths` | 経路 | `graph:read` |
| POST | `/projects/{pid}/graph/subgraph` | サブグラフ出力（上限 5,000） | `graph:read` |
| GET | `/projects/{pid}/ontology` | 現行 ontology | `graph:read` |
| POST | `/projects/{pid}/ontology/versions` | 新版公開 | `ontology:admin` + 承認 |
| GET | `/projects/{pid}/traceability` | トレーサビリティ | `graph:read` |

### 2.4 Code Analysis

| メソッド | パス | 説明 | 権限 |
|---|---|---|---|
| POST | `/projects/{pid}/code/repositories` | リポジトリ紐付け | `code:admin` |
| POST | `/projects/{pid}/code/revisions:sync` | 版同期（非同期） | `code:write` |
| GET | `/projects/{pid}/code/revisions` | 版一覧 | `code:read` |
| POST | `/projects/{pid}/code/analysis-runs` | 解析開始（非同期） | `code:write` |
| GET | `/code/analysis-runs/{id}` | 実行状態と統計 | `code:read` |
| GET | `/projects/{pid}/code/symbols?fqn=` | シンボル | `code:read` |
| GET | `/projects/{pid}/code/facts?subject=&relation=` | Fact | `code:read` |

### 2.5 Impact

| メソッド | パス | 説明 | 権限 |
|---|---|---|---|
| POST | `/projects/{pid}/changes` | 変更セット作成 | `impact:write` |
| GET | `/changes/{cid}` | 変更セット詳細 | `impact:read` |
| POST | `/projects/{pid}/impacts` | 影響分析（非同期） | `impact:run` |
| GET | `/impacts/{iid}` | 概要 + 統計 | `impact:read` |
| GET | `/impacts/{iid}/items?level=&resolution=` | 影響項目 | `impact:read` |
| GET | `/impacts/{iid}/graph` | グラフデータ（Cytoscape 形式） | `impact:read` |
| POST | `/impacts/{iid}/reports` | Markdown/PDF 生成 | `impact:read` |

### 2.6 Test

| メソッド | パス | 説明 | 権限 |
|---|---|---|---|
| POST | `/impacts/{iid}/test-plans:derive` | 影響から計画を派生 | `test:write` |
| GET | `/test-plans/{tid}` | 計画詳細（シナリオ木） | `test:read` |
| POST | `/test-scenarios/{sid}/cases:generate` | ケース派生（草稿） | `test:write` |
| POST | `/test-cases/{cid}:approve` | 草稿承認 | `test:approve` |
| POST | `/projects/{pid}/test-runs` | 実行作成（非同期） | `test:execute` |
| GET | `/test-runs/{rid}` | 進捗と結果 | `test:read` |
| GET | `/test-executions/{eid}` | 単回実行（成果物リンク） | `test:read` |
| GET | `/projects/{pid}/coverage?test_run_id=` | 被覆スナップショット | `test:read` |
| POST | `/test-executions/{eid}/defects` | 欠陥登録 | `test:write` |

### 2.7 AI / Workflow

| メソッド | パス | 説明 | 権限 |
|---|---|---|---|
| GET | `/agents` | 一覧 | `ai:read` |
| POST | `/agents/{aid}/runs` | 実行（非同期 / ストリーム） | `ai:execute` |
| GET | `/agent-runs/{rid}` | 詳細 + ステップ | `ai:read` |
| POST | `/agent-runs/{rid}:cancel` | キャンセル | `ai:execute` |
| GET | `/approvals?status=pending` | 承認待ち | `approval:read` |
| POST | `/approvals/{id}:decide` | 承認 / 却下 | `approval:decide` |
| GET | `/workflows` | 一覧 | `workflow:read` |
| POST | `/workflows/{wid}/runs` | 起動 | `workflow:execute` |

### 2.8 Platform

| メソッド | パス | 説明 | 権限 |
|---|---|---|---|
| GET | `/providers` | 一覧 + 能力マトリクス | `platform:read` |
| POST | `/providers` | 登録 | `platform:admin` |
| PATCH | `/providers/{id}` | 有効化 / 優先度 | `platform:admin` |
| POST | `/providers/{id}:probe` | 能力プローブ | `platform:admin` |
| GET | `/capabilities` | 能力登録簿 | `platform:read` |
| GET | `/mcp/servers` | MCP Server 一覧 | `platform:read` |
| POST | `/mcp/servers` | 登録 | `platform:admin` |
| POST | `/mcp/servers/{id}:discover` | ツール再検出 | `platform:admin` |
| GET | `/audit-logs?actor=&action=&from=` | 監査照会 | `audit:read` |
| GET | `/jobs/{jid}` | ジョブ状態 | 資源権限を継承 |
| POST | `/jobs/{jid}:cancel` | キャンセル | 資源権限を継承 |
| GET | `/jobs/{jid}/events` | SSE 進捗 | 資源権限を継承 |

---

## 3. 詳細例

### 3.1 検索

```http
POST /api/v1/projects/8f2c.../knowledge/search
Authorization: Bearer <pat>
Content-Type: application/json

{
  "query": "住所チェックの桁数と文字種",
  "mode": "hybrid",
  "filters": {
    "doc_types": ["DETAILED_DESIGN", "DETAILED_SPEC"],
    "business_domain": "user_management",
    "revision_id": null
  },
  "top_k": 12,
  "rerank": true,
  "graph_assist": true
}
```

```json
{
  "search_id": "SRH-01J9...",
  "degraded": [],
  "lexical_mode": "BOTH",
  "results": [{
    "chunk_id": "CH-9a1c...",
    "document": {"id": "DOC-0012", "title": "ユーザ管理機能 詳細設計書", "doc_type": "DETAILED_DESIGN"},
    "revision": {"id": "REV-0007", "source_revision": "svn-18422"},
    "locator": {"kind": "xlsx", "sheet": "画面項目定義", "range": "C12:F12"},
    "score": 0.0312,
    "sources": ["vector", "fulltext", "graph"],
    "rerank_score": 0.87,
    "quote": "項目名: 住所 / 入力形式: 全角20文字以内 / 必須: 必須"
  }],
  "total_candidates": 143,
  "latency_ms": 186
}
```

`sources` は「なぜ上位か」を利用者に示す。`degraded` は降格の明示。

### 3.2 影響分析

```http
POST /api/v1/projects/8f2c.../impacts
Idempotency-Key: sha256:9f1c...
Content-Type: application/json

{
  "change_set": {"kind": "DIFF", "base_revision": "abc123", "head_revision": "def456"},
  "options": {"max_depth": 6, "include_tests": true, "include_docs": true, "hub_threshold": 200}
}
```

```json
{
  "job_id": "JOB-01J9Z...",
  "impact_id": "IMP-2026-0007",
  "status": "QUEUED",
  "links": {
    "self": "/api/v1/impacts/IMP-2026-0007",
    "job": "/api/v1/jobs/JOB-01J9Z...",
    "events": "/api/v1/jobs/JOB-01J9Z.../events"
  }
}
```

SSE：

```text
event: progress
data: {"phase":"TRAVERSE","done":1240,"total":null,"message":"depth 3"}

event: progress
data: {"phase":"EVIDENCE","done":18,"total":37,"message":"matching documents"}

event: done
data: {"impact_id":"IMP-2026-0007","stats":{"total":37,"unresolved":1},"truncated":{"hubs":1}}
```

### 3.3 テスト実行

```http
POST /api/v1/projects/8f2c.../test-runs
Content-Type: application/json

{
  "name": "IMP-2026-0007 回帰",
  "case_ids": ["TC-2026-0118", "TC-2026-0119"],
  "environment": {"id": "env-stg-01", "base_url": "https://stg.example.internal"},
  "runner": {"type": "playwright", "browser": "chromium", "headless": true},
  "retry": {"max": 1, "on": ["TIMEOUT", "NETWORK"]}
}
```

```json
{"test_run_id": "RUN-2026-0112", "status": "QUEUED",
 "case_count": 23, "estimated_duration_sec": 420}
```

単回結果：

```json
{
  "execution_id": "EXE-01JA...",
  "test_case_id": "TC-2026-0118",
  "status": "FAILED",
  "failure_kind": "ASSERTION",
  "duration_ms": 8420,
  "step_results": [
    {"index": 1, "action": "goto", "status": "PASSED"},
    {"index": 4, "action": "assert_text", "status": "FAILED",
     "expected": "住所は全角20文字以内で入力してください",
     "actual": "住所を入力してください"}
  ],
  "artifacts": {
    "screenshot": "s3://sep-artifacts/.../failure.png",
    "trace": "s3://sep-artifacts/.../trace.zip",
    "console_log": "s3://sep-artifacts/.../console.log"
  }
}
```

---

## 4. 権限モデル

### 4.1 ロール × 行為

| 行為 | OWNER | ARCHITECT | DEVELOPER | TESTER | VIEWER |
|---|---|---|---|---|---|
| プロジェクト設定 | 書 | 読 | 読 | 読 | 読 |
| 文書取込 / 削除 | 書 | 書 | 書 | 読 | 読 |
| 検索 / 証拠 | 読 | 読 | 読 | 読 | 読 |
| コード同期 / 解析 | 書 | 書 | 書 | 読 | 読 |
| 影響分析実行 | 書 | 書 | 書 | 読 | 読 |
| テスト資産編集 | 書 | 書 | 読 | 書 | 読 |
| テスト実行 | 書 | 書 | 読 | 書 | 読 |
| ケース承認 | 書 | 書 | — | 書 | — |
| Ontology 公開 | 書 | 承認 | — | — | — |
| Agent 実行 | 書 | 書 | 書 | 書 | — |
| Provider / MCP 管理 | 書 | — | — | — | — |
| 監査照会 | 書 | 読 | — | — | — |

### 4.2 データ水準の制約

1. **プロジェクト分離**：全クエリに `project_id` を強制注入。フロントから来た値を信頼しない。
2. **文書 ACL**：`document.acl_groups` とユーザーグループの積集合。交わらなければ検索結果に出さない（SQL 層でフィルタ）。
3. **Agent の降格**：有効権限 = `agent.capabilities ∩ 起動ユーザー権限 ∩ プロジェクト範囲`。
4. **重要操作の二段確認**：プロジェクト削除、Ontology 公開、正式環境テストは `X-Confirm-Action: <resource_id>` + 承認フロー。

### 4.3 監査

```json
{
  "actor_type": "USER",
  "actor_id": "u-1024",
  "action": "impact.run",
  "target_type": "project",
  "target_id": "8f2c...",
  "input_digest": "sha256:...",
  "result": "ACCEPTED",
  "detail": {"change_set": "abc123..def456", "job_id": "JOB-..."},
  "trace_id": "..."
}
```

Agent 起点は `agent_run_id` と `capability` を追加。監査ログは更新・削除不可（DB 層で REVOKE）。

---

## 5. OpenAPI と契約テスト

- `utoipa` で Rust 型から OpenAPI 3.1 を生成する。**手書き YAML を正にしない**（実装と乖離する）。
- CI で生成 spec と `contracts/api/openapi.json` の一致を検査。不一致は明示コミットを要求。
- フロント型は OpenAPI から生成（`openapi-typescript`）。重複定義を書かない。
- 破壊的変更は `oasdiff` で検出し、人手承認を必須にする。
