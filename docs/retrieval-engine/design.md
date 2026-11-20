# Retrieval Engine 设计

> 本文件基于**当前仓库的实际代码与数据库结构**（2026-11-20 实测），不是通用方案。
> 每个结论都标注了依据（表名 / 文件 / 字段），便于复核。
> 与题目假设不一致的地方，在 §1.6 单独列出并给出取舍。

---

## 0. 结论摘要

- 现有系统已经具备「向量 + 关键词 + RRF 融合 + 规则重排 + 证据追溯」的检索链路，
  **不需要新建检索基础设施**，需要的是把散在 `app/knowledge.rs` 里的流程抽成可编排的引擎。
- 层级结构已经存在（`corpus.doc_node` + `corpus.chunk.node_path`），
  **不要新建第二套层级**。
- **跨文档业务关系完全不存在**——这是本次唯一需要新增的存储结构，
  且只能落在 Chunk 级（原因见 §3.4：`chunk` 不是图实体）。
- Neo4j 在本系统里是**投影**，不是查询源（`graph_neighbors` 等全部是 SQL；
  `NEO4J_DISABLED=1` 时检索完全正常）。因此「PostgreSQL / Neo4j 联合检索」
  的正确形态是：**同一套遍历逻辑，Postgres 为权威，Neo4j 可选做深路径与可视化**。
- 系统**没有 MCP 服务端**，只有客户端（`crates/app/src/mcp_client.rs`）。
  第六节的 MCP Tool 属于新增适配层，必须只调引擎、不碰存储。

---

## 1. 现状分析（实证）

### 1.1 存储：知识侧

| 表 | 关键字段 | 说明 |
|---|---|---|
| `corpus.document` | `doc_type`(text), `title`, `business_domain`, `acl_groups[]`, `deleted_at` | **文档类型已有元数据**，无需从文件名猜 |
| `corpus.document_revision` | `content_hash`, `object_key`, `status`, `parsed_at`, `source_encoding` | 版本与解析产物 |
| `corpus.doc_node` | `parent_id`, `node_type`, `locator jsonb`, `ordinal`, `text` | 文档内部树：`DOCUMENT`(5) / `SECTION`(203) |
| `corpus.chunk` | `node_path text[]`, `locator`, `text`, `fts`, `is_current`, `metadata`, `token_count` | 段落级；`chunk_type` 实测为 `PARAGRAPH`(2814) / `TEXT`(203) |
| `corpus.evidence` | `chunk_id`, `quote`, `quote_hash`, `resolution`, `confidence`, `locator` | 原文引证，**已存在** |
| `corpus.chunk_p00..p31` / `corpus.embedding_1024(_pNN)` | — | 分片（物理），应用层只写 `corpus.chunk` / `corpus.embedding_1024` |
| `corpus.retrieval_log` | `query`, `filters`, `lexical_mode`, `candidates jsonb`, `selected[]`, `rerank_used`, `latency_ms` | 单次搜索的日志，**已有** |

结论：`node_path` + `doc_node.parent_id` 已经能表达「文档 → 章节 → 段落」，
且 `locator` 里带页码/Sheet/行号。**Hierarchical Retrieval 的数据基础是现成的。**

### 1.2 检索链路（现状）

`crates/app/src/knowledge.rs::search`：

```
normalize_nfkc → strict_tsquery（全词）→ 无果则 tsquery（宽松）
              → embed(query)  → search_vector
              → engine_rag::fuse(RRF)     ← 已有融合
              → engine_rag::rerank(规则)  ← 已有重排
              → 写 corpus.retrieval_log
```

`SearchRequest`（`contracts/src/knowledge.rs:42`）已有：
`query / mode / filters{doc_types, business_domain, revision_id} / top_k / rerank / graph_assist / include`。

`SearchResult` **已经带 `sources: Vec<String>` 与 `evidence_id`**——「保留召回来源」这条需求
现状已满足一半，缺的是把 `sources` 填全（现在只有 `lexical` / `vector`）。

`graph_assist` 目前是**空实现**：只往 `degraded` 里写 `"graph_assist_off"`
（`knowledge.rs:284`）。这是最明显的「说了没做」的入口。

`corpus.retrieval_log` 只记单次搜索的候选与选中，**没有多阶段**，无法回答
「图扩展命中了什么、融合前后分数怎么变」。

### 1.3 图

`graph_meta.entity` 实测实体类型：`Section(100) / Method(47) / TestCase(41) / Module(10) / Class(9) / Screen(4) / Requirement(4) / Field(3)`。
关系：`CONTAINS(203) / DECLARES(143) / CALLS(67) / VERIFIES(32) / REFERENCES(6) / QUERIES / READS / DEPENDS_ON / SUBMITS_TO / DISPATCHES`。

几件重要的事实：

1. `Document -CONTAINS-> Section` 的存在说明**文档层级在图里也投影了一份**，
   与 `doc_node` 重复。检索不能从这里读层级（双源必然漂移）。
2. `TestCase -VERIFIES-> Requirement`、`Screen -CALLS-> Method` 已经跨了「文档 ↔ 代码」，
   但 **Section 与代码实体之间没有边**——文档提到代码只发生在查询时
   （`graph_entities_mentioned_in` 做文本包含匹配）。
3. 遍历全部是 SQL：`graph_neighbors` / `graph_subgraph` / `graph_path`
   （`crates/store/src/graph.rs`）。Neo4j 只在 `neo4j_counts` / 投影时用到。
4. `engine-impact` 已有成熟的遍历器：`TraversalOptions{max_depth(1..10), hub_threshold}`、
   `truncation`、`degraded`、`adjusted_risk`。**跨文档扩展应当复用它，而不是另写 BFS。**

### 1.4 MCP

- 只有客户端：`crates/app/src/mcp_client.rs`（注册外部 MCP、发现 tools、把外部 tool 映射到本地 Capability）。
- 路由是 `/ai/mcp-servers`（注册/发现/列工具），**没有对外暴露 MCP 服务端**。
- 所以「MCP Tool」是**从无到有**，且必须与引擎解耦（§8）。

### 1.5 现状与目标的差距

| 需求 | 现状 | 差距 |
|---|---|---|
| 统一 RetrievalRequest/Response | `SearchRequest` 只覆盖 GLOBAL | 需扩展策略、上下文、trace |
| 文档类型检索 | `filters.doc_types` 已有 | 无「按类型优先」的策略 |
| 层级检索 | `doc_node` / `node_path` / `ChunkNeighbors` 已有 | 无按命中点向上/向下扩展的 API |
| 跨文档检索 | **无任何文档间业务关系** | 需新增关系存储 + 遍历 |
| 图辅助 | `graph_assist` 空实现 | 需实现或用新策略替代 |
| 融合 | RRF + 规则 rerank 已有 | 缺归一化说明、权重配置、来源标注不全 |
| Context Builder | 无通用实现（inquiry 里有摘要，不是证据组织） | 新增 |
| Trace | `retrieval_log` 单阶段 | 需分阶段记录 |
| MCP 服务端 | 无 | 新增 |
| 评估 | 有 eval 脚本，但无检索指标 | 需 Recall@K/MRR/nDCG |

### 1.6 与题目假设不一致之处（取舍）

| 题目表述 | 实际 | 取舍 |
|---|---|---|
| 「Neo4j 联合检索」 | 图查询是 SQL，Neo4j 是投影 | **不把 Neo4j 变成查询源**；保留「可选加速」位，避免引入双写一致性风险 |
| 「关系类型 DERIVED_FROM 等」 | 现有只有 CONTAINS/VERIFIES/REFERENCES/CALLS… | 先做**映射**，只补真正缺的（§3.5），不照抄清单 |
| 「Hierarchical 从章节摘要到 Chunk」 | 无章节摘要字段 | 用 `doc_node.text`（章节正文）代替摘要；摘要生成留到可选增强 |
| 「MCP 已经向外提供检索」 | 只有客户端 | 按「新增」排期，不假装已有 |

---

## 2. 目标架构

### 2.1 分层

```
┌───────────────────────────────────────────────┐
│ 外部接口                                        │
│  REST: /retrieval:query   MCP: search_knowledge │  ← 薄适配，无检索逻辑
└───────────────┬───────────────────────────────┘
                │
┌───────────────▼───────────────────────────────┐
│ Retrieval Service（编排）                       │
│  策略选择 → 各阶段执行 → 融合 → 重排 → 上下文      │
│  crate: engine-retrieval + app/src/retrieval.rs │
└───┬───────────┬───────────┬───────────┬───────┘
    │           │           │           │
┌───▼───┐  ┌────▼────┐  ┌───▼────┐  ┌───▼──────┐
│Vector │  │Keyword  │  │Graph   │  │Hierarchy │   各阶段独立，可单独关闭
│Adapter│  │Adapter  │  │Adapter │  │Adapter   │
└───┬───┘  └────┬────┘  └───┬────┘  └───┬──────┘
    │           │           │           │
┌───▼───────────▼───────────▼───────────▼───────┐
│ 既有存储：corpus.* / graph_meta.* / pgvector   │  ← 不新增存储引擎
└───────────────────────────────────────────────┘
```

要点：

- **不新增进程**。Retrieval Service 就在 `app` 层，和 `knowledge::search` 同一进程，
  共用鉴权、审计、`store::Tx`。
- **不新增检索后端**。Vector 走现有 `search_vector`，Keyword 走 `search_lexical`。
- Adapter 是「trait + 现有实现」，便于测试时替换（§9 用假实现跑单测）。

### 2.2 模块与依赖方向

| 模块 | 位置 | 依赖 | 职责 |
|---|---|---|---|
| 契约 | `crates/contracts/src/retrieval.rs`（新） | 无 | Request/Response/Candidate/Trace 结构 |
| 纯算法 | `crates/engine-retrieval`（新） | contracts | 融合、归一化、上下文装箱、策略选择（**无 IO**） |
| 存储适配 | `crates/store`（现有） | — | 加 `chunk_relation` 读写、层级扩展查询 |
| 编排 | `crates/app/src/retrieval.rs`（新） | 上述 | 鉴权、执行阶段、落 trace、审计 |
| 路由 | `crates/api/src/routes/retrieval.rs`（新） | app | HTTP |
| MCP | `crates/mcp-server`（新） | **只依赖 HTTP/契约** | 6 个 tool 的薄包装 |

依赖方向单向：`api → app → (engine-retrieval, store)`；`mcp-server → api`（或直接调 app，二选一，见 §8.3）。
`engine-retrieval` 不依赖 `store`，保证可以用纯数据单测。

---

## 3. 数据模型调整

### 3.1 复用（不改）

`document` / `document_revision` / `doc_node` / `chunk` / `evidence` / `retrieval_log` 全部保留。
`chunk.node_path` 是层级检索的主键路径，不改结构。

### 3.2 新增：`corpus.chunk_relation`（唯一的必要新增）

```sql
CREATE TABLE corpus.chunk_relation (
    id            uuid PRIMARY KEY,
    project_id    uuid NOT NULL,
    src_chunk_id  uuid NOT NULL REFERENCES corpus.chunk(id),
    dst_chunk_id  uuid NOT NULL REFERENCES corpus.chunk(id),
    relation      text NOT NULL,              -- 见 §3.5 的词表
    confidence    real NOT NULL DEFAULT 0,    -- 0..1
    method        text NOT NULL,              -- MANUAL / RULE / IMPORTED
    evidence_id   uuid REFERENCES corpus.evidence(id),
    note          text,
    created_at    timestamptz NOT NULL DEFAULT now(),
    verified_at   timestamptz,                -- NULL = 未人工确认
    UNIQUE (project_id, src_chunk_id, dst_chunk_id, relation)
);
CREATE INDEX ON corpus.chunk_relation (project_id, src_chunk_id, relation);
CREATE INDEX ON corpus.chunk_relation (project_id, dst_chunk_id, relation);
```

**为什么必须新建，而不是复用 `graph_meta.relationship`：**
后者的两端是 `graph_meta.entity`，`chunk` 不是实体（`entity.natural_key` 体系里没有 chunk id）。
把 2814 个 chunk 全部投影成实体，会让图的大小翻倍且污染「代码知识图」的语义
——这是重复建设，不是复用。

**为什么不做三张表（Document/Chunk/Business）：** 关系的主体是 Chunk；
文档级关系可以退化为「该文档任一 chunk 到另一文档任一 chunk」的聚合视图，
不必单独存一份，否则两边要同步。

### 3.3 章节级/业务级关系：复用图

已有 `Section` 实体（100 个），因此 `Section -REFINES-> Section` 这类**章节级**关系
放 `graph_meta.relationship` 即可，不需要新表。

判定规则（写进文档，避免两头乱放）：

| 关系粒度 | 存哪 | 例子 |
|---|---|---|
| 文档内部层级 | `doc_node.parent_id` | 章节 → 段落 |
| 章节 ↔ 章节、章节 ↔ 代码/测试 | `graph_meta.relationship` | 概要 §2 `REFINES` 详细 §3；详细 §3 `IMPLEMENTS` `UserEditAction` |
| 段落 ↔ 段落（含跨文档精确引用） | `corpus.chunk_relation` | 详细 CH-202 → 测试 CH-301 |

### 3.4 关系类型词表（映射，不照抄）

| 题目建议 | 采用 | 落点 | 说明 |
|---|---|---|---|
| `DERIVED_FROM` | ✅ | graph（Section）+ chunk_relation | 下层文档派生自上层 |
| `REFINES` | ✅ | 同上 | 细化同一主题 |
| `IMPLEMENTS` | ✅ | graph（Section→Class/Method） | 文档描述实现 |
| `REFERENCES` | ✅ 已有 | graph | 已存在（Screen→Tag 用过） |
| `TESTS` | ⚠️ 用 `VERIFIES` | graph | **已有 32 条**，不新建同义词 |
| `VALIDATED_BY` | ⚠️ 用 `VERIFIES`（反向） | graph | 同上，用方向区分 |
| `DEPENDS_ON` | ✅ 已有 | graph | 已存在 |
| `RELATED_TO` | ✅ 新增 | chunk_relation | 弱关联，`confidence` 必须给 |

**新增同义词是禁止的**：`TESTS` / `VALIDATED_BY` 与既有 `VERIFIES` 语义重叠，
再造一个只会让「查不到」变成常态。

### 3.5 需要在 `graph_meta.relation_type` 登记的类型

`DERIVED_FROM` / `REFINES` / `IMPLEMENTS` / `RELATED_TO`（`TESTS` 不登记）。
登记走已有的 Ontology 编辑流程（`PUT /graph/ontology/relation-types`，已实现），
**不写迁移**——本体是数据，不是 schema。

---

## 4. 统一接口

### 4.1 请求

```rust
pub struct RetrievalRequest {
    pub query: String,
    /// 显式策略；None 表示按 §5.6 的规则自动选择
    pub strategy: Option<RetrievalStrategy>,
    pub document_ids: Vec<DocumentId>,
    pub document_types: Vec<DocType>,        // 复用现存 DocType 枚举
    pub business_function_ids: Vec<String>,  // 现阶段映射到 business_domain
    pub top_k: i64,                          // 1..=100，与现有一致
    pub similarity_threshold: Option<f32>,   // 向量余弦下限
    pub graph: GraphOptions,                 // depth/direction/relations/limit/confidence
    pub context: ContextOptions,             // token_limit/include_hierarchy
    pub filters: SearchFilters,              // 复用现有（doc_types/business_domain/revision_id）
}

pub struct GraphOptions {
    pub depth: i32,                 // 1..=3，超过按 3 处理并记 degraded
    pub direction: Direction,       // OUT / IN / BOTH
    pub relationship_types: Vec<String>,
    pub max_related: i64,
    pub min_confidence: f32,
}
```

与现有字段的对应：`top_k`、`filters`、`document_types→doc_types` 直接复用；
`query` 走同一套 `normalize_nfkc`，保证与 `/search` 结果一致。

### 4.2 响应

```rust
pub struct RetrievalResponse {
    pub retrieval_id: Uuid,                  // 对应 retrieval_run.id
    pub query: String,
    pub selected_strategy: RetrievalStrategy,
    pub strategy_reason: String,             // 为什么选它（可解释）
    pub chunks: Vec<RetrievedChunk>,         // 已排序、已去重
    pub relationships: Vec<RelationPath>,    // 图路径，含边类型与方向
    pub sources: Vec<SourceDocument>,        // 版本/标题/类型/文件名
    pub context: ContextBlock,               // 给 AI 的证据块
    pub trace: RetrievalTrace,
    pub stats: RetrievalStats,               // 各阶段耗时、候选数
}

pub struct RetrievedChunk {
    pub chunk_id: ChunkId,
    pub document_id: DocumentId,
    pub revision_id: RevisionId,
    pub quote: String,                       // 原文，不改写
    pub locator: serde_json::Value,          // 页码/Sheet/章节/行号
    pub node_path: Vec<String>,              // 层级路径
    pub score: f64,                          // 融合后
    pub raw_scores: Vec<RawScore>,           // 每路原始分，不丢
    pub recall_sources: Vec<String>,         // vector / lexical / graph / hierarchy
    pub evidence_id: Option<EvidenceId>,
    pub via: Option<RelationPath>,           // 若是图扩展来的，记录怎么来的
}
```

`raw_scores` 是硬要求：**融合后必须能回看每一路的原始分**，
否则「为什么这条排第一」永远说不清（现有一 `score` 字段做不到）。

### 4.3 端点

| 方法 | 路径 | 说明 |
|---|---|---|
| POST | `/api/v1/projects/{pid}/retrieval:query` | 统一入口 |
| GET | `/api/v1/retrieval-runs/{id}` | 读 trace（管理界面用） |
| GET | `/api/v1/projects/{pid}/retrieval-runs?limit=` | 列表 |

`/search` 保留不动（前端在用），内部改为 `strategy=GLOBAL` 的薄封装，
避免两套逻辑漂移。**这是本次唯一会动到既有接口的地方**，需回归 phase2/phase5 的 UI 检查。

---

## 5. 策略

### 5.1 GLOBAL

`vector + lexical → RRF → rerank`，即现有链路，额外：填满 `recall_sources`、
把 `similarity_threshold` 真正应用（现在只有 `top_k`）。

### 5.2 DOCUMENT_LEVEL

在 GLOBAL 之上加 `doc_type` 过滤。关键：**类型来自 `document.doc_type`，不看文件名**
（实测该列存在，且 `BASIC_DESIGN` 等枚举已在 `DocType`）。

若问题里出现「詳細設計」「基本設計」等词，由策略选择器映射到 `DocType`；
映射表放 `engine-retrieval`（纯数据，可单测）。

### 5.3 HIERARCHICAL

入口是「命中的 chunk」，不是根节点：

```
命中 CH-202
  → 向上：chunk.node_path → doc_node 逐级取 SECTION 文本（作为该章节的「摘要」）
  → 同级：同 parent_id 下前后各 N 个 chunk（用 ordinal，不重跑向量）
  → 向下：parent_id = 命中章节的子 chunk（有上限）
```

- 全部走 SQL（`doc_node` + `chunk.node_path`），**不查图的 CONTAINS 边**（那是投影副本）。
- 扩展量：`max_siblings`、`max_children` 默认各 3，记入 `degraded` 若被截断。
- 「先摘要后正文」：第一轮只取 SECTION 文本，第二轮才展开子 chunk；
  两轮之间不做向量重算（用 `node_path` 前缀匹配）。

### 5.4 CROSS_DOCUMENT（重点）

两个来源，**先本地后图**：

1. `corpus.chunk_relation` 直连（精确、带证据）。
2. `graph_meta.relationship` 上的章节级关系（`REFINES`/`IMPLEMENTS`/`VERIFIES`），
   把命中的 chunk 映射到所属 `Section` 实体，再按 `depth/relations/confidence` 扩展，
   最后把目标 Section 下的 chunk 取回。

映射方式（已核对）：`Section.natural_key` **就是** `doc_node.id`，
且 `properties` 里带 `document_id` / `revision_id` / `locator` / `node_type`。
因此从命中的 chunk 取所属章节实体的路径是：

```
chunk.node_path 末段 → doc_node.id → graph_meta.entity(entity_type='Section', natural_key=id)
```

不需要额外建映射表。

遍历复用 `engine-impact` 的 `TraversalOptions`（max_depth/hub_threshold）与截断语义，
**不新写 BFS**。

返回时必须带 `RelationPath`（边序列 + 方向 + 置信度 + 证据 id），
并把「因超限被截断」写进 `degraded`。

### 5.5 HYBRID

```
各路候选（vector / lexical / graph / hierarchy）
  → 归一化：RRF（现有 fuse），另外可选 MIN_MAX 供调试对比
  → 权重：{vector, lexical, graph} 可配（默认 1:1:0.8）
  → rerank（现有规则型；可换成外部模型，见 §5.7）
  → 去重：chunk_id，保留最高分并合并 recall_sources
```

**禁止把不同来源的相似度直接相加**：向量余弦 ∈[-1,1]、ts_rank 无上界、
图扩展没有分数。RRF 本来就是为「不可比的分数」设计的，延续它，
并在 `raw_scores` 里保留原值供排查。

### 5.6 策略选择（规则优先，不依赖 LLM）

| 判据 | 策略 |
|---|---|
| 问题含「影响」「波及」「修正后」「変更したら」 | `CROSS_DOCUMENT` |
| 含「比較」「一致」「差分」 | `CROSS_DOCUMENT` + `HIERARCHICAL` |
| 含文档类型词（概要/基本/詳細/テスト設計） | `DOCUMENT_LEVEL` |
| 含「章」「節」「この画面の項目」等指代 | `HIERARCHICAL` |
| 其余 | `GLOBAL` |

规则命中即返回，并把命中规则写进 `strategy_reason`（可解释、可回归测试）。
预留 `trait StrategySelector`，后续接模型只换实现。

---

## 6. Context Builder

输入：`Vec<RetrievedChunk>`（已排序）；输出：`ContextBlock{text, tokens, entries[]}`。

规则：

1. **按文档层次分组**：同一 `document_id` 的 chunk 聚在一起，组内按 `node_path` 排。
2. **组间按最高分排序**，组内保持文档顺序（让 AI 读起来像在读一份文档）。
3. **每条必须带**：`document title` / `revision`（版本号）/ `locator`（页/Sheet/章节/行）/ `chunk_id` / `evidence_id`。
4. **去重**：`content_hash` 相同或 `quote` 前缀重合 >80% 视为重复，只留分高者。
5. **Token 预算**：`context_token_limit`（默认 3000）。超出时先丢「低分 + 无语义邻居」的 chunk，
   不截断单条内容（截断会产生断章取义的证据）。
6. **禁止**：不生成摘要、不改写原文、不把模型生成的文字混进 `entries`。
   `ContextBlock.text` 只用原文拼接 + 来源标注。

---

## 7. Retrieval Trace

现有 `corpus.retrieval_log` 是「一次搜索」的扁平日志，放不下多阶段。
新增两张表（与 `retrieval_log` 并存，后者继续服务 `/search`）：

```sql
CREATE TABLE corpus.retrieval_run (
    id uuid PRIMARY KEY, project_id uuid NOT NULL,
    query text NOT NULL, strategy text NOT NULL, strategy_reason text,
    request jsonb NOT NULL, response_stats jsonb NOT NULL,
    degraded text[] NOT NULL DEFAULT '{}',
    latency_ms integer NOT NULL, created_by uuid, created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE corpus.retrieval_stage (
    id uuid PRIMARY KEY, run_id uuid NOT NULL REFERENCES corpus.retrieval_run(id) ON DELETE CASCADE,
    seq integer NOT NULL, stage text NOT NULL,       -- vector / lexical / graph / hierarchy / fuse / rerank / context
    input_summary jsonb NOT NULL, output_summary jsonb NOT NULL,
    candidates integer NOT NULL, kept integer NOT NULL,
    latency_ms integer NOT NULL
);
```

界面（复用现有 `/impact`、`/graph` 的页面骨架）：

- 原始 query、选中策略 + 原因
- 每个阶段的候选数/保留数/耗时（就是上面的 `retrieval_stage`）
- 每条结果的 `raw_scores` 与 `recall_sources`
- 图扩展的关系类型与路径
- 最终 context 文本 + token 数 + 被丢弃的条目数

**默认不落原文**：`input_summary` 存 query 的 hash 与长度，避免把敏感文档内容写进日志表；
需要时按 `retrieval_id` 关联 `chunk` 回读。

---

## 8. MCP Adapter

### 8.1 tool 定义（6 个）

| tool | 对应策略/动作 |
|---|---|
| `search_knowledge` | GLOBAL / HYBRID |
| `search_by_document_type` | DOCUMENT_LEVEL |
| `search_hierarchical` | HIERARCHICAL |
| `expand_related_chunks` | CROSS_DOCUMENT（只扩展，不检索） |
| `trace_document_lineage` | 沿 `DERIVED_FROM`/`REFINES` 向上/向下 |
| `analyze_impact` | 复用现有 impact 能力（已有独立实现，这里只做转发） |

### 8.2 约束（写进代码，不靠自觉）

- MCP 层**不 import `store`、不 import `sqlx`**。用 `cargo deny`-style 的 CI 检查：
  `crates/mcp-server/Cargo.toml` 的依赖里出现 `store` 即失败（一个 grep 就能做）。
- 每个 tool 的实现体不超过「参数转换 + 一次调用 + 结果映射」；
  超过 30 行视为逻辑泄漏，评审拒绝。

### 8.3 与引擎的耦合方式

两种可选：

- **A（推荐）**：MCP 走 HTTP 调 `/retrieval:query`。进程隔离，可单独部署、单独限流。
- B：同进程直调 `app::retrieval`。省一次序列化，但 MCP 部署会绑死 API。

选 A，理由：题目要求「后续可以更换 MCP 实现，不影响 Retrieval Engine」，
只有进程边界能保证这一点。

---

## 9. 测试与评估

### 9.1 单元（无 DB）

`engine-retrieval` 全纯函数：

- 归一化/融合：给定两路分数与权重，断言顺序；`raw_scores` 不丢。
- 策略选择：题目给的 4 个例句 → 期望策略（直接作为用例）。
- Context Builder：超预算时丢谁、层级分组顺序、去重阈值。
- 关系词表映射：`TESTS` → `VERIFIES`。

### 9.2 集成（真 DB）

用现有 `scripts/verify-db.sh` 建的库 + 现成 100 页文档：

- 层级：命中某 chunk → 向上必得 SECTION、向下不越界。
- 跨文档：造 2 个 chunk_relation（一条 MANUAL 一条 RULE）→ 断言只返回置信度达标者，
  且 `RelationPath` 完整。
- 融合：同一 query 跑 GLOBAL 与 HYBRID，断言 HYBRID 的召回集是超集。
- 截断：`max_related` 设 1 → `degraded` 里出现截断标记（不能静默丢）。

### 9.3 评估（Phase 7）

测试集：20–30 题，覆盖 4 类（事实型 / 文档定位型 / 层级型 / 跨文档影响型）。
标注：每题给出「应命中的 chunk_id 集合」。

对比 5 种组合：Vector / Vector+Metadata / Vector+Graph / Hierarchical / Hybrid。

指标：Recall@K、Precision@K、MRR、nDCG@10、context 完整率、证据覆盖率、P95 延迟。
产出沿用现有 eval 风格：`data/retrieval-eval/report.json` + 一张对比表。

**验收线（先定，避免自说自话）**：Hybrid 的 Recall@10 不低于 Vector 单路的 +10%，
且证据覆盖率 ≥95%；否则不认为 Hybrid 有效。

---

## 10. 分阶段计划

| Phase | 内容 | 完成标志（可验收） |
|---|---|---|
| 1 | 本文档 + 现状复核 | 表/字段/代码位置逐条对上；与本节不符处更新 |
| 2 | 契约 + GLOBAL + Metadata + 去重 + Trace | ✅ **已实现**（2026-11-20）：`scripts/retrieval-eval.py` 验证同序、策略、未实现标记、trace 两表 |
| 3 | 图扩展（章节级）+ `chunk_relation` + 跨文档 | ✅ **已实现**（2026-11-20）：置信度过滤、上限截断标记、`via` 路径均验证 |
| 4 | 层级检索 | 命中→上/下/同级扩展开关生效；截断写 degraded |
| 5 | Hybrid + Reranker trait | 权重可配；单测覆盖融合顺序；rrf/minmax 可切换 |
| 6 | MCP Adapter | 6 个 tool 可被发现；MCP 进程无 store 依赖 |
| 7 | 评估 | 5 种组合的对比表 + 是否达到 §9.3 验收线 |

依赖：2 是 3/4 的前提；5 依赖 3/4（要有图与层级候选才能融合）；6 可在 2 之后并行。

---

## 11. 明确不做（防过度设计）

1. **不引入新的向量库**（不装 Qdrant/Milvus）。pgvector 够用，换库是运维决策不是检索需求。
2. **不重写 chunk**。现有 `PARAGRAPH` 切分 + `node_path` 已能支撑层级，
   重切会让所有 `evidence.quote_hash` 失效。
3. **不让 LLM 参与策略选择**。先用规则跑通并回归，模型只在 Phase 7 之后作为可选实现。
4. **不做自动的关系抽取**。`chunk_relation` 初期只接受 MANUAL/RULE 两种来源；
   自动抽取的准确率不足时会把「查不到」变成「查错」。
5. **不把文档层级迁到图里**（也不反向迁移）。层级只读 `doc_node`；
   图里的 `CONTAINS` 仅用于展示，检索路径不经过它。

---

## 附：与现有代码的落点

| 新增 | 位置 |
|---|---|
| 契约 | `crates/contracts/src/retrieval.rs` |
| 纯算法 | `crates/engine-retrieval/src/{lib,fuse,strategy,context}.rs` |
| 编排 | `crates/app/src/retrieval.rs` |
| 路由 | `crates/api/src/routes/retrieval.rs` |
| 存储 | `crates/store/src/knowledge.rs`（加 `chunk_relations` 读写） |
| 迁移 | `migrations/034_chunk_relation.sql`、`035_retrieval_run.sql` |
| MCP | `crates/mcp-server/src/main.rs`（新 crate） |
| 评估 | `scripts/retrieval-eval.py` |
