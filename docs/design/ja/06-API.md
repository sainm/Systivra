# 第 6 部：API 設計

> リクエスト/レスポンスの詳細な契約は `detail/04-api-contract.md` を正とする。本稿は一覧と規約の全体像を示す。**ID 表記規約（全リソース UUID、`code` は表示用）は `detail/04` §0.2、`doc_type` 列挙は同 §0.3 が正**。

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
| `AGENT_` | Agent 権限 | `AGENT_CAPABILITY_DENIED` |
| `EVIDENCE_` | 証拠の整合 | `EVIDENCE_QUOTE_MISMATCH` |
| `SCHEMA_` | 出力 Schema 検証 | `SCHEMA_VIOLATION` |

`CAPABILITY_NOT_FOUND` / `CAPABILITY_UNSUPPORTED` は `CAPABILITY_` に含まれる。全コードの status / 再試行可否は `detail/04-api-contract.md` §0.1 を正とする。

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
| POST | `/projects/{pid}/documents:bulk-import` | マニフェストによる一括取込（非同期。部分成功） | `document:write` |
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
| POST | `/projects/{pid}/change-inquiries` | **変更影響の単独問合せ**（非同期 + 分割 SSE） | `impact:run` |
| GET | `/change-inquiries/{qid}` | 5 セクションの取得（section ごとにカーソル） | `impact:read` |
| GET | `/projects/{pid}/change-inquiries` | 自分の問合せ履歴 | `impact:read` |
| POST | `/change-inquiries/{qid}:rerun` | 同じ質問で再実行（新しい `impact_id`） | `impact:run` |
| GET | `/change-inquiries/{qid}/report.md` | Markdown 出力（共有・チケット添付） | `impact:read` |

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
    "doc_types": ["DETAILED_DESIGN", "UI_SPEC"],
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
    "chunk_id": "c4b1...",
    "document": {"id": "d1a2...", "title": "ユーザ管理機能 詳細設計書", "doc_type": "DETAILED_DESIGN"},
    "revision": {"id": "r9c3...", "source_revision": "svn-18422"},
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
  "job_id": "j3d9...",
  "impact_id": "i3d7...",
  "status": "QUEUED",
  "links": {
    "self": "/api/v1/impacts/i3d7...",
    "job": "/api/v1/jobs/j3d9...",
    "events": "/api/v1/jobs/j3d9.../events"
  }
}
```

SSE：

```text
id: 41
event: progress
data: {"phase":"TRAVERSE","done":1240,"total":null,"message":"depth 3"}

id: 42
event: progress
data: {"phase":"EVIDENCE","done":18,"total":37,"message":"matching documents"}

id: 43
event: done
data: {"impact_id":"i3d7...","stats":{"total":37,"unresolved":1},"truncated":{"hubs":1}}
```

> 全イベントに単調増加の `id` を付け、再接続は `Last-Event-ID` で再開する。ハートビート `: ping` は 20 秒ごと。詳細は `detail/04-api-contract.md` §3.2。

### 3.3 テスト実行

```http
POST /api/v1/projects/8f2c.../test-runs
Content-Type: application/json

{
  "name": "影響分析 i3d7 の回帰",
  "case_ids": ["tc18...", "tc19..."],
  "environment": {"id": "env-stg-01", "base_url": "https://stg.example.internal"},
  "runner": {"type": "playwright", "browser": "chromium", "headless": true},
  "retry": {"max": 1, "on": ["TIMEOUT", "NETWORK"]}
}
```

```json
{"test_run_id": "t5e6...", "status": "QUEUED",
 "case_count": 2, "estimated_duration_sec": 40}
```

単回結果：

```json
{
  "execution_id": "x8e2...",
  "test_case_id": "tc18...",
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

## 3.4 変更影響の単独問合せ（Q-01）

「XX を変更したら何が起きるか」を 1 回の問いで、**概要 / 画面 / 文書 / コード / テスト**の 5 セクションに分けて返す。中身は `POST /projects/{pid}/impacts` と**同じ ImpactReport** であり、`impact_id` を返すので A-04 / A-05 へ遷移できる。

```http
POST /api/v1/projects/8f2c.../change-inquiries
Idempotency-Key: sha256:9f1c...
Content-Type: application/json

{
  "question": "住所の入力桁数を全角20文字から30文字に変えたい",
  "input": {"kind": "NATURAL_LANGUAGE"},
  "mode": "FULL",
  "include": ["screens", "documents", "code", "tests", "evidence"]
}
```

```json
202
{
  "inquiry_id": "q7b2...",
  "impact_id": "i3d7...",
  "job_id": "j6...",
  "status": "QUEUED",
  "links": {
    "self": "/api/v1/change-inquiries/q7b2...",
    "events": "/api/v1/jobs/j6.../events"
  }
}
```

分割 SSE（`summary` を最初に出す）：

```text
id: 52
event: partial
data: {"section":"summary","data":{"risk":{"score":0.62,"level":"MEDIUM"},"counts":{"screens":3,"documents":7,"code":12,"tests":9},"degraded":[]}}

id: 53
event: partial
data: {"section":"screens","data":{"items":[],"total":3,"has_more":false}}

id: 57
event: done
data: {"inquiry_id":"q7b2...","impact_id":"i3d7...","status":"SUCCEEDED","latency_ms":8420}
```

| 約束 | 内容 |
|---|---|
| 推測しない | シードが曖昧なら `NEEDS_CLARIFICATION` を返し、候補と `reason` を出す。勝手に 1 つに絞らない |
| 空と失敗を区別 | 未索引・未解析は `degraded[]` に理由を出す。「影響なし」と誤読させない |
| テストは提案まで | 「計画に入れる」は `POST /impacts/{iid}/test-plans:derive`。**実行は別操作**（D-04） |
| 速度 | `summary` まで 8s 以内（目標 3s）、全体は受入 120s / 目標 90s。`QUICK` は受入 15s（`detail/08` §9） |

> リクエスト/レスポンスの完全な契約、4 分類への投影、降格、受入 14 ケースは `detail/08-change-inquiry.md` を正とする。

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
| 変更影響問合せ（Q-01） | 書 | 書 | 書 | 書 | 読 |
| テスト資産編集 | 書 | 書 | 読 | 書 | 読 |
| テスト実行 | 書 | 書 | 読 | 書 | 読 |
| ケース承認 | 書 | 書 | — | 書 | — |
| Ontology 公開 | 書 | 承認 | — | — | — |
| Agent 実行 | 書 | 書 | 書 | 書 | — |
| Provider / MCP 管理 | 書 | — | — | — | — |
| 監査照会 | 書 | 読 | — | — | — |

### 4.2 データ水準の制約

1. **プロジェクト分離**：全クエリに `project_id` を強制注入。本文・クエリの `project_id` は受け付けない。パス `/projects/{pid}/…` の `{pid}` は「スコープ宣言」として扱い、セッションの許可範囲と照合して一致した場合だけ ctx へ採用する。不一致は 404（403 にしない）、本文に `project_id` が残っていれば破棄し乖離時は `VALIDATION_SCHEMA`（`detail/04-api-contract.md` §7）。
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
