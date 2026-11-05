# 第二部分：总体架构

## 1. 系统上下文

```text
                         ┌──────────────────────────────────────────┐
   設計書 (PDF/DOCX/     │                                          │
   XLSX/PPTX)  ─────────▶│                                          │
                         │                                          │
   ソースコード ────────▶│        Systivra Platform                 │───▶ 影響分析レポート
   (Git / SVN / Dir)     │        (分析・追跡・テスト派生)           │───▶ テスト資産 / 実行結果
                         │                                          │───▶ 追跡マトリクス
   DB 定義 / DDL ───────▶│                                          │
                         │                                          │
   テスト仕様書 ────────▶│                                          │───▶ MCP (对外能力)
                         └───────┬──────────────┬───────────────────┘
                                 │              │
                    ┌────────────▼───┐   ┌──────▼─────────────┐
                    │ AI Provider    │   │ 外部 MCP Server    │
                    │ DeepSeek /     │   │ (Git / Issue /     │
                    │ Copilot / vLLM │   │  Test / Graph ...) │
                    └────────────────┘   └────────────────────┘

   外部执行环境：被测系统 (Java Web + Browser) ◀── Playwright Runner
```

上下文边界的三条规则：

1. **平台不持有被测系统的运行环境**，只持有其访问配置（A-06）。
2. **AI Provider 是可选外部依赖**，断开后平台仍可完成知识构建、代码分析、图构建、影响计算、测试资产管理（P6）。
3. **MCP 是边界协议，不是内部调用方式**。内部一律走 Capability，见 §3。

---

## 2. 总体组件图

按依赖方向分层，**上层可依赖下层，下层不得反向依赖**。

```text
┌─────────────────────────────────────────────────────────────────────────────┐
│ L1 Experience                                                               │
│   React Workbench  ·  Monaco  ·  Cytoscape.js  ·  React Flow  ·  TanStack   │
└───────────────────────────────┬─────────────────────────────────────────────┘
                                │ REST / SSE (OpenAPI 3.1)
┌───────────────────────────────▼─────────────────────────────────────────────┐
│ L2 API Edge (crates/api)                                                    │
│   axum router · AuthN (session/PAT) · AuthZ (RBAC + project scope)          │
│   request validation · idempotency · rate limit · problem+json · tracing    │
└───────────────────────────────┬─────────────────────────────────────────────┘
┌───────────────────────────────▼─────────────────────────────────────────────┐
│ L3 Application (crates/app)                                                 │
│   UseCase 编排：IngestDocument / RunCodeAnalysis / AnalyzeImpact /          │
│   GenerateTestAssets / ExecuteTestRun / RunAgent / RunWorkflow              │
│   事务边界 · 幂等键 · 领域事件发件箱                                          │
└───────────────────────────────┬─────────────────────────────────────────────┘
┌───────────────────────────────────────────▼─────────────────────────────────┐
│ L4 Capability (runtime::capability)  ← 唯一对外/对 AI 暴露的稳定契约         │
│   KnowledgeCap · GraphCap · CodeAnalysisCap · ImpactCap · TestCap ·         │
│   WorkflowCap · AdminCap                                                    │
│   每个 Capability = 输入 Schema + 输出 Schema + 权限 + 超时 + 错误契约        │
└───────────────────────────────┬─────────────────────────────────────────────┘
┌───────────────────────────────▼─────────────────────────────────────────────┐
│ L5 Engine (纯业务，不知道 Provider 是谁)                                      │
│   rag-engine │ graph-engine │ ontology │ code-analysis │ impact-engine │    │
│   test-engineering │ agent-runtime │ workflow-engine                        │
└───────────────────────────────┬─────────────────────────────────────────────┘
┌───────────────────────────────▼─────────────────────────────────────────────┐
│ L6 Provider (可替换实现，唯一允许出现厂商名词的层)                             │
│   RagProvider · GraphProvider · EmbeddingProvider · RerankProvider ·        │
│   AiProvider · ParserProvider · CodeAnalyzerProvider · TestRunnerProvider · │
│   StorageProvider                                                           │
└───────────────────────────────┬─────────────────────────────────────────────┘
┌───────────────────────────────▼─────────────────────────────────────────────┐
│ L7 Data & Ops                                                               │
│   PostgreSQL (SoR) · pgvector · Neo4j (projection) · S3/MinIO               │
│   job queue · outbox · audit · metrics · object lifecycle                   │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 2.1 为什么 Capability 层独立于 Engine 层

Engine 是「怎么算」，Capability 是「允许谁、以什么代价、按什么契约算」。两者混在一起会导致：

- Agent 直接拿到 Engine 句柄 → 权限无法收口（Prompt 注入即可越权）。
- MCP Tool 直接映射 Engine 方法 → 换 Provider 时 Tool 语义漂移。
- 无法统一做超时、配额、审计。

Capability 是**唯一的执行入口**，MCP Server、Agent Tool、内部 UseCase 都走它。区别只在调用者身份与权限集。

---

## 3. 模块边界

| 模块 | 拥有（唯一写入者） | 允许依赖 | 明令禁止 |
|---|---|---|---|
| `api` | HTTP 契约、认证会话 | app, capability | 直接访问 sqlx / neo4j driver |
| `app` | 用例事务边界、幂等、发件箱 | capability, store, domain | 包含解析/检索算法 |
| `runtime::capability` | 能力 Schema、权限映射、超时预算 | engine traits, `providers` | 包含业务算法 |
| `rag-engine` | 文档结构、切块、检索融合、证据组装 | `RagProvider`, `EmbeddingProvider`, `RerankProvider` | 直接写 PG（通过 store port） |
| `graph-engine` | 图域模型、遍历算法、路径评分 | `GraphProvider`, store | 拼接未经校验的 Cypher 片段 |
| `ontology` | 实体/关系类型注册表、约束、版本 | store | 硬编码业务实体类型到代码分支 |
| `code-analysis` | 扫描调度、Fact 归一化、增量 | `CodeAnalyzerProvider`, `ParserProvider`, store | 直接调用 OpenRewrite HTTP |
| `impact-engine` | 种子解析、分层遍历、风险打分 | graph-engine, rag-engine, store | 生成无证据的影响项 |
| `test-engineering` | 规则目录、Scenario/Case/Data、覆盖口径 | store, `TestRunnerProvider`, code-analysis | 把「生成」当「覆盖」 |
| `agent-runtime` | Agent 注册、会话、预算、审批 | capability | 持有数据库连接 / 执行任意 SQL |
| `workflow-engine` | 定义解析、状态机、检查点 | capability | 内嵌第二套 AI 编排 |
| `provider-manager`（`providers::manager`） | 注册、健康、切换、密钥引用 | provider traits | 业务逻辑 |
| `store` | 所有 PG 访问、迁移、发件箱 | domain | 跨模块业务判断 |
| `worker` | 作业领取、执行、重试、心跳 | app, capability | 绕过 app 直接改状态 |

**边界判据**：如果一个改动需要同时修改两个模块的「拥有物」，说明边界划错了，先改边界再改代码。

> 表中的名称是逻辑模块。crate 名以 `08-roadmap.md` §3 为准。`runtime::capability` 属 `runtime` crate，`providers::manager` 属 `providers` crate，`ontology` 属 `engine-graph` crate。`rag-engine` / `graph-engine` / `code-analysis` / `impact-engine` / `test-engineering` 分别对应 crate `engine-rag` / `engine-graph` / `engine-code` / `engine-impact` / `engine-test`。`agent-runtime` 与 `workflow-engine` 归入 `runtime` crate，但架构层仍是 L5（crate 划分与分层并不一一对应）。

---

## 4. 数据权威与一致性

这是整个架构最容易出错的地方。Prompt 明确要求「不能只考虑把所有对象保存到 Neo4j」，本节给出可执行的答案。

### 4.1 权威源矩阵

| 数据 | 权威源 | 派生/缓存 | 重建方式 | 丢失后果 |
|---|---|---|---|---|
| 项目、用户、权限 | PostgreSQL | — | 备份恢复 | 严重 |
| 文档元数据、修订 | PostgreSQL | — | 备份恢复 | 严重 |
| 原始文件字节 | Object Storage | 本地缓存 | 从源系统重导 | 中（可重导） |
| 解析树 / doc_node | PostgreSQL | — | 重新解析（结果可能不同，需锁 parser 版本） | 中 |
| Chunk 文本 | PostgreSQL | — | 从 doc_node 重建 | 中 |
| Embedding 向量 | PostgreSQL (pgvector) | — | 重新 embed（需同模型版本） | 低（可重建） |
| Analysis Fact | PostgreSQL（**只追加**） | — | 重跑分析（需同 analyzer 版本） | 高（历史不可复现） |
| 图实体/关系 | **PostgreSQL** | Neo4j | 从 fact + entity 全量重投影 | 低（可重建） |
| 测试资产 / 执行结果 | PostgreSQL | — | 备份恢复 | 严重 |
| 截图 / trace / 附件 | Object Storage | — | 不可重建 | 中 |
| 审计日志 | PostgreSQL（只追加） | — | 备份恢复 | 严重（合规） |

> **核心决策（ADR-002）**：Neo4j 不持有任何「只存在于图里」的数据。所有实体、关系、属性在 PG 都有对应行。Neo4j 是查询加速器与遍历引擎，删库可重建。

### 4.2 为什么不让 Neo4j 当权威源

四条理由，按重要性排序：

1. **Provider 可替换性**。如果 Neo4j 是唯一事实源，「换成别的图库」就意味着数据迁移 + 风险，Provider 抽象就是假的。
2. **事务一致性**。测试资产、审批、审计必须和关系变更在同一事务里。跨 PG/Neo4j 的分布式事务成本远高于「PG 单事务 + 异步投影」。
3. **许可与部署**（F-02）。Neo4j Community 为 GPLv3，把它链接进产品会波及我方代码；让它可以被替换、可以缺席（lite 模式），是商业上的必要。
4. **证据模型**。关系要带 `revision / resolution / evidence`，这些是关系型结构，用属性图表达会退化成「边上一坨 JSON」，查询反而更难。

### 4.3 同步机制：Outbox + 幂等投影

```text
[事务边界内]
  INSERT/UPDATE entity
  INSERT/UPDATE relationship
  INSERT outbox_event(event_id, aggregate, seq, type, payload, occurred_at)
COMMIT

[投影 worker]
  SELECT ... FROM outbox_event
   WHERE published_at IS NULL
   ORDER BY seq
   FOR UPDATE SKIP LOCKED LIMIT 200
  → 逐条应用（MERGE + 条件写）
  → 标记 published_at
```

关键细节（不做这些就会出错）：

| 问题 | 处理 |
|---|---|
| 乱序 | 每个 `entity_id` 维护 `projection_seq`；写入时 `WHERE e.projection_seq < $seq`，旧事件直接丢弃 |
| 重复投递 | `event_id` 唯一约束 + `MERGE` 幂等；投影表记录 `last_event_id` |
| 部分失败 | 单条事件失败不阻塞整批；进入 `outbox_dead_letter`，超过阈值告警 |
| 删除传播 | 不物理删除，写 `deleted_at` + `ProjectionTombstone` 事件；图侧 `DETACH DELETE` 分批执行 |
| 全量重建 | `POST /admin/graph/rebuild` 创建 `projection_build`，影子标签 `:Entity@build_id`，完成后原子切换读别名（Neo4j 无别名则用双写+切换查询参数） |
| 漂移检测 | 每日抽样 1000 个实体比对 PG 与 Neo4j 的度数/属性哈希，差异率 > 0.1% 告警 |

### 4.4 删除与修订

文档更新不是「覆盖」，是新增 `document_revision`。旧 revision 的 chunk 保留但标记 `superseded_at`；检索默认只查 `is_current = true`，历史查询显式指定 revision。

级联顺序（必须按此顺序，否则会查出「幽灵证据」）：

```text
1. document_revision.status = 'superseded'      (PG, 事务内)
2. outbox: DocumentRevisionSuperseded
3. 投影：相关 entity/relationship 标记 revision 过期
4. RAG：chunk.superseded_at = now()；从当前索引逻辑排除（不立即删向量）
5. 异步 GC：超过 N 天的 superseded chunk 物理删除 + 向量删除
```

第 4 步「逻辑排除而非立即删除」是刻意设计：删除向量会导致正在运行的影响分析拿到半套证据。用 `is_current` 过滤，保证读者看到一致快照。

---

## 5. 数据流

### F1 知识构建（文档 → 可检索证据）

```text
上传/同步
  → 计算 content_hash（去重）
  → 创建 document_revision（绑定 source_revision / acquired_at）
  → 入队 parse job
      → ParserProvider 分流：
          XLSX        → 自研 Rust 解析器 (calamine + OOXML 补充)
          PDF/DOCX/PPTX → docling-serve（现成容器）
          MD/TXT/CSV  → 原生
          HTML        → 原生 (scraper + 结构提取)
      → 产出 DocNode 树（含 locator）
  → 结构分类（doc_type / business_domain / screen 关联）
  → 语义切块（结构感知，见 04.1）
  → 元数据抽取 + 实体链接候选
  → Embedding（批量、可中断、断点续跑）
  → 写 pgvector + tsvector
  → outbox: ChunkIndexed → 图投影（只投影 Document / Section。chunk 不投影）
```

每步都有独立 `parse_run` 记录，失败可从任意步骤重跑，不必从头。

### F2 关系构建（代码 → Fact → 图）

```text
Repository 同步（Git/SVN/Dir）
  → 解析 revision（commit sha / svn rev / tree digest）
  → 变更检测（对比上一基线，得到 added/modified/deleted/moved）
  → 文件分类与语言识别
  → 分发：
      Tree-sitter tags 查询 → 符号表、import、注解、字符串字面量
      OpenRewrite Worker  → 类型归属、方法解析、继承/实现
      Framework Adapter    → Action 映射、Dicon DI、JSP taglib、SQL 绑定
      Config Analyzer      → XML/YAML/Properties/CSV
  → 归一化为 Fact（统一 Schema，见 05.4）
  → 冲突检测（同 subject+relation 多目标 → CONFLICT 组）
  → 写入 analysis_fact（只追加，旧 fact 标 superseded）
  → outbox: FactAccepted → 图投影
```

### F3 影响分析（变更 → 影响报告）

```text
输入：change_set（diff / 手工指定 / 需求变更）
  → ① 种子解析：文件+行号 → 所属符号（tree-sitter 定位）→ entity_id
  → ② 变更分类：代码 / 配置 / DB / 文档 / API
  → ③ 反向遍历（调用方、实现方、引用方）  ┐
  → ④ 共享资源闭包（表、外部 API、文件）    ├─ 确定性（impact-engine）
  → ⑤ 前向遍历（DB 写入、外部调用）        ┘
  → ⑥ 文档证据检索（实体名 + 业务规则关键词 → RAG，带过滤器）
  → ⑦ 测试资产匹配（追踪矩阵：entity → test_case）
  → ⑧ 风险打分（确定性公式，见 04.5）
  → ⑨ AI 补充：遗漏假设、业务解释（标记 INFERRED，不进主表）
  → ⑩ 产出 ImpactReport + 图 + 证据包
```

### F4 测试闭环

```text
ImpactItem / BusinessRule
  → Scenario 派生（业务流 / 异常 / 边界 / 状态迁移 / 并发）
  → Case 派生（步骤 + 输入 + 预期，绑定 evidence）
  → Data 派生（等价类 + 边界，来自 ValidationRule）
  → 人工评审（Draft → Approved）
  → TestRunnerProvider（Playwright）执行
  → 结果采集（截图 / trace / log / 断言）
  → Coverage 重算（四维）
  → 失败 → Defect（带 evidence 与 impact 关联）
```

### F5 AI 调用链（受控）

```text
AgentRun 启动（预算：tokens / tool_calls / wall_time / cost）
  → Agent 决定调用某 Capability（不是工具名，是能力名）
  → Capability Gateway：
       1. 校验 input JSON Schema
       2. 校验 agent 白名单 ∩ 用户权限 ∩ 项目范围
       3. 注入 trace_id / project_id（不可被 Agent 覆盖）
       4. 执行（超时 / 取消令牌）
       5. 输出 Schema 校验
       6. 标记 untrusted=true（若内容来自文档或外部 MCP）
       7. 审计（who / what / why / cost / duration）
  → 结果回到 Agent（作为 data，不作为 instruction）
```

---

## 6. 运行时架构

### 6.1 进程模型

| 进程 | 语言 | 职责 | 伸缩方式 | 状态 |
|---|---|---|---|---|
| `sep-api` | Rust | HTTP/SSE、认证、用例编排 | 水平（无状态） | 无本地状态 |
| `sep-worker` | Rust | 解析调度、投影、影响计算、测试编排 | 水平（按队列分区） | 通过 job 表协调 |
| `docling-serve` | Python | Docling 解析（现成容器，无自研代码） | 水平（CPU/GPU 密集） | 无状态 |
| `rewrite-worker` | Java | OpenRewrite LST 解析 | 水平（JVM 内存大） | 无状态 |
| `test-runner` | Node | Playwright 执行 | 水平（按浏览器实例） | 临时 workspace |

**为什么 docling-serve 与 rewrite-worker 必须是独立进程**：

- 依赖冲突：Docling 需要特定 PyTorch/Python 版本；OpenRewrite 需要 JDK 与完整类型解析。
- 故障隔离：解析一个畸形 Excel 不应该拖垮 API。
- 资源画像不同：Docling 吃内存与 CPU，OpenRewrite 吃堆，API 吃连接数。

但它们**不持有业务状态**，只做「输入字节 → 输出结构化结果」，所有状态回到 Rust 侧。这是「Python/Java 只作为独立处理能力」的落地方式。

### 6.2 并发与背压

```text
API 请求 → 入队（job 表）→ 立即返回 202 + job_id
                │
                ├─ 队列按 (project_id, priority) 分区
                ├─ 每类作业有独立并发上限（Excel 解析 8~16 / 投影 8 / 影响 4 / 执行 2）
                ├─ 全局限流：单项目并发作业数上限，防止一个项目饿死其他项目
                └─ 背压：队列深度 > 阈值时，新作业进入 PENDING 并给出预估等待
```

作业状态机（严格）：

```text
PENDING → QUEUED → RUNNING → SUCCEEDED
                    │  ├──→ FAILED → (retry) → QUEUED
                    │  ├──→ CANCELLED
                    │  └──→ WAITING_APPROVAL → RUNNING
                    └──→ TIMEOUT → FAILED
```

幂等：每个作业有 `idempotency_key`（默认 `hash(project_id, operation, input_digest, revision)`）。重复提交返回同一 `job_id`，不重复执行。

检查点：长作业（> 5 分钟）必须每 N 个单元写 `job_checkpoint`（已处理文件列表、游标）。进程崩溃后从检查点续跑，不从头开始。

### 6.3 取消语义

`CancellationToken` 贯穿 Capability → Engine → Provider。Provider 层必须响应取消（HTTP 请求带 `abort`，子进程 kill group）。**不允许出现「API 说取消了，worker 还在跑」的情况**——这会导致用户在界面上看到取消成功、后台却写入了半套数据。

---

## 7. 部署架构

### 7.1 Docker Compose（首期）

```yaml
services:
  api:            sep/api:0.1        # :8080
  worker:         sep/worker:0.1
  docling-serve:  quay.io/docling-project/docling-serve-cpu:v1.38.1  # :5001
  rewrite-worker: sep/rewrite:0.1    # :8081
  test-runner:    sep/test-runner:0.1# :8082
  postgres:       pgvector/pgvector:pg18  # 生产环境固定 0.8.6 系的 digest
  neo4j:          neo4j:5.26.32-community
  minio:          minio/minio
  otel-collector: otel/opentelemetry-collector-contrib
volumes:
  pgdata, neo4jdata, miniodata, workspace
```

### 7.2 三种部署形态

| 形态 | 组成 | 数据隔离 | 适用 | 差异点 |
|---|---|---|---|---|
| **Local / Dev** | 全部容器 + `lite` profile | 单库 | 开发、演示 | Graph 可用 `PgGraphProvider`（递归 CTE）替代 Neo4j；AI 走 Ollama 或关闭 |
| **私有化（默认）** | 全量容器，客户内网 | 单库多 schema，项目行级隔离 | 日企客户 | 离线镜像包；AI Provider 可选；审计日志接客户 SIEM |
| **SaaS（未来）** | 每租户独立 schema 或独立库 | 物理隔离（推荐）或 schema 隔离 | 多客户 | 需 SSO、配额计费、跨租户运维；Phase 8 才考虑 |

### 7.3 lite profile 的意义

不是「阉割版」，是**可验证性工具**。它证明了一件事：把 Neo4j 拿掉，平台的核心闭环仍然成立（只是大图遍历变慢）。如果做不到，说明图能力被过度耦合了。

`PgGraphProvider` 的能力声明（见 04.10）会明确写：支持 1~3 跳邻居与路径查询，不支持大规模子图导出与图算法。能力矩阵里显示为 `PARTIAL`，UI 上会提示。

### 7.4 资源基线（单项目，A-02 规模）

| 服务 | 数量 | CPU | 内存 | 磁盘 |
|---|---|---|---|---|
| xlsx 解析 worker（`sep-worker` 解析池） | 16 | 4 | 8 GB | 50 GB |
| postgres | 1 | 16 | 64 GB | 2 TB NVMe |
| neo4j | 1 | 8 | 64 GB | 500 GB |
| minio | 2 | 4 | 8 GB | 2 TB |

> 全部服务与依据以 `ja/reference/scale-model.md` §7 为准。旧假设（20 万 chunk、4 CPU / 16 GB）的数字不再使用。Phase 2 与 Phase 4 用实测值回填。

---

## 8. 架构层面的失败模式（提前写下来）

| 失败模式 | 触发条件 | 表现 | 预防 |
|---|---|---|---|
| 投影漂移 | outbox 消费失败未告警 | 图上少了一条关键边，影响分析静默漏项 | 每日漂移检测 + 「图完整度」指标上首页 |
| 证据悬空 | 文档改版但 chunk 未失效 | 点击证据打开空白/错位 | revision 绑定 + stale 标记 + 抽样校验 |
| 解析雪崩 | 一个超大 XLSX 占满 docling-serve | 其他项目排队超时 | 单文件大小/页数上限 + 独立队列 + 超时熔断 |
| 事务越界 | Capability 内部开事务，调用方也开 | 死锁、部分提交 | 事务只在 app 层开启；engine 只接受 `&mut Tx` |
| 幂等失效 | 重试时 input_digest 包含时间戳 | 重复生成重复资产 | 幂等键只取内容相关字段 |
| AI 越权 | Agent 直接把 project_id 作为参数传入 | 跨项目读取 | project_id 由网关注入，Agent 不可见不可改 |
