# 第四部分：核心架构设计

本部分逐个设计十个核心模块。每个模块统一按「职责边界 → 核心模型 → 关键流程 → 接口 → 失败模式 → 验收」展开。

一个贯穿全篇的约定：**Engine 不知道 Provider 是谁**。所有 Engine 只依赖 trait 与领域类型，任何厂商名词出现在 Engine 层都视为设计缺陷。

---

## 4.1 RAG Engine

### 职责边界

| 负责 | 不负责 |
|---|---|
| 文档结构建模（Document → Node → Chunk） | 文件上传与存储（属 app/store） |
| 切块策略、元数据抽取、实体链接 | 文档格式解析（属 ParserProvider） |
| 混合检索、融合、重排、上下文组装 | 向量计算（属 EmbeddingProvider） |
| 证据定位与引用生成 | 权限判定（属 capability，RAG 只接受过滤条件） |
| 索引版本与增量更新 | 图遍历（属 graph-engine） |

### 核心模型

```text
document
  └── document_revision (revision_id, source_revision, acquired_at, status)
        └── doc_node (node_id, parent_id, node_type, locator, ordinal, text, meta)
              ├── node_type: DOCUMENT | SECTION | PARAGRAPH | TABLE | TABLE_ROW
              │              | FIGURE | CODE | LIST | SHEET | CELL_REGION
              └── locator: 结构坐标（见下）
                    └── chunk (chunk_id, node_ids[], chunk_type, text, token_count, content_hash)
                          ├── chunk_embedding (model_id, vec)
                          └── chunk_fts (tsvector)
```

`locator` 是证据定位的唯一依据，按来源类型不同：

```json
{"kind":"pdf","page":42,"bbox":[72,118,540,196]}
{"kind":"docx","section_path":["3.2 画面項目定義","3.2.1 住所"],"paragraph":14}
{"kind":"xlsx","sheet":"画面項目定義","range":"C12:F12","anchor_cell":"C12","merged_span":"C12:F12"}
{"kind":"code","file":"src/main/java/.../UserEditAction.java","line_start":128,"line_end":131,"symbol":"update"}
{"kind":"markdown","heading_path":["設計","住所チェック"],"line_start":88}
```

### 关键流程（18 步 → 5 段）

| 段 | 步骤 | 实现要点 |
|---|---|---|
| **A. 摄取** | 1 摄取 / 2 解析 / 3 结构抽取 / 4 分类 | 内容哈希去重；解析器版本写入 `parse_run`；结构树必须保留父子关系，不能只有扁平文本 |
| **B. 建模** | 5 语义切块 / 6 元数据 / 7 实体链接 | 切块不跨结构边界；元数据含 project / doc_type / business_domain / screen / lang / acl |
| **C. 索引** | 8 Embedding / 9 向量索引 / 10 全文索引 / 11 索引版本 | 批量、可中断、断点续跑；索引构建有独立 `index_build_id` |
| **D. 检索** | 12 混合检索 / 13 重排 / 14 证据 / 15 上下文组装 / 16 引用 | RRF 融合；重排可缺席（降级但要标注）；引用必须带 locator |
| **E. 演进** | 17 反馈 / 18 增量更新 | 点击/采纳作为弱标签；文档 diff 驱动增量，不全量重建 |

### 切块规则（按格式，不是按固定 token）

| 来源 | 切块单元 | 大小 | 重叠 | 特殊处理 |
|---|---|---|---|---|
| PDF 正文 | 章节 → 段落组 | 300~800 token | 0（同章节内），跨章节不重叠 | 页眉页脚剔除；跨页段落合并 |
| PDF 表格 | 整表 | ≤ 1200 token | — | 超长表按 20 行分组，每组重复表头 |
| DOCX | 标题层级 | 300~800 | 0 | 编号列表保持完整 |
| XLSX | **单元格区域（region）** | 见下 | 0 | 合并单元格展开；公式与缓存值都保留 |
| 代码 | 符号（类/方法） | 按符号 | 0 | 不按行切；签名 + 注释 + 方法体为一个 chunk |
| Markdown | 标题层级 | 300~800 | 0 | 代码块不拆 |
| 图片 | 图 + 题注 + OCR | ≤ 400 | — | 图单独成 chunk，避免污染正文 |

**XLSX 切块的具体算法**（这是本项目最特殊的部分）：

```text
1. 取 sheet 的 used range
2. 识别「区域」：按空行/空列切分连续非空块（允许 1 行/列空隙）
3. 识别表头：区域前 3 行中，满足以下任一条即为表头
   - 整行非空且单元格类型为字符串
   - 存在合并单元格横跨该行
   - 下一行数据类型与上一行显著不同（字符串 → 数值/日期）
4. 生成 chunk：
   - 区域 ≤ 30 行：一个 chunk（表头 + 全部行）
   - 区域 > 30 行：按 20 行分组，每组 chunk 头部重复表头
   - 每个 chunk 的 text 渲染为「表头: 值」的键值对形式，而非 CSV
5. locator 记录 sheet + range；行级 chunk 额外记录 row_index
6. 合并单元格：值挂在左上角，其余单元格写入 merged_from 引用，
   渲染时展开为实际值（否则会出现大量空值，检索失效）
```

举例：`画面項目定義` sheet 的 `C12:F12` 是「住所 / 全角20文字 / 必須 / エラー時msg」。切块后 chunk 文本是：

```text
項目名: 住所
入力形式: 全角20文字以内
必須: 必須
エラー時メッセージ: 住所を正しく入力してください
```

而不是 `住所 | 全角20文字以内 | 必須 | ...`——后者在检索「住所 必須」时匹配度低，且人读起来也不像规格。

### 日语文本的三个技术细节

**1）长度单位不是「词数」**

切块预算按 **embedding 模型的 token** 计算，不按空白分词。日语 1 token 约对应 1~1.5 个字符（模型而异），800 token 大约是 500~700 字。`kb.chunk` 同时存 `token_count` 与 `char_count`，前者用于预算，后者用于 UI 展示（「約 600 文字」比「800 tokens」直观）。

**2）归一化双轨制**

| 用途 | 处理 | 存哪里 |
|---|---|---|
| 匹配（索引/查询） | NFKC + 旧字体映射（髙→高、﨑→崎）+ 半角カナ→全角 | `tokenized` |
| 展示 | 原文（NFC），不做任何替换 | `chunk.text` |
| 证据校验 | 原文的原始字节哈希 | `evidence.quote_hash` |

绝不能用归一化后的文本做证据引用——用户点开原文看到的是 `髙橋`，报告里写着 `高橋`，信任瞬间归零。

**3）PDF 的纵書き与ルビ**

- `locator` 增加 `writing_mode: horizontal | vertical`；纵書き的 bbox 按列切分，前端高亮按列而非按行。
- ルビ（振り仮名）作为独立文本层，不并入正文——并入会污染检索（「東京」旁边的小字「とうきょう」会让 chunk 变成两种读法混杂）。
- 扫描件走日文 OCR，OCR 结果标记 `ocr_confidence`；低于阈值的内容不进入证据，只作为「可参考」。

### 检索流程

```text
query
 ├─ 归一化：全角/半角、旧字体、大小写（仅用于匹配，不改原文）
 ├─ 查询分析：Lindera 分词 / 实体识别（命中 ontology 别名表）
 ├─ 词法路由（ADR-007）：
 │    ├─ 标识符 / 引号短语 / 短输入（≤4 字）→ BIGRAM（部分一致）
 │    └─ 自然语言 → TOKENIZED（词项），pg_bigm 可用时两路并行
 ├─ 过滤器构建：project_id（强制） + doc_type + business_domain + revision + acl
 ├─ 并行：
 │    ├─ 向量检索：HNSW top 60（iterative_scan = relaxed_order）
 │    ├─ 词法检索：ts_rank_cd / pg_bigm top 60
 │    └─ 图辅助：实体 → 关联 chunk（boost，不作硬过滤）
 ├─ RRF 融合：score = Σ 1/(60 + rank_i)
 ├─ 重排：top 40 → RerankProvider → top 12（缺失时跳过并标注 degraded）
 ├─ 邻域扩展：命中 chunk 的兄弟节点 / 表格上一行，避免语义断裂
 └─ 上下文组装：token 预算内去重、按文档聚簇、生成引用编号
```

词法路由的完整规则与基准计划见 `reference/japanese-search.md`。响应必须返回 `lexical_mode`（`tokenized` / `bigram` / `both`），便于事后判断「没搜到」是索引问题还是路由问题。

三个必须写死的规则：

1. **图辅助是加权，不是过滤**。用图做硬过滤会把「文档里提到但图上没连」的证据全部丢掉，这正是影响分析最需要的长尾。
2. **重排缺失必须显式标注**。`search_response.degraded = ["rerank_unavailable"]`，UI 上显示「本次结果未重排」。
3. **同文档聚簇**。上下文里来自同一文档的 chunk 放一起，避免模型把两份文档的规则混为一谈。

### Provider 接口

```rust
#[async_trait]
pub trait RagProvider: Send + Sync {
    fn capabilities(&self) -> RagCapabilities;

    async fn search(
        &self,
        ctx: &ProviderCtx,
        req: SearchRequest,
    ) -> Result<SearchResponse, ProviderError>;

    async fn hybrid_search(
        &self,
        ctx: &ProviderCtx,
        req: HybridSearchRequest,
    ) -> Result<SearchResponse, ProviderError>;

    async fn get_evidence(
        &self,
        ctx: &ProviderCtx,
        req: EvidenceRequest,
    ) -> Result<Evidence, ProviderError>;

    async fn delete_scope(
        &self,
        ctx: &ProviderCtx,
        req: DeleteScopeRequest,
    ) -> Result<DeleteResult, ProviderError>;
}
```

注意 `ingest` **不在** Provider 接口里。摄取是引擎自己的流程（解析、切块、元数据都在引擎内），只有「写索引」这一动作下推给 Provider。否则换一个 RAG Provider 就要重写整个摄取管线。

`RagCapabilities` 必须包含：`semantic / fulltext / hybrid / rerank / metadata_filter / graph_assist / evidence_locator`，未实现的能力在能力矩阵里显示 `Unsupported`，调用时返回 `CAPABILITY_UNSUPPORTED` 而不是静默降级。

### 失败模式

| 现象 | 根因 | 处理 |
|---|---|---|
| 检索「答非所问」 | 切块丢了表头/章节上下文 | 邻域扩展 + 父节点摘要 chunk |
| 日语查询召回低 | 分词粒度与索引不一致 | 索引与查询使用同一分词器版本；版本写入索引元数据 |
| 过滤后结果为空 | HNSW 过度过滤 | `iterative_scan`；必要时退化为「先过滤后暴力扫描」 |
| 证据点开错位 | locator 用了字符偏移而非结构坐标 | 只允许结构坐标；字符偏移仅作辅助 |
| 换模型后结果崩坏 | 新旧向量混用 | 索引按 `model_id` 隔离；切换走 rebuild |

### 验收

- 200 页 PDF + 50 sheet 的 XLSX 导入后，任取 50 个问题，top-5 命中率 ≥ 85%，且 100% 结果可跳转到原文。
- 任一 chunk 都能反查到 `document_revision` 与 `parse_run`。
- 删除文档后，检索不再返回其 chunk（逻辑删除立即生效，物理删除异步）。

---

## 4.2 Graph Engine

### 职责边界

Graph Engine 负责「关系怎么存、怎么查、怎么算路径」，不负责「关系是什么意思」（那是 Ontology）。

### 三层分离

```text
Ontology（类型系统）    ：有哪些实体类型、关系类型、约束、基数
Domain Model（域模型）  ：Entity / Relationship / Evidence / Revision 的 Rust 类型
Provider（存储与遍历）  ：Neo4j / PgGraph / 外部 Graph MCP
```

### 存储策略

PG 是权威源（ADR-002），Neo4j 是投影。写入路径：

```text
fact accepted
  → normalize to entity/relationship rows (PG)
  → outbox event
  → projector: MERGE (e:Entity {project_id, entity_id})
               MERGE (a)-[r:CALLS {rel_id}]->(b)
               ON CREATE / SET r.resolution, r.confidence, r.revision
```

Neo4j 节点只存「查询需要的属性」：`entity_id / project_id / entity_type / name / qualified_name / revision / resolution`。**不存大文本**，需要正文时回 PG 取。

### 方言约束：只用 Cypher 5 子集

Neo4j 5.26 LTS 只支持 Cypher 5；2026.x 起支持 Cypher 25（可与 Cypher 5 并存，按库或按查询指定）。为了同时兼容「默认 5.26、客户已上 2026.x」两种环境，**平台生成的 Cypher 必须是 Cypher 5 的语法子集**：

| 禁用（Cypher 25 专有） | 替代写法 |
|---|---|
| 新的 `MATCH` 简写、GQL 风格路径模式 | 使用显式 `()-[]->()` |
| 仅 25 支持的函数/过程 | 在 Rust 侧计算，或用 Cypher 5 等价函数 |
| 依赖 `CYPHER 25` 前缀的查询 | 不写前缀，按数据库默认 |

CI 里用 5.26 容器跑全部图查询测试；2026.x 的测试作为「兼容性验证」单独跑，不作为门禁。这样升级窗口到来时，切换成本只是改连接串。

### 查询能力

| 能力 | 语义 | 实现 |
|---|---|---|
| `entity_search` | 按名称/别名/类型模糊查 | PG trigram + Neo4j 索引双查，合并去重 |
| `neighbor` | 1 跳邻居（可按关系类型/方向过滤） | Cypher 单跳，`LIMIT` 保护 |
| `path` | 两实体间最短路径（≤ 8 跳） | `shortestPath` + 关系白名单 |
| `dependency_traversal` | 依赖方向遍历（正向/反向） | Rust 侧逐层 BFS，每层一次 Cypher |
| `impact_traversal` | 带方向规则与评分的多层遍历 | impact-engine 调用，见 4.5 |
| `subgraph` | 按条件导出子图 | 分批拉取，节点上限 5000 |
| `graph_compare` | 两个 revision 的结构差异 | 分别取边集，做集合差 |
| `ontology_query` | 查类型与约束 | 查 PG 的 ontology 注册表，不走 Neo4j |

**为什么遍历在 Rust 侧逐层做，而不是写一条变长 Cypher**：

```cypher
// 不采用：无法控制预算、无法评分、容易拖垮数据库
MATCH p = (n)-[:CALLS*1..8]->(m) RETURN p
```

逐层 BFS 的好处：可以设访问集、按边类型剪枝、按置信度衰减、限制每层节点数、中途取消。代价是多次往返——在 8 跳以内、每层 < 2000 节点时，实测优于一条巨型 Cypher。

### Cypher 安全

关系类型和标签是**不能参数化**的（Cypher 限制）。因此：

```rust
// 唯一允许的构造方式
let rel = ontology.relationship_type(&req.rel_type)?; // 枚举校验，非白名单直接报错
let cypher = format!("MATCH (a:Entity {{project_id:$pid, entity_id:$sid}})-[r:{}]->(b) ...", rel.as_cypher_ident());
// rel.as_cypher_ident() 只返回注册表中存在的标识符，且经 [A-Z_]+ 正则校验
```

绝不允许把用户输入拼进 Cypher 字符串。

### 版本与时间

每个实体/关系带：

```text
revision         首次出现的 revision
last_seen_rev    最后出现的 revision（用于判断是否已删除）
valid_from       逻辑有效期起点
valid_to         逻辑有效期终点（NULL = 当前有效）
```

查询默认 `valid_to IS NULL`。历史对比用 `AS OF revision` 语义：取 `valid_from <= R AND (valid_to IS NULL OR valid_to > R)`。

### 失败模式

| 现象 | 处理 |
|---|---|
| 图与 PG 不一致 | 每日漂移检测；差异 > 0.1% 触发重建 |
| 巨型 hub 节点（一张表被 800 个类引用） | 遍历时识别 `degree > threshold`，标记为 `HUB`，要求人工缩小范围，不自动展开 |
| 关系类型拼写错误 | 写入前经 Ontology 校验，未知类型直接拒绝 |
| 子图导出 OOM | 分批 + 节点上限 + 流式 NDJSON 返回 |

---

## 4.3 Ontology

### 三种「本体」不要混为一谈

| 类型 | 内容 | 存储 | 谁改 |
|---|---|---|---|
| **Schema Ontology** | 实体类型、关系类型、属性定义、约束、基数 | PG 表（版本化） | 平台管理员，需审批 |
| **Instance Graph** | 具体项目里的实体与关系 | PG + Neo4j 投影 | 分析器自动写入 |
| **Business Taxonomy** | 业务领域、功能分类、画面分组 | PG 表（项目级） | 业务分析师 |

Prompt 里列出的 Business / Document / Application / Code / Database / Testing / AI 七个域属于 Schema Ontology；某个项目的「用户管理」模块属于 Instance Graph。

### Schema 用数据表达，不用代码

```sql
ontology_version(id, version, status, published_at, published_by)
entity_type(id, ontology_version_id, name, display_name_ja, parent_type, is_abstract, properties_schema)
relation_type(id, ontology_version_id, name, display_name_ja, source_types[], target_types[],
              cardinality, direction, is_transitive, properties_schema)
constraint(id, ontology_version_id, kind, expression, severity)
```

`properties_schema` 是 JSON Schema，用于校验实例属性。

**为什么不硬编码成 Rust enum**：客户 A 的「画面」可能叫 `Screen`，客户 B 的「帳票」也要建 `Screen`；不同项目的实体类型会扩展。硬编码导致每接一个客户就改代码、发版。用数据表达后，新增类型只需发布新的 ontology version。

**代价**：失去编译期检查。缓解方式——核心类型（约 30 个）在 Rust 侧有对应常量与类型别名，CI 校验「代码中引用的类型名必须在默认 ontology 中存在」。

### 默认 Ontology（v1 摘要）

| 域 | 实体 |
|---|---|
| Business | BusinessDomain, BusinessFunction, BusinessRule, BusinessProcess, BusinessEvent |
| Document | Document, DocumentRevision, Section, Table, DesignElement, Requirement, Specification |
| Application | Application, Module, Package, Screen, Form, Action, Controller, Service, Component, API, Job, Batch |
| Code | Class, Method, Field, Interface, Dependency, Annotation, Configuration, SqlStatement |
| Database | Database, Schema, Table, Column, PrimaryKey, ForeignKey, View, StoredProcedure |
| Testing | TestPlan, TestScenario, TestCase, TestData, TestRun, TestExecution, TestResult, Defect, Coverage |
| AI | Agent, Skill, Prompt, Tool, Workflow, AnalysisSession |

关系类型（带方向语义）：

```text
结构：CONTAINS, DECLARES, EXTENDS, IMPLEMENTS
依赖：DEPENDS_ON, CALLS, REFERENCES, IMPORTS
数据：READS, WRITES, QUERIES, MAPPED_TO
行为：TRIGGERS, VALIDATES, HANDLES, DISPATCHES
文档：DESCRIBES, DEFINES, MENTIONS
追踪：IMPLEMENTED_BY, VERIFIES, GENERATED_FROM, DERIVED_FROM, AFFECTS
执行：EXECUTED_IN, PRODUCED
```

每个关系的实例必须带：`source_revision / resolution / confidence / evidence_ids[] / analysis_engine / created_at`。

### 演进规则

1. **只增不删**：废弃类型标 `deprecated_at`，不物理删除（历史 fact 还要引用）。
2. **版本发布需审批**：`OntologyPublish` 是受控操作（Prompt §12）。
3. **升级不回溯**：旧 fact 保留旧版本语义，新分析用新版本；报告里标注所用 ontology 版本。
4. **破坏性变更需迁移脚本**：改名/改基数必须提供 `migration`，并在影子图上验证。

---

## 4.4 Code Analysis Engine

### 分层

```text
Rust Coordinator
  ├─ 扫描：walkdir + ignore 规则 + 语言识别
  ├─ 变更检测：file_hash / tree_digest / git diff
  ├─ 调度：按语言/分析器分发，控制并发
  ├─ 归一化：所有分析器输出 → Fact
  └─ 持久化：fact 只追加 + supersede

Tree-sitter（Rust，进程内）
  └─ 语法结构、符号、import、注解、字符串字面量、简单调用

OpenRewrite Worker（Java，HTTP）
  └─ 类型归属、方法解析、继承/实现、跨文件符号

Framework Adapter（Rust，可插拔）
  └─ Struts1 / SAStruts / Seasar2 / Spring / MyBatis / JSP taglib

Config Analyzer（Rust）
  └─ XML / Dicon / YAML / Properties / CSV

SQL Analyzer（Java Worker 内 JSqlParser）
  └─ 表/列引用、SQL 类型（SELECT/INSERT/UPDATE/DELETE）、动态标签
```

### 为什么必须两套解析

Tree-sitter 快、容错、多语言，但**没有类型信息**。它只能告诉你 `userService.update(x)` 这个调用存在，不能告诉你 `userService` 是哪个类型、`update` 解析到哪个重载。

OpenRewrite 有类型归属，但慢、需要 classpath、只支持 Java。

所以：**语法层用 Tree-sitter 铺满，语义层用 OpenRewrite 打穿 Java**。两套结果在 Fact 层合并，冲突时以 OpenRewrite 为准（`resolution=RESOLVED`），Tree-sitter 的结果降级为 `INFERRED`。

### 统一 Fact 的产生规则

| 分析器 | 产出关系 | 默认 resolution |
|---|---|---|
| Tree-sitter 符号表 | `DECLARES`（类声明方法/字段） | RESOLVED（语法确定） |
| Tree-sitter 调用 | `CALLS`（仅方法名匹配，无类型） | INFERRED（可能同名不同类） |
| OpenRewrite 方法解析 | `CALLS`（精确到方法签名） | RESOLVED |
| OpenRewrite 继承 | `EXTENDS` / `IMPLEMENTS` | RESOLVED |
| Framework: Struts config | `HANDLES`（Action → path） | RESOLVED |
| Framework: 注解映射 | `HANDLES`（Controller → URL） | RESOLVED |
| Dicon DI | `DEPENDS_ON`（组件注入） | RESOLVED |
| 反射/字符串拼接 | `CALLS`（候选集） | UNRESOLVED 或 INFERRED |
| JSP form → Action | `SUBMITS_TO` | PARTIAL（依赖运行时） |
| MyBatis XML | `QUERIES`（Mapper → SQL → Table） | RESOLVED（XML 明确） |

### JSP 混合解析

JSP 没有 grammar，必须自建。策略：

```text
1. 用 HTML grammar 解析整体（JSP 本质是 HTML + 指令 + 脚本）
2. 预处理：把 <% ... %>、<%= ... %>、<%-- --%>、${...}、<jsp:*>、自定义 tag
   替换为等长占位符（保持行列号不变）
3. HTML 解析后，按占位符位置回填：
   - scriptlet → 提取 Java 片段，交给 Java 语法解析（不要求类型）
   - EL 表达式 → 提取变量路径（如 ${userForm.address}）
   - tag → 解析 taglib 声明，得到标签处理器类
4. 产出：Form 字段、name 属性、action 目标、include 关系、tag 使用
```

关键：**占位符必须等长**，否则所有行列号都会错位，证据定位全废。

### 反射与动态行为的处理

不猜。做法是「收集线索 + 标记不确定」：

| 模式 | 识别方式 | 输出 |
|---|---|---|
| `Class.forName("...")` | 字符串字面量 | `UNRESOLVED` + 候选类名 |
| `getMethod("...")` | 字符串字面量 | `INFERRED` + 候选方法名 + 所在类 |
| Spring `@Qualifier` + 接口多实现 | 类型 + 注解 | `INFERRED` + 所有实现（多候选） |
| Dicon 组件名拼接 | 配置文件字符串 | `INFERRED` |
| JS 动态 `eval` / 事件绑定 | 正则 + AST 有限分析 | `UNRESOLVED`，UI 提示「需人工确认」 |

每个不确定关系都带 `unresolved_reason` 与 `candidates[]`。UI 上这些边用虚线 + 问号显示，**不允许默认隐藏**。

### 文字编码：Shift_JIS 是默认风险点

日本遗留系统的源码与设计资料**默认不能假设 UTF-8**。JSP、Java、SQL、CSV、Properties 都可能是 Shift_JIS / Windows-31J，且同一项目内混用。

处理流程：

```text
读原始字节
  → BOM 检测（UTF-8/UTF-16）
  → 无 BOM：chardetng 探测 + 统计校验（非法字节率、假名/汉字比例）
  → confidence < 0.75：不猜，标记 ENCODING_UNDETECTED，文件级 UNRESOLVED
  → 转码为 UTF-8 供解析
  → 同时建立 UTF-8 字符位置 ↔ 原始字节偏移 的映射表（line_offsets）
```

三条硬规则：

1. **证据定位以原始字节为准**。`locator` 记录 `{file, line_start, line_end, byte_start, byte_end, encoding}`。只记行号不够——同一行在不同编码下字节长度不同，回读时无法精确高亮。
2. **不回写文件**。分析全程只读，转码只发生在内存与派生数据中。任何「顺手把文件转成 UTF-8」的行为都会污染客户的版本库。
3. **解析失败要显式**。`encoding_undetected` 的文件进入「未解析清单」，不参与关系推断，也不假装解析成功。

Shift_JIS 的具体坑：

| 现象 | 原因 | 处理 |
|---|---|---|
| `\` 被当成 `¥` | Shift_JIS 中 0x5C 在不同字体下显示为 ¥ | 按字节解析，不按显示字符 |
| 正则/转义错乱 | 0x5C 出现在多字节字符第二字节 | 转 UTF-8 后再做语法解析 |
| `①②③` `㈱` `髙` 丢失 | 机种依存文字 / 外字 | 保留原始字节；匹配时用映射表，不删除 |
| 行号偏移 | 转码后换行位置变化（极少但存在） | 用原始字节的换行位置建行号表 |

### OpenRewrite Worker 协议

```http
POST /analyze
Content-Type: application/json

{
  "project_root": "/workspace/proj",
  "revision": "abc123",
  "classpath": ["/deps/a.jar", "..."],
  "files": ["src/main/java/.../UserEditAction.java"],
  "options": { "type_attribution": true, "include_generated": false }
}

→ 200 application/x-ndjson
{"kind":"symbol","fqn":"com.example.UserEditAction","file":"...","line":12}
{"kind":"call","from":"...update","to":"...updateUser","resolution":"RESOLVED","evidence":{...}}
{"kind":"unresolved","reason":"NO_CLASSPATH","subject":"..."}
```

用 NDJSON 而非单个大 JSON：大项目的结果可能几百 MB，流式返回可以边收边写，避免 Worker OOM 和 Coordinator 内存峰值。

Worker 无状态，不写数据库。所有持久化在 Rust 侧。

### 增量

```text
变更文件集合 = git diff --name-status base..head
              ∪ (文件 hash 变化)
              ∪ (分析器版本变化涉及的文件)
              ∪ (classpath 变化时的全量)

对每个变更文件：
  1. 删除其旧 fact（标记 superseded，不物理删）
  2. 重新解析该文件
  3. 重建「以该文件符号为 subject 或 object」的边
  4. 对被删除的符号，标记其所有边 valid_to = now
```

不做「全量重扫」的例外：分析器大版本升级、ontology 破坏性变更、图漂移超阈值。

### 准确性度量

必须有金标集，否则「解析准确率」是空话。

- 从 SAMPLE 工程中人工标注 200 个调用关系（含 30 个反射/动态案例）。
- 每次分析器改动跑一遍，输出 precision / recall / 各 resolution 分布。
- 目标：RESOLVED 关系的 precision ≥ 98%（宁可少，不可错）；整体 recall ≥ 85%。

---

## 4.5 Impact Analysis Engine

### 输入

| 类型 | 例 | 解析方式 |
|---|---|---|
| Diff | `git diff base..head` | 文件+行 → 符号 |
| 手工指定 | 「改 UserEditAction.update」 | entity_id 直查 |
| 文档变更 | 设计书某条规则修订 | doc_node → 关联实体 |
| DB 变更 | `ALTER TABLE user ADD COLUMN` | DDL 解析 → Column 实体 |
| API 变更 | OpenAPI diff | path/operation → API 实体 |

### 种子解析（最容易出错的一步）

```text
文件 + 行号
  → 该文件在该 revision 的语法树
  → 找到包含该行的最小符号（方法 > 字段 > 类）
  → 若跨方法（改的是字段），种子包含「字段 + 所有读写该字段的方法」
  → 若改的是 SQL 字符串，种子包含「SQL 所属方法 + 该 SQL 解析出的表」
```

边界情况：改的是注释/空行 → 无种子，直接返回空报告并说明「变更不包含可分析逻辑」。改的是配置文件 → 种子是「受该配置影响的组件」（需 Framework Adapter 提供映射）。

### 遍历算法

```rust
fn analyze(seed: Vec<EntityId>, budget: Budget) -> ImpactGraph {
    let mut g = ImpactGraph::new();
    let mut visited = HashSet::new();
    let mut frontier = seed;

    for depth in 1..=MAX_DEPTH {
        let mut next = vec![];
        for node in frontier.drain(..) {
            if !visited.insert(node) { continue; }
            if budget.exhausted() { mark_truncated(&mut g, node); break; }

            // 方向 1：反向依赖（谁在用我）
            for e in graph.incoming(node, REL_UP[depth]) {   // CALLS/IMPLEMENTS/EXTENDS/REFERENCES
                g.add_edge(e, depth, Direction::Upstream);
                next.push(e.src);
            }
            // 方向 2：数据流（我写了谁）
            for e in graph.outgoing(node, REL_DATA) {         // WRITES/QUERIES
                g.add_edge(e, depth, Direction::Downstream);
                // 共享资源闭包：同一张表的其他写入者
                if e.dst.kind == Table && degree(e.dst) <= HUB_THRESHOLD {
                    for peer in graph.incoming(e.dst, REL_DATA) {
                        g.add_edge(peer, depth + 1, Direction::SharedResource);
                    }
                }
                next.push(e.dst);
            }
            // 方向 3：实现/分发（接口 → 实现）
            for e in graph.outgoing(node, REL_DISPATCH) {     // IMPLEMENTS/DISPATCHES
                g.add_edge(e, depth, Direction::Dispatch);
                next.push(e.dst);
            }
        }
        frontier = next;
        if frontier.is_empty() { break; }
    }
    g
}
```

三条硬约束：

1. **深度上限默认 6，可配但不超过 10**。超过 6 跳的关系在业务上已无法解释。
2. **Hub 节点不自动展开**。表/工具类/基类的入度超过阈值（默认 200），标记为 `HUB_TRUNCATED`，要求用户选择「只看直接消费方」或「指定业务域」。
3. **每条边保留来源**。最终报告里任意一条影响路径都能展开成 `A --CALLS--> B --WRITES--> T`，每条边带 evidence。

### 分层

| 层 | 内容 | 数据来源 |
|---|---|---|
| L1 直接影响 | 变更的文件/类/方法/配置/SQL | diff |
| L2 技术影响 | 调用方、被调用方、DB 读写、外部 API | 图遍历 |
| L3 业务影响 | 画面、业务功能、业务规则、流程 | 图上 `Screen/BusinessRule/BusinessProcess` 节点 + RAG 证据 |
| L4 测试影响 | 计划、场景、用例、数据、脚本 | 追踪矩阵 |
| L5 风险影响 | 高风险业务、历史缺陷、复杂度、未覆盖、不确定关系 | 规则 + 统计 |

### 风险打分（确定性部分）

```text
risk_score =
    0.30 * business_criticality      // 业务功能等级 1~5
  + 0.20 * impact_breadth            // min(影响项数/50, 1)
  + 0.15 * change_size               // min(变更行数/500, 1) + 文件数因子
  + 0.15 * defect_history            // 该模块近 12 个月缺陷密度（分位）
  + 0.10 * (1 - test_coverage)       // 该范围已有覆盖
  + 0.10 * uncertainty_ratio         // 路径中 INFERRED/UNRESOLVED 边占比
```

输出 `risk_score ∈ [0,1]` + `risk_factors[]`（每项贡献值）。UI 上必须能展开看到构成，不允许只给一个「高风险」标签。

AI 的介入仅限：

- 生成 `narrative`（给人读的解释）
- 提出 `hypotheses[]`（「可能还影响 X，因为……」，标 `INFERRED`，默认折叠）
- 优化测试建议的措辞

**AI 不得修改 `impact_items` 主表，不得调整 risk_score。**

### 输出结构

```json
{
  "impact_id": "IMP-2026-0007",
  "project_id": "PRJ-001",
  "base_revision": "abc123",
  "head_revision": "def456",
  "seeds": [{"entity_id":"E-1","kind":"Method","name":"UserEditAction.update"}],
  "items": [
    {
      "impact_id": "II-001",
      "level": "L2",
      "type": "CALLER",
      "direction": "upstream",
      "entity": {"id":"E-9","kind":"Class","name":"UserBatchService"},
      "path": ["E-1","E-5","E-9"],
      "path_edges": [{"rel":"CALLS","resolution":"RESOLVED","evidence":"EV-11"}],
      "resolution": "RESOLVED",
      "confidence": 0.92,
      "reason": "间接调用：Batch 经 CommonValidator 复用该校验",
      "risk": "medium",
      "related_tests": ["TC-101","TC-118"]
    }
  ],
  "stats": {"total": 37, "by_level": {"L1":4,"L2":18,"L3":9,"L4":4,"L5":2},
            "resolved": 31, "inferred": 5, "unresolved": 1},
  "truncated": {"hubs": 1, "depth": false, "budget": false},
  "generated_at": "2026-11-05T12:00:00Z",
  "engine_version": "impact-0.3.1",
  "ontology_version": "v1.4"
}
```

`truncated` 字段是可信度的关键：**必须告诉用户「这次分析有截断」**，否则报告看起来完整，实则漏项。

### 可复现

同一 `(base_revision, head_revision, engine_version, ontology_version, analyzer_versions)` 必须产出同一结果。做法：所有分析器版本写入报告；图遍历使用稳定排序（按 entity_id）；不使用时间戳参与计算。

---

## 4.6 Test Engineering Engine

### 生命周期

```text
ValidationRule ──┐
ImpactItem ──────┼─→ TestScenario ─→ TestCase ─→ TestData ─→ TestRun
BusinessProcess ─┘        │              │           │          │
                          └── evidence ──┴───────────┴──────────┘
                                                TestExecution ─→ TestResult
                                                                     │
                                                              Coverage / Defect
```

### 规则抽取

从三处抽取，合并去重：

| 来源 | 方法 | 例 |
|---|---|---|
| Java 校验代码 | 模式匹配 + 数据流（简单） | `if (s.length() > 20) errors.add(...)` |
| JSP / JS | 属性提取（`maxlength`、`pattern`、`required`）+ 函数体 | `<input maxlength="20">` |
| 设计书 | RAG 检索「必須/桁数/範囲/形式」段落 + 表格 | `全角20文字以内` |

冲突处理：代码与文档不一致 → 生成 `CONFLICT` 规则，**两条都保留**，并在测试里同时验证（这本身就是高价值发现）。

```rust
pub struct ValidationRule {
    pub id: RuleId,
    pub field: FieldRef,
    pub kind: RuleKind,          // REQUIRED | LENGTH | RANGE | PATTERN | CROSS_FIELD | CUSTOM
    pub condition: String,        // 结构化表达，如 "len(value) <= 20 && is_fullwidth(value)"
    pub source: RuleSource,       // CODE | SPEC | INFERRED
    pub evidence: Vec<EvidenceId>,
    pub resolution: Resolution,
}
```

### 场景派生

不是让 AI「想几个场景」，而是按规则组合 + 流程图：

| 场景类型 | 派生规则 |
|---|---|
| 正常系 | 每个业务流的主路径（从 `BusinessProcess` 图取） |
| 异常系 | 每个外部依赖（DB/API）的失败分支 |
| 边界值 | 每条 `LENGTH/RANGE` 规则的 min-1 / min / max / max+1 |
| 权限 | 每个 `Screen` 的角色访问矩阵差异 |
| 状态迁移 | `Status` 字段的所有合法/非法迁移 |
| 重复提交 | 表单 + POST 端点 → 幂等性场景 |
| 并发 | 同一资源的并发更新（标记为可选，需环境支持） |
| 回滚 | 多表更新 + 事务边界 → 中途失败场景 |

AI 的增值点：补充「业务上可能但图上没有」的场景（如「月末结算时批量与单件同时执行」）。这些进入 `suggested_scenarios`，**标 INFERRED，需人工采纳后才进入正式场景**。

### 用例结构

```yaml
id: TC-2026-0118
scenario_id: TS-0042
title: 住所に半角英字を入力した場合、エラーメッセージが表示される
preconditions:
  - ログイン済み（権限: 一般ユーザ）
  - ユーザ編集画面を開いている
steps:
  - action: fill
    target: {label: "住所"}
    value: "Tokyo"
  - action: click
    target: {role: button, name: "登録"}
expected:
  - type: text_visible
    value: "住所は全角20文字以内で入力してください"
  - type: field_error
    target: {label: "住所"}
evidence:
  - EV-7f3a   # 詳細設計書 画面項目定義 C12
  - EV-91bc   # UserEditAction.java:128
covers:
  rules: [RULE-USER-ADDR-001]
  impact_items: [II-003]
automation:
  type: playwright
  selector_strategy: role_first
```

`covers` 是覆盖计算的唯一依据。没有 `covers` 的用例不计入任何覆盖。

### 测试数据生成

按规则生成三组：`valid` / `invalid` / `boundary`。

| 规则类型 | valid | invalid | boundary |
|---|---|---|---|
| LENGTH(20) | 10 文字 | 21 文字、空 | 20 文字、19 文字、0 文字 |
| RANGE(1..999) | 500 | 0、1000 | 1、999 |
| PATTERN(全角) | 東京都 | Tokyo、東京1 | 全角1文字、全角20文字（混合半角） |
| REQUIRED | 値あり | 空、空白のみ | — |
| CROSS_FIELD(開始<終了) | 1/1〜12/31 | 12/31〜1/1 | 同日、1日差 |

日语特有的坑（必须内置）：

- 全角/半角数字：`１２３` vs `123`
- 全角空格 `　` 不等于半角空格
- 旧字体/異体字：`髙` `﨑` `邉`
- サロゲートペア（emoji、外字）在 `length` 与 `bytes` 上的差异
- 結合文字与濁点（`か` + `゛`）
- 日付和暦（令和）与西历混用

这些不是「边界值测试」的学术练习，是日本现场真实会出的 bug。

### 执行

Playwright Runner 使用 **JSON 步骤 DSL**，不是直接生成 `.spec.ts`：

| 方案 | 优点 | 缺点 | 结论 |
|---|---|---|---|
| 生成 TS 代码 | 灵活 | 需要沙箱执行任意代码；难以校验；版本升级即失效 | 不采用（除非客户明确要求） |
| JSON DSL + 通用 Runner | 可校验、可版本化、安全 | 表达能力有限 | **采用**；提供 `custom_step` 扩展点，默认禁用 |

Runner 职责：

```text
接收 {test_case_id, steps, data, env, artifacts_dir}
  → 启动 browser context（隔离 storage）
  → 逐步执行，每步超时 10s
  → 失败时：截图 + DOM 快照 + console log + trace
  → 返回 {status, step_results[], artifacts[], duration}
  → 产物上传对象存储，PG 只存引用
```

重试策略：仅对 `timeout` / `network` 类失败自动重试 1 次，断言失败不重试（重试会掩盖真实缺陷）。

### 覆盖口径（必须写死，不允许解释）

```text
Requirement Coverage = 至少 1 个关联用例「最近一次执行 = PASSED」的需求数 / 范围内需求数
Rule Coverage        = 至少 1 组数据「执行 = PASSED」的规则数 / 范围内规则数
Screen Coverage      = 至少 1 个用例「执行 = PASSED」的画面数 / 范围内画面数
Impact Coverage      = 至少 1 个验证用例「执行 = PASSED」的影响项数 / 影响项总数
Regression Coverage  = 受影响用例集中「执行 = PASSED」的比例

不计入覆盖：
  - 状态为 DRAFT 的用例
  - 从未执行过的用例
  - 执行结果为 BLOCKED / SKIPPED
  - 没有 covers 关联的用例
```

`Impact Coverage` 是平台独有的指标，也是「闭环是否成立」的直接度量。

代码覆盖率（JaCoCo）单独展示，不与上述混算。UI 测试无法测出后端分支覆盖率，把两者相加是自欺欺人。

---

## 4.7 Agent Runtime

### Agent 定义

```yaml
id: impact-analyst
version: 3
model_profile: reasoning          # 逻辑名，不是具体模型
prompt_ref: prompt://impact-analyst@3
capabilities:                     # 能力白名单，不是工具名
  - knowledge.search
  - knowledge.get_evidence
  - graph.neighbor
  - graph.path
  - code.get_symbol
budgets:
  max_tool_calls: 40
  max_tokens: 120_000
  max_wall_time_sec: 300
  max_cost_usd: 1.50
output_schema: schema://impact-narrative@1
approval_required: false
```

三个设计点：

1. **`model_profile` 而非模型名**。`fast` / `reasoning` / `long_context` 映射到 Provider 配置。换模型不改 Agent。
2. **`capabilities` 而非 tool 名**。MCP 工具改名、换 Server，Agent 定义不变。
3. **`output_schema` 强制**。Agent 输出必须是结构化对象，自由文本只能放在 `narrative` 字段里。

### Capability Gateway

```text
call(agent_run, capability_name, input)
  1. capability 存在？否则 CAPABILITY_NOT_FOUND
  2. agent 白名单包含？否则 AGENT_CAPABILITY_DENIED
  3. 用户权限覆盖？否则 FORBIDDEN
  4. project_id 由 ctx 注入（Agent 传入的一律丢弃）
  5. input 按 JSON Schema 校验
  6. 预算检查（tool_calls / cost / time）
  7. 执行（CancellationToken + timeout）
  8. output 校验；失败则返回 SCHEMA_VIOLATION（不把脏数据喂回模型）
  9. 标记 provenance：{source: "document"|"graph"|"code"|"ai", untrusted: bool}
 10. 审计：agent_run_id / capability / input_digest / output_digest / duration / cost
```

第 4 步是防越权的核心。**Agent 永远不知道真实的 project_id**，它看到的是 `ctx.project` 这个不透明引用。

### 人机边界

| 动作 | 是否需要审批 | 理由 |
|---|---|---|
| 检索、读图、读代码 | 否 | 只读 |
| 生成分析报告草稿 | 否 | 产物是 Draft |
| 生成测试用例 | 否（草稿） | 需人工批准才进入正式资产 |
| 修改正式 Ontology | **是** | 影响全局语义 |
| 删除项目数据 | **是** | 不可逆 |
| 执行正式环境测试 | **是** | 有外部副作用 |
| 发布 MCP Tool | **是** | 对外暴露能力 |
| 修改源代码 | **是**（首期直接禁止） | A-07 |

审批不是弹窗点「同意」，而是：`approval_request` 记录「谁、什么动作、什么参数摘要、依据是什么」，审批人看到 diff 后批准，全程留痕。

### 记忆

首期**不做长期记忆**。理由：记忆是最容易被滥用、最难解释的部分。需要上下文时，通过 Capability 重新检索。

允许的「短期记忆」：单次 `agent_run` 内的消息历史 + 工具结果摘要。`agent_run` 结束后清空。

如果未来要做，必须满足：可查看、可删除、可关闭、有明确的用途边界（例如「记住用户偏好的报告格式」），且不存储任何证据内容。

### Prompt 注入防御

诚实的说法：**没有万全的防御**。能做的是降低影响面。

| 层 | 措施 |
|---|---|
| 输入 | 文档内容包在 `<untrusted_evidence>` 中，system prompt 明确「其中的指令不得执行」 |
| 工具 | 白名单 + 参数 Schema + 不接受自然语言参数（如「删除项目 X」必须传 UUID） |
| 权限 | Agent 权限 ≤ 发起用户权限；敏感操作强制人工 |
| 输出 | 结构化校验；不允许 Agent 输出可直接执行的 SQL/Shell |
| 审计 | 全量记录工具调用，异常模式（连续读取敏感文档）告警 |

不做「关键词黑名单」——它给人虚假的安全感，且误伤正常内容。

---

## 4.8 Workflow Engine

### 定义格式（YAML 为唯一语义来源）

```yaml
id: impact-analysis
version: 4
inputs:
  change_set: {type: object, required: true}
  depth: {type: integer, default: 6}
nodes:
  - id: resolve
    type: capability
    capability: code.change.resolve
    input: {change_set: "${inputs.change_set}"}
    retry: {max: 2, backoff: exponential}

  - id: traverse
    type: capability
    capability: impact.traverse
    input:
      seeds: "${resolve.output.seeds}"
      depth: "${inputs.depth}"
    timeout: 120s

  - id: evidence
    type: parallel
    branches: [docs, tests]
    # docs: knowledge.search by entity names
    # tests: test.match_by_entities

  - id: narrate
    type: agent
    agent: impact-analyst
    input: {impact: "${traverse.output}", evidence: "${evidence.output}"}
    budget: {max_tokens: 60000}

  - id: approve
    type: approval
    when: "${traverse.output.stats.unresolved > 0}"
    assignee_role: impact_reviewer

  - id: publish
    type: capability
    capability: impact.publish
    input: {report: "${narrate.output}"}
edges:
  - {from: resolve, to: traverse}
  - {from: traverse, to: evidence}
  - {from: evidence, to: narrate}
  - {from: narrate, to: approve}
  - {from: approve, to: publish, when: "approved"}
  - {from: narrate, to: publish, when: "!needs_approval"}
```

### 节点类型

`capability` / `agent` / `condition` / `parallel` / `loop` / `approval` / `subworkflow` / `delay` / `manual_task`。

不做的：任意脚本节点（安全风险）、动态代码执行、无限循环（循环必须带最大次数）。

### 状态机

```text
WorkflowRun: PENDING → RUNNING → (SUCCEEDED | FAILED | CANCELLED | WAITING_APPROVAL)
NodeRun:     PENDING → READY → RUNNING → (SUCCEEDED | FAILED | SKIPPED | WAITING_APPROVAL)
                              └→ RETRYING → RUNNING
```

每次节点状态变更写 `node_run`（含 `attempt`、`input_digest`、`output_digest`、`error`），支持「从任意失败节点重跑」。

### 检查点与恢复

```text
node 完成 → 持久化 output（大对象存 S3，PG 存引用）
          → 写 checkpoint(workflow_run_id, node_id, seq)
进程重启 → 扫描 RUNNING 且 heartbeat 超时的 run
          → 从最后一个 SUCCEEDED 节点的下游继续
          → 已经 SUCCEEDED 的节点不重跑（用 output_digest 校验）
```

副作用节点（执行测试、发布）必须幂等，否则重跑会产生重复执行。幂等键 = `hash(workflow_run_id, node_id, attempt_group)`。

### 可视化 ↔ YAML

| 方案 | 问题 |
|---|---|
| 双向同步（各自为源） | 必然丢失信息（布局、注释、顺序），且冲突无法合并 |
| **YAML 为语义源，画布为投影**（采用） | 画布编辑产生 YAML patch；布局存 `x-ui-layout` 扩展块 |

具体做法：

- 画布读 YAML → 生成图（节点坐标来自 `x-ui-layout`，缺失则自动布局）。
- 画布改动 → 生成结构化 patch（add_node / update_edge / remove_node），服务端应用到 YAML AST（保留注释与顺序）。
- 保存前做 round-trip 测试：`parse(serialize(parse(yaml))) == parse(yaml)`。

不做「所见即所得地编辑任意 YAML」——表达式、嵌套结构在画布上无法忠实表达，强行做会得到一个半残的编辑器。

### 为什么自研（D-04）

首期只有 6~8 条流程，节点类型固定，且需要与 Capability、审批、审计深度耦合。引入 Temporal 要多运维一套集群，且其编程模型（deterministic workflow code）与「配置化流程」不同。保留 `WorkflowRuntime` trait，未来可替换。

---

## 4.9 MCP Integration

### 双向能力

```text
MCP Client：连接外部 Server（Git / Issue / 测试管理 / 外部 Graph）
MCP Server：把平台能力暴露给外部 Agent（Claude / Copilot / 自研 Agent）
```

两者共用 `Capability Registry`，但方向相反：

```text
外部工具 → [MCP Client] → 归一化 → Capability（作为 Provider 的一种）
Capability → [MCP Server Adapter] → 工具描述 → 外部 Agent
```

### 对外工具设计（控制在 12 个以内）

```text
platform-mcp
├── knowledge_search          检索文档，返回带 locator 的证据
├── knowledge_get_evidence    取证据原文与上下文
├── graph_query               实体/邻居查询
├── graph_trace_path          两实体间路径
├── code_get_symbol           符号定义与引用
├── impact_analyze            执行影响分析（异步，返回 job_id）
├── impact_get_report         取报告
├── test_plan_generate        派生测试计划（草稿）
├── test_case_generate        派生用例（草稿）
├── test_run_execute          执行测试（需审批）
├── coverage_get              取覆盖快照
└── project_list              列出可访问项目
```

原则：**按业务能力组织，不按数据库表/内部函数组织**。不暴露 `sql_query`、`graph_cypher` 这类万能工具——它们是权限漏洞。

### 工具描述

```json
{
  "name": "knowledge_search",
  "version": "1.2.0",
  "capability": "knowledge.search",
  "description": "プロジェクトの設計書・仕様書を検索し、根拠位置付きで返す",
  "input_schema": { "$ref": "schema://knowledge.search.input@1" },
  "output_schema": { "$ref": "schema://knowledge.search.output@1" },
  "permission": "knowledge:read",
  "timeout_ms": 30000,
  "idempotent": true,
  "side_effects": "none"
}
```

`side_effects` 是给外部 Agent 的提示：`none / read / write / destructive`。写操作必须显式标注，且外部调用需要额外授权。

### 兼容性

外部 MCP Server 的工具参数随时可能变。处理方式：

```text
注册时：拉取 tools/list → 与本地 mapping 比对 → 记录 schema_hash
变更时：schema_hash 变化 → 标记 server 为 DEGRADED → 通知管理员 → 不自动适配
调用时：先按本地 Schema 校验输入 → 转换 → 调用 → 再按本地 Schema 校验输出
```

**不做自动 Schema 适配**。自动适配会在字段语义变化时静默出错，比报错危险得多。

### 不可信内容

MCP 返回的内容与文档内容同级：`untrusted=true`。它不能直接进入 system prompt，不能作为工具参数被二次执行，必须经过：

1. 大小限制（防 DoS）
2. 结构校验
3. 内容清洗（剥离可能的指令片段？——不做正则过滤，改为隔离与标注）
4. 权限复核（外部 Server 可能返回越权数据）

---

## 4.10 Provider Manager

### Trait 设计

```rust
#[async_trait]
pub trait Provider: Send + Sync {
    fn id(&self) -> &ProviderId;
    fn kind(&self) -> ProviderKind;
    fn capabilities(&self) -> CapabilitySet;
    async fn health(&self) -> HealthStatus;
}

#[async_trait]
pub trait GraphProvider: Provider {
    async fn upsert_entity(&self, ctx: &ProviderCtx, e: &Entity) -> Result<(), ProviderError>;
    async fn upsert_relationship(&self, ctx: &ProviderCtx, r: &Relationship) -> Result<(), ProviderError>;
    async fn neighbors(&self, ctx: &ProviderCtx, q: NeighborQuery) -> Result<Vec<Entity>, ProviderError>;
    async fn shortest_path(&self, ctx: &ProviderCtx, q: PathQuery) -> Result<Option<Path>, ProviderError>;
    async fn subgraph(&self, ctx: &ProviderCtx, q: SubgraphQuery) -> Result<Subgraph, ProviderError>;
}
```

设计取舍：

| 问题 | 选择 | 理由 |
|---|---|---|
| `async fn` in trait 与 dyn | 用 `#[async_trait]`（boxed future） | 需要 `Arc<dyn GraphProvider>` 动态注册。装箱开销在 Provider 边界（毫秒级 IO）可忽略 |
| 错误类型 | 统一 `ProviderError`，带 `kind/retryable/upstream_code` | 上层不需要知道是 reqwest 还是 neo4j 的错误 |
| 能力声明 | 每个 Provider 返回 `CapabilitySet` | 不支持的能力显式报错，不静默降级 |
| 上下文 | `ProviderCtx { project_id, revision, trace_id, deadline, cancel }` | 统一超时/取消/追踪；避免每个方法都传一堆参数 |

### 注册与配置

```text
provider(id, kind, impl_name, config_json, secret_ref, priority, enabled)
provider_health(provider_id, checked_at, status, latency_ms, message)
capability_binding(capability, provider_id, priority, fallback_order)
```

- 配置里的密钥一律 `secret_ref`，实际值存在加密的 `secret` 表（envelope encryption，主密钥来自环境变量或 KMS）。
- 同一 capability 可绑定多个 Provider，按 priority 形成 fallback 链。
- **fallback 只对「可重试错误」生效**（超时、503）。业务错误（参数非法、权限不足）不 fallback，否则会用错误的 Provider 重试出一堆脏数据。

### 能力矩阵（运行时可见）

| Provider | semantic | hybrid | rerank | evidence | graph_assist |
|---|---|---|---|---|---|
| NativeRag | Yes | Yes | Yes | Yes | Yes |
| ExternalRagA | Yes | Partial | No | Yes | Unknown |
| RagMcpB | Yes | Unknown | Unknown | Partial | No |

`Unknown` 是合法状态，表示「未探测」。UI 上显示为灰色问号，不显示为「支持」。探测方式：注册时执行一组探测查询（每个能力一条最小样例），结果写入 `capability_probe`。

### 运行时切换

```text
旧实例 Arc<dyn Provider> 仍被在途请求持有
新配置生成新实例，替换 registry 中的指针
新请求走新实例
旧实例在所有 Arc drop 后自然释放
```

不做「热更新配置字段」——Provider 内部可能缓存连接/模型，部分更新会产生不一致状态。整体替换更安全。

### Contract Test

每个 Provider 实现必须通过同一套契约测试（`tests/contract/`）：

```text
□ 正常输入 → 输出符合 Schema
□ 空结果 → 返回空数组，不报错
□ 超大输入 → 返回 INVALID_REQUEST，不 OOM
□ 超时 → 返回 TIMEOUT，且可重试标记为 true
□ 取消 → 在 100ms 内返回 CANCELLED
□ 权限过滤 → 不返回越权数据
□ 不支持的能力 → CAPABILITY_UNSUPPORTED
□ 并发 50 请求 → 无死锁、无连接泄漏
```

换 Provider 时，这套测试是唯一门槛。跑不过就不许上线。
