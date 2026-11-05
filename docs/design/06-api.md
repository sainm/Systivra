# 第六部分：API 设计

## 1. 通用约定

| 项目 | 约定 |
|---|---|
| 基础路径 | `/api/v1`，版本只在破坏性变更时升 |
| 内容类型 | `application/json; charset=utf-8`（上传用 presigned URL，不经过 API 进程） |
| 认证 | 浏览器用 `HttpOnly + SameSite=Lax` 会话 Cookie；程序调用用 `Authorization: Bearer <pat>`（不透明令牌，哈希后存库，可即时吊销） |
| 授权 | RBAC + 项目范围。每个端点声明所需 permission，中间件统一校验 |
| 错误 | RFC 9457 `application/problem+json`，附业务 `code` |
| 分页 | 游标式：`?cursor=...&limit=50`，返回 `next_cursor`；不用 offset（深翻页性能差且会漂移） |
| 幂等 | 写操作接受 `Idempotency-Key`；未提供时，对「创建分析/执行」类接口用内容摘要生成 |
| 异步 | 耗时操作返回 `202 + {job_id, links}`；进度通过 `GET /jobs/{id}` 或 SSE |
| 流式 | `text/event-stream`，事件类型 `progress / partial / done / error` |
| 追踪 | 响应头 `X-Trace-Id`，与 OTel trace 一致；错误体里也带 |
| 时间 | 全部 ISO 8601 UTC（`2026-11-05T12:00:00Z`），前端按 locale 渲染 |
| 命名 | 资源用复数名词；动作用子资源或 `:verb`（仅用于无法用 CRUD 表达的操作） |

### 1.1 错误模型

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

错误码分层：

| 前缀 | 含义 | 例 |
|---|---|---|
| `AUTH_` | 认证 | `AUTH_TOKEN_EXPIRED` |
| `FORBIDDEN_` | 授权 | `FORBIDDEN_PROJECT_SCOPE` |
| `VALIDATION_` | 入参 | `VALIDATION_SCHEMA` |
| `NOT_FOUND_` | 资源 | `NOT_FOUND_DOCUMENT` |
| `CONFLICT_` | 状态冲突 | `CONFLICT_REVISION_SUPERSEDED` |
| `CAPABILITY_` | 能力不可用 | `CAPABILITY_UNSUPPORTED` |
| `PROVIDER_` | 上游故障 | `PROVIDER_TIMEOUT` |
| `BUDGET_` | 超限 | `BUDGET_TOKENS_EXCEEDED` |
| `JOB_` | 作业 | `JOB_CANCELLED` |
| `POLICY_` | 策略拦截 | `POLICY_APPROVAL_REQUIRED` |

`PROVIDER_*` 且 `retryable=true` 时，响应带 `Retry-After`。

---

## 2. 端点清单

### 2.1 Workspace / Project

| 方法 | 路径 | 说明 | 权限 |
|---|---|---|---|
| GET | `/workspaces` | 列出工作区 | `workspace:read` |
| POST | `/workspaces` | 创建工作区 | `system:admin` |
| GET | `/projects?workspace_id=` | 项目列表 | `project:read` |
| POST | `/workspaces/{wid}/projects` | 创建项目 | `project:create` |
| GET | `/projects/{pid}` | 项目详情（含统计） | `project:read` |
| PATCH | `/projects/{pid}` | 修改配置 | `project:admin` |
| GET | `/projects/{pid}/overview` | 首页聚合数据 | `project:read` |

### 2.2 Document / Knowledge

| 方法 | 路径 | 说明 | 权限 |
|---|---|---|---|
| POST | `/projects/{pid}/documents:upload-url` | 获取预签名上传地址 | `document:write` |
| POST | `/projects/{pid}/documents:bulk-import` | 按清单批量导入（异步，允许部分成功） | `document:write` |
| POST | `/projects/{pid}/documents` | 登记文档（上传完成后） | `document:write` |
| GET | `/projects/{pid}/documents` | 文档列表（过滤：type/domain/status） | `document:read` |
| GET | `/documents/{did}` | 文档详情 + 当前修订 | `document:read` |
| GET | `/documents/{did}/revisions` | 修订历史 | `document:read` |
| POST | `/documents/{did}/revisions/{rid}:parse` | 触发解析（异步） | `document:write` |
| GET | `/documents/{did}/tree?revision_id=` | 结构树 | `document:read` |
| GET | `/chunks/{cid}` | chunk 详情 + 前后邻域 | `document:read` |
| DELETE | `/documents/{did}` | 软删 + 索引失效 | `document:delete` |
| POST | `/projects/{pid}/knowledge/search` | 检索 | `knowledge:read` |
| POST | `/projects/{pid}/knowledge/index-builds` | 重建索引 | `knowledge:admin` |
| GET | `/evidence/{eid}` | 证据原文 + 上下文 | `knowledge:read` |

### 2.3 Graph / Ontology

| 方法 | 路径 | 说明 | 权限 |
|---|---|---|---|
| POST | `/projects/{pid}/graph/query` | 结构化查询（实体/邻居） | `graph:read` |
| GET | `/projects/{pid}/graph/entities/{eid}` | 实体详情 + 度统计 | `graph:read` |
| POST | `/projects/{pid}/graph/paths` | 路径查询 | `graph:read` |
| POST | `/projects/{pid}/graph/subgraph` | 子图导出（上限 5000 节点） | `graph:read` |
| GET | `/projects/{pid}/ontology` | 当前 ontology | `graph:read` |
| POST | `/projects/{pid}/ontology/versions` | 发布新版本 | `ontology:admin` + 审批 |
| GET | `/projects/{pid}/traceability` | 追踪矩阵 | `graph:read` |

### 2.4 Code Analysis

| 方法 | 路径 | 说明 | 权限 |
|---|---|---|---|
| POST | `/projects/{pid}/code/repositories` | 绑定仓库 | `code:admin` |
| POST | `/projects/{pid}/code/revisions:sync` | 同步版本（异步） | `code:write` |
| GET | `/projects/{pid}/code/revisions` | 版本列表 | `code:read` |
| POST | `/projects/{pid}/code/analysis-runs` | 启动分析（异步） | `code:write` |
| GET | `/code/analysis-runs/{id}` | 运行状态与统计 | `code:read` |
| GET | `/projects/{pid}/code/symbols?fqn=` | 符号查询 | `code:read` |
| GET | `/projects/{pid}/code/facts?subject=&relation=` | Fact 查询 | `code:read` |

### 2.5 Impact

| 方法 | 路径 | 说明 | 权限 |
|---|---|---|---|
| POST | `/projects/{pid}/changes` | 创建变更集（diff / 手工） | `impact:write` |
| GET | `/changes/{cid}` | 变更集详情 | `impact:read` |
| POST | `/projects/{pid}/impacts` | 执行影响分析（异步） | `impact:run` |
| GET | `/impacts/{iid}` | 报告摘要 + 统计 | `impact:read` |
| GET | `/impacts/{iid}/items?level=&resolution=` | 影响项分页 | `impact:read` |
| GET | `/impacts/{iid}/graph` | 影响图数据（Cytoscape 格式） | `impact:read` |
| POST | `/impacts/{iid}/reports` | 生成 Markdown/PDF | `impact:read` |

### 2.6 Test

| 方法 | 路径 | 说明 | 权限 |
|---|---|---|---|
| POST | `/impacts/{iid}/test-plans:derive` | 从影响项派生计划 | `test:write` |
| GET | `/test-plans/{tid}` | 计划详情（含场景树） | `test:read` |
| POST | `/test-scenarios/{sid}/cases:generate` | 派生用例（草稿） | `test:write` |
| POST | `/test-cases/{cid}:approve` | 批准草稿 | `test:approve` |
| POST | `/projects/{pid}/test-runs` | 创建执行（异步） | `test:execute` |
| GET | `/test-runs/{rid}` | 执行进度与结果 | `test:read` |
| GET | `/test-executions/{eid}` | 单次执行详情（含产物链接） | `test:read` |
| GET | `/projects/{pid}/coverage?test_run_id=` | 覆盖快照 | `test:read` |
| POST | `/test-executions/{eid}/defects` | 登记缺陷 | `test:write` |

### 2.7 AI / Workflow

| 方法 | 路径 | 说明 | 权限 |
|---|---|---|---|
| GET | `/agents` | Agent 列表 | `ai:read` |
| POST | `/agents/{aid}/runs` | 启动 Agent（异步/流式） | `ai:execute` |
| GET | `/agent-runs/{rid}` | 运行详情 + 步骤 | `ai:read` |
| POST | `/agent-runs/{rid}:cancel` | 取消 | `ai:execute` |
| GET | `/approvals?status=pending` | 待审批列表 | `approval:read` |
| POST | `/approvals/{id}:decide` | 批准/驳回 | `approval:decide` |
| GET | `/workflows` | 流程列表 | `workflow:read` |
| POST | `/workflows/{wid}/runs` | 启动流程 | `workflow:execute` |

### 2.8 Platform

| 方法 | 路径 | 说明 | 权限 |
|---|---|---|---|
| GET | `/providers` | Provider 列表 + 能力矩阵 | `platform:read` |
| POST | `/providers` | 注册 | `platform:admin` |
| PATCH | `/providers/{id}` | 启停 / 改优先级 | `platform:admin` |
| POST | `/providers/{id}:probe` | 能力探测 | `platform:admin` |
| GET | `/capabilities` | 能力注册表 | `platform:read` |
| GET | `/mcp/servers` | MCP Server 列表 | `platform:read` |
| POST | `/mcp/servers` | 注册 | `platform:admin` |
| POST | `/mcp/servers/{id}:discover` | 重新发现工具 | `platform:admin` |
| GET | `/audit-logs?actor=&action=&from=` | 审计查询 | `audit:read` |
| GET | `/jobs/{jid}` | 作业状态 | 按资源权限继承 |
| POST | `/jobs/{jid}:cancel` | 取消 | 按资源权限继承 |
| GET | `/jobs/{jid}/events` | SSE 进度 | 按资源权限继承 |

---

## 3. 详细示例

### 3.1 检索

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
  "results": [
    {
      "chunk_id": "CH-9a1c...",
      "document": {"id": "DOC-0012", "title": "ユーザ管理機能 詳細設計書", "doc_type": "DETAILED_DESIGN"},
      "revision": {"id": "REV-0007", "source_revision": "svn-18422"},
      "locator": {"kind": "xlsx", "sheet": "画面項目定義", "range": "C12:F12"},
      "score": 0.0312,
      "sources": ["vector", "fulltext", "graph"],
      "rerank_score": 0.87,
      "quote": "項目名: 住所 / 入力形式: 全角20文字以内 / 必須: 必須"
    }
  ],
  "total_candidates": 143,
  "latency_ms": 186
}
```

要点：`sources` 让用户知道这条结果是「语义命中」还是「关键词命中」，便于判断可信度。`degraded` 显式声明降级。

### 3.2 影响分析

```http
POST /api/v1/projects/8f2c.../impacts
Idempotency-Key: sha256:9f1c...
Content-Type: application/json

{
  "change_set": {
    "kind": "DIFF",
    "base_revision": "abc123",
    "head_revision": "def456"
  },
  "options": {
    "max_depth": 6,
    "include_tests": true,
    "include_docs": true,
    "hub_threshold": 200
  }
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

SSE 进度：

```text
event: progress
data: {"phase":"TRAVERSE","done":1240,"total":null,"message":"depth 3"}

event: progress
data: {"phase":"EVIDENCE","done":18,"total":37,"message":"matching documents"}

event: done
data: {"impact_id":"IMP-2026-0007","stats":{"total":37,"unresolved":1},"truncated":{"hubs":1}}
```

`GET /impacts/{iid}` 返回结构见 04.5 的输出示例。补充 `links` 指向每个影响项的证据与关联用例。

### 3.3 测试执行

```http
POST /api/v1/projects/8f2c.../test-runs
Content-Type: application/json

{
  "name": "IMP-2026-0007 回帰",
  "case_ids": ["TC-2026-0118", "TC-2026-0119", "..."],
  "environment": {"id": "env-stg-01", "base_url": "https://stg.example.internal"},
  "runner": {"type": "playwright", "browser": "chromium", "headless": true},
  "retry": {"max": 1, "on": ["TIMEOUT", "NETWORK"]}
}
```

```json
{
  "test_run_id": "RUN-2026-0112",
  "status": "QUEUED",
  "case_count": 2,
  "estimated_duration_sec": 40
}
```

单次结果：

```json
{
  "execution_id": "EXE-01JA...",
  "test_case_id": "TC-2026-0118",
  "status": "FAILED",
  "failure_kind": "ASSERTION",
  "duration_ms": 8420,
  "step_results": [
    {"index": 1, "action": "goto", "status": "PASSED"},
    {"index": 2, "action": "fill", "status": "PASSED"},
    {"index": 3, "action": "click", "status": "PASSED"},
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

## 4. 权限模型

### 4.1 角色 × 动作

| 动作 | OWNER | ARCHITECT | DEVELOPER | TESTER | VIEWER |
|---|---|---|---|---|---|
| 项目设置 | 写 | 读 | 读 | 读 | 读 |
| 文档导入/删除 | 写 | 写 | 写 | 读 | 读 |
| 检索/证据 | 读 | 读 | 读 | 读 | 读 |
| 代码同步/分析 | 写 | 写 | 写 | 读 | 读 |
| 影响分析执行 | 写 | 写 | 写 | 读 | 读 |
| 测试资产编辑 | 写 | 写 | 读 | 写 | 读 |
| 测试执行 | 写 | 写 | 读 | 写 | 读 |
| 用例批准 | 写 | 写 | — | 写 | — |
| Ontology 发布 | 写 | 审批 | — | — | — |
| Agent 执行 | 写 | 写 | 写 | 写 | — |
| Provider / MCP 管理 | 写 | — | — | — | — |
| 审计查询 | 写 | 读 | — | — | — |

### 4.2 数据级约束

1. **项目隔离**：所有查询强制注入 `project_id`，由 Capability 层完成，不接受前端传入的 `project_id` 作为信任依据。
2. **文档 ACL**：`document.acl_groups` 与用户组求交，无交集则不出现在检索结果中（在 SQL 层过滤，不做事后过滤）。
3. **Agent 降权**：Agent 的有效权限 = `agent.capabilities ∩ 发起用户权限 ∩ 项目范围`。
4. **敏感操作二次确认**：删除项目、发布 Ontology、执行正式环境测试，需要 `X-Confirm-Action: <resource_id>` 头 + 审批流。

### 4.3 审计要求

每个写操作记录：

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

Agent 发起的操作额外记录 `agent_run_id` 与 `capability`。审计日志不可修改、不可删除（数据库层 REVOKE）。

---

## 5. OpenAPI 与契约测试

- 用 `utoipa` 从 Rust 类型生成 OpenAPI 3.1，**不允许手写 YAML 作为源头**（会与实现漂移）。
- CI 校验：生成的 spec 与 `contracts/api/openapi.json` 一致；不一致则失败，要求显式提交变更。
- 前端类型由 OpenAPI 生成（`openapi-typescript`），不允许手写重复类型。
- 破坏性变更检测：`oasdiff` 对比 main 与 PR，标记 breaking change，需要人工批准。
