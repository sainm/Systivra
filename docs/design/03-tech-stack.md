# 第三部分：技术选型

## 1. 技术栈总览

```text
Frontend    React + TypeScript + Vite + Tailwind + shadcn/ui
            TanStack Query / Zustand / React Router
            Cytoscape.js (关系图) + React Flow (流程图) + Monaco (只读/diff)
            i18next + ICU MessageFormat (ja 默认 / zh-CN / en)

API         Rust · axum · tower · tokio · utoipa (OpenAPI 3.1)
            SSE 流式 · problem+json 错误模型

Core        Rust · sqlx (PostgreSQL) · serde · thiserror · async-trait
            petgraph (内存图算法) · rayon (CPU 密集) · tokio-util (取消)
            encoding_rs + chardetng (Shift_JIS/Windows-31J 探测与转码)

Document    XLSX: calamine + 自研 OOXML 补充解析 (Rust)
            PDF/DOCX/PPTX: docling-serve (Python, 独立服务)
            HTML/MD/CSV/TXT: 原生 Rust

Code        tree-sitter 0.27.x + 各语言 grammar (Rust)
            XML/Dicon/web.xml: quick-xml (Rust)
            JSP: 自建混合解析器 (HTML grammar + scriptlet 重解析)
            Java 语义: OpenRewrite LST Worker (Java 21, 独立服务)
            SQL 语义: JSqlParser (Java Worker 内)

Graph       Neo4j 5.26 LTS (投影库) · neo4j-rust-driver
            lite 模式: PgGraphProvider (递归 CTE, 能力受限)

RAG         PostgreSQL 18 + pgvector 0.8.6
            全文: Lindera 预分词 + tsvector(simple)；可选 pg_bigm
            融合: RRF (k=60) + RerankProvider

Test        Playwright 1.64.1 (Node ≥ 20, 独立 Runner) · JSON 步骤 DSL
            可选: JaCoCo (后端覆盖率, Phase 6+)

AI          AiProvider trait
            ├ OpenAiCompatProvider  (DeepSeek / vLLM / Ollama / Azure OpenAI 兼容端点)
            └ CopilotRuntimeProvider (github-copilot-sdk, 可选)

Deploy      Docker Compose · MinIO (S3 兼容) · OpenTelemetry Collector
```

---

## 2. 版本核查结果（核查日期 2026-11-05）

> 原则：不写「latest」，写「核查到的具体版本 + 日期」。三个月后这份表就会过期，因此每条都附带「过期后如何复核」。

| 组件 | 核查版本 | 发布日期 | 来源 | 对本项目的影响 | 复核方式 |
|---|---|---|---|---|---|
| PostgreSQL | 18.6 | 2026-08-13 | postgresql.org/docs/release | 基线大版本 18；18.5 因回归未发布，直接到 18.6 | `postgresql.org/docs/release/` |
| pgvector | 0.8.6 | 2026-07-29 | GitHub Releases / 官方 README | **生产 HNSW 最低 0.8.4**（0.8.3 修复索引损坏）；0.8 起必须启用 iterative scan 解决过滤后召回崩塌 | `github.com/pgvector/pgvector/releases` |
| Neo4j | **5.26.32（LTS）**；2026.10.1 为当前日历版本 | 2026-10-08 / 2026-10-20 | neo4j.com/developer/kb/neo4j-supported-versions | **不存在「2026.x LTS」**：日历版本只 hotfix 到下一月度版本发布（约 1 个月）；5.26 是唯一 LTS，hotfix 至 2028-06-06。首期锁定 5.26（D-01）；**不得内嵌分发**（GPLv3） | `neo4j.com/developer/kb/neo4j-supported-versions` |
| Lindera | 6.0.0（工作区在向 7.0 迁移） | 2026-08-25 | GitHub Releases / lib.rs | 日语分词；**词典不随 crate 分发**，需在部署包中单独提供 `lindera-ipadic`；本体 MIT | `lib.rs/crates/lindera` |
| axum | 0.8.9 | 2026-04-14 | crates.io API | API 层主框架；0.8 系列已稳定，注意 0.9 可能带来 breaking change | `crates.io/api/v1/crates/axum` |
| sqlx | 0.9.0 | 2026-05-21 | crates.io API | 0.9 是新大版本，与 0.8 有 API 差异；编译期检查需 `.sqlx` 离线缓存 | `crates.io/api/v1/crates/sqlx` |
| Tree-sitter | 0.27.1 | 2026-10-08 | Wikipedia / GitHub Releases | grammar ABI 与 0.23~0.25 不完全兼容，**必须锁 grammar 版本并做 ABI 兼容测试** | `github.com/tree-sitter/tree-sitter/releases` |
| tree-sitter-java | 0.23.5 | 2026-06 | Ubuntu / crates.io | 语法层解析；语义交给 OpenRewrite | `crates.io/api/v1/crates/tree-sitter-java` |
| OpenRewrite | 8.90.x | 2026-08 | Maven Central 索引 | 仅用 `rewrite-java` 核心（Apache-2.0）做 LST 与类型归属；**不用其 recipe 分发**（F-03） | `mvnrepository.com/artifact/org.openrewrite` |
| Docling | 2.142.1 | 2026-10-23 | 官方文档 / GitHub Releases | PDF/DOCX/PPTX 解析；XLSX 不用它（F-06） | `github.com/docling-project/docling/releases` |
| docling-serve | 1.38.1 | 2026-10-23 | 官方 README | 直接以容器方式提供解析服务，**不自己写 Python Worker**（见 §4.3） | 同上 |
| Playwright | 1.64.1 | 2026-11（npm latest） | registry.npmjs.org | Apache-2.0；Node ≥ 20；离线环境需预置浏览器二进制 | `registry.npmjs.org/playwright/latest` |
| GitHub Copilot SDK | GA（1.x） | 2026-06-02 | github.blog changelog | 提供 Rust SDK；BYOK 支持 OpenAI 兼容端点（含自定义 baseUrl）；但运行时是 CLI sidecar（F-01） | `github.blog/changelog` |
| pg_bigm / PGroonga | 未核查具体版本 | — | PGXN / 官方文档 | 均为 PostgreSQL License。**pg_bigm 为 L2 可选（有扩展则启用）；PGroonga 为 L3，仅复用客户既有，不作默认**（ADR-007） | `pgxn.org` / `pgroonga.github.io` |

**明确未核查、不做承诺的组件**：React、Vite、Tailwind、shadcn/ui、TanStack Query、Zustand、Cytoscape.js、React Flow、Monaco、Lindera 及其词典。理由见 D-07：前端与词典类依赖写死版本号会在两周内过期。这些在 Phase 0 结束时由 `pnpm-lock.yaml` / `Cargo.lock` / 词典版本清单固定，并记录进 `docs/operation/dependency-baseline.md`。

---

## 3. 逐层选型理由

### 3.1 为什么核心必须是 Rust

| 理由 | 具体表现 |
|---|---|
| 确定性计算密集 | 影响遍历、Fact 归一化、增量 diff 都是 CPU/IO 密集且需要可预测延迟。Rust 无 GC 停顿，P99 可控 |
| 单二进制分发 | 私有化部署最怕「装一堆运行时」。Rust 产出静态二进制，容器镜像可做到 < 100 MB |
| 资源占用 | 同规模下内存占用约为 JVM 的 1/3~1/5，客户内网虚拟机规格通常不高 |
| 类型系统承担契约 | Capability 的输入输出、Provider 的能力声明，用类型表达比用文档表达可靠 |
| 与 Worker 的边界清晰 | Rust 做「调度 + 归一化 + 计算」，Python/Java/Node 只做「格式转换」，职责不会渗透 |

**代价（必须承认）**：开发速度慢于 Python；生态在文档解析、NLP 上不如 Python；招聘成本高。因此把「格式解析」「NLP 分词」外包给成熟组件或独立 Worker，而不是硬用 Rust 重写。

### 3.2 数据存储

| 选择 | 理由 | 拒绝的替代 |
|---|---|---|
| PostgreSQL 作 SoR | 事务、JSONB、全文、向量、行级安全都在一个引擎里；备份/恢复/PITR 成熟；客户 IT 普遍接受 | 不用 MongoDB（事务弱）、不用 MySQL（pgvector/JSONB 弱） |
| pgvector 而非专用向量库 | 向量与元数据同库，权限过滤、revision 过滤、JOIN 都能下推到 SQL；省一个中间件 | 不用 Qdrant/Milvus：多一套服务、多一次网络往返、跨库一致性難 |
| Neo4j 作投影 | Cypher 表达路径查询最自然；生态与可视化工具成熟 | 不用 NebulaGraph（运维复杂）、不用 JanusGraph（性能与活跃度） |
| MinIO | S3 兼容，客户可换成任意对象存储；本地开发零依赖 | 不用本地文件系统作权威（无法多实例、无版本） |

**日语语料对 Embedding / Rerank 的额外要求**：

- Embedding 必须按**日语**评测选型（如 bge-m3 一类多语言模型），不能用英文 benchmark 代替。日文检索的语义召回全靠它，模型选错，后面的词法路由与重排都救不回来。
- Rerank 同理，必须支持日语；只支持英文的 reranker 在日文上会把正确结果排到后面，比不重排更糟。
- 评测集必须来自真实日文设计书，不能用人翻译过的英文样本——敬体/常体、片假名外来语、省略主语的句式，机器翻译样本覆盖不到。
- 向量维度与模型版本写入 `embedding_model` 注册表；换模型必须重建索引（见 05 §1.3）。

**pgvector 的关键限制（必须写进设计）**：

1. `vector(N)` 的维度在列类型上固定，**不同维度的模型不能共表**。方案：按维度建表（`embedding_768` / `embedding_1024` / `embedding_1536`），同表内不同模型用 **partial HNSW index** 隔离（`WHERE model_id = $1`），查询必须带 `model_id` 才能命中索引。
2. HNSW 索引构建内存需求高，`maintenance_work_mem` 不足会退化。大项目重建索引要单独排期。
3. 过滤 + ANN 的召回问题：必须 `SET LOCAL hnsw.iterative_scan = relaxed_order`，否则「只要这个项目的 chunk」会把候选集过滤到几乎为空。

**大规模下的额外约束（A-02：embedding chunk 1,200 万）**：

- 1,200 万 × 1024 维 × 4 B = 49 GB；HNSW 索引 75~100 GB。单索引构建时内存不够。
- 按 `project_id` 分 **32~64 个分区**，每个分区独立 HNSW。查询固定 `project_id`，分区裁剪有效。
- 保留 `halfvec` 把向量减半、再用 rerank 补精度的选项。
- 该规模下搜索 P95 目标为 2.0 秒（`ja/reference/scale-model.md` §8）。

### 3.3 代码分析

| 层次 | 技术 | 为什么不用别的 |
|---|---|---|
| 语法层 | tree-sitter | 增量解析、错误容忍、多语言统一 API。ANTLR 需要为每种语言写语法且错误恢复差 |
| 语义层（Java） | OpenRewrite LST | 有类型归属、方法解析、跨文件符号表。**JavaParser 没有类型归属**，Spoon 需要可编译的完整 classpath 且内存开销大 |
| 配置层 | quick-xml | Tree-sitter 官方**没有 XML grammar**（F-04）；quick-xml 流式、低内存、可保留位置信息 |
| SQL | JSqlParser（Java Worker 内） | Tree-sitter 的 SQL grammar 为第三方、方言覆盖不稳；JSqlParser 对 Oracle/PostgreSQL 方言支持更实用 |
| JSP | 自建混合解析 | 没有可用 grammar。策略：按 HTML 解析，遇到 `<% %>` / `${}` / `<jsp:include>` 切出区域单独处理 |

**OpenRewrite 的使用边界（重要）**：

```text
允许：解析 Java 源码为 LST、类型归属、方法调用解析、继承/实现关系、注解提取
禁止：执行 recipe、依赖 Code Genome Project 仓库、把 OpenRewrite 的运行时嵌入产品
```

类型归属需要 classpath。离线客户的依赖 jar 必须预先收集（`mvn dependency:build-classpath` 或 Gradle 的 `dependencies` 任务），存为 `classpath_index`。**没有 classpath 时，分析降级为语法级，所有跨文件关系标记 `PARTIAL`**，不允许假装解析成功。

### 3.4 文档解析

| 格式 | 方案 | 理由 |
|---|---|---|
| XLSX | 自研（calamine + OOXML） | 日本设计书的 Excel 不是「表格」，是「用单元格排版的画布」。必须保留合并单元格、行高列宽、图形锚点、公式与缓存值、定义名称、批注 |
| PDF | docling-serve | 版面分析、阅读顺序、表格结构、OCR。自研不现实 |
| DOCX | docling-serve | 章节结构 + 表格；也可用 `docx-rs` 但结构提取不如 Docling |
| PPTX | docling-serve | 图文混排，需保留 slide 序号与形状位置 |
| HTML | 原生 Rust（scraper） | 结构清晰，无需外部服务 |
| MD / TXT / CSV | 原生 | — |

**编码前提（日语项目的硬约束）**：

客户语料不能假设 UTF-8。遗留 Java Web 的 JSP / Java / SQL / CSV 常见 Shift_JIS 与 Windows-31J，且项目内混用。所有文本入口（文档、源码、配置、测试数据）统一走 `chardetng` 探测 + `encoding_rs` 转码，并保留原始字节偏移用于证据定位。探测置信度低于 0.75 时标记 `ENCODING_UNDETECTED`，不猜测。详见 04.4。

**XLSX 解析的具体要求**（这是最容易被低估的部分）：

```text
必须提取：
  workbook.sheets[]            → Sheet 名、索引、可见性、顺序
  sheet.dimension              → 使用范围
  cells[r][c]                  → 值、类型、公式、样式 ID、批注
  merged_ranges[]              → 合并区域（值只在左上角，其余为空，必须展开）
  drawings[]                   → 图形/图片/图表，锚点 cell + 偏移
  defined_names[]              → 命名区域（设计书常用「印刷範囲」）
  hyperlinks[]                 → 内外链接
  row_heights / col_widths     → 用于判断「视觉分组」（设计书靠布局表达层级）

不允许：
  把 sheet 拍平成一段文本
  丢掉合并单元格的从属关系
  把公式的计算结果当作唯一值（公式本身也是信息）
```

### 3.5 AI Provider

| 实现 | 用途 | 关键约束 |
|---|---|---|
| `OpenAiCompatProvider` | **默认**。DeepSeek、vLLM、Ollama、Azure OpenAI 兼容端点 | 只用 `/chat/completions` + tools；结构化输出用 JSON Schema + 重试校验，不假设厂商支持 `response_format` |
| `CopilotRuntimeProvider` | 可选。需要其 agent loop（文件编辑、MCP、hook）时启用 | 依赖 Copilot CLI 进程；需要 GitHub 订阅或 BYOK；**不得反向代理 Copilot 内部接口** |

两个实现共享同一套 `AiRequest` / `AiResponse` / `ToolCall` 抽象。业务代码只认 `AiProvider`，不认模型名。

能力声明（Provider 必须自报，不允许假设）：

```rust
pub struct AiCapabilities {
    pub streaming: bool,
    pub tool_calling: bool,
    pub parallel_tool_calls: bool,
    pub json_schema_output: bool,
    pub vision: bool,
    pub max_context_tokens: u32,
    pub max_output_tokens: u32,
}
```

缺失能力时的行为必须是**显式降级**：`json_schema_output=false` → 用「提示词约束 + 解析失败重试 + 失败则报错」，而不是「假装成功」。

### 3.6 前端

| 选择 | 职责 | 理由 |
|---|---|---|
| React + TS + Vite | 应用框架 | 生态、组件库、招聘面；Vite 冷启动快 |
| Tailwind + shadcn/ui | 设计系统 | shadcn 是「复制到项目里」的组件，可深度改造；不像 Ant Design 那样把你锁进它的视觉语言 |
| TanStack Query | 服务端状态 | 缓存、失效、乐观更新、轮询作业进度 |
| Zustand | 客户端状态 | 图上的选中/过滤/布局等局部状态，比 Redux 轻 |
| Cytoscape.js | 关系图 | 大图性能、布局算法（dagre/cose）、子图导出。**不用 D3 手搓**：成本高且交互要重写 |
| React Flow | 流程图 | 节点是 React 组件，表单/预览可直接嵌进节点。**不用它画关系图**：几万个节点会卡 |
| Monaco | 代码/差异 | VS Code 同源，diff 视图开箱可用 |

---

## 4. 被拒绝的方案（记录理由，避免反复讨论）

| 方案 | 拒绝理由 | 什么情况下重新考虑 |
|---|---|---|
| 核心用 Python / Java | Prompt 明确要求 Rust 掌握业务与执行控制；且 Python 在高并发编排与长事务上需要额外工程 | 不重新考虑 |
| 一开始就微服务 | 团队规模与运维能力不匹配；分布式事务、链路追踪、部署复杂度会吃掉全部进度 | 单进程无法支撑 > 50 并发分析作业时 |
| Kafka / NATS 作队列 | 首期用 PG job 表足够（SKIP LOCKED 的吞吐可达数千/秒）；多一个中间件就多一套运维 | 作业吞吐 > 500/s 或需要跨机房时（Phase 8） |
| Elasticsearch 作全文 | 日语分词仍需插件；多一个集群；与 PG 的权限/事务割裂 | 需要复杂相关度调优或日志检索时 |
| LangChain / LlamaIndex 作 RAG 核心 | 它们是「胶水」，不是引擎；切块策略、证据契约、权限过滤都要自己控制 | 不重新考虑（可参考其 chunker 思路） |
| Dify 作 Workflow 引擎 | 它是完整应用，不是可嵌入引擎；引入即锁定其数据模型与 UI | 若放弃自研 Workflow（D-04 被否决） |
| Temporal | 能力强但运维重（Cassandra/PG + 多服务）；首期流程简单 | 流程数 > 50 且需要跨天持久化与复杂补偿时 |
| Neo4j 作 SoR | 见 02 §4.2 | 不重新考虑 |
| Tree-sitter 解 XML | 官方无 XML grammar（F-04） | 出现成熟且维护良好的 grammar 时 |
| 自己写 Python Docling Worker | `docling-serve` 已提供稳定 HTTP API，重复造轮子 | 需要定制 DoclingDocument → 我们的 DocNode 映射且无法在适配层完成时 |

---

## 5. 兼容性与风险清单

| # | 风险 | 具体表现 | 缓解 |
|---|---|---|---|
| T-01 | sqlx 0.9 与 0.8 的 API 差异 | 宏、类型映射、连接池配置变更 | Phase 1 第一周锁定版本并写 ADR；不使用 nightly 特性；升级前跑全量集成测试 |
| T-02 | Tree-sitter grammar ABI 不匹配 | 0.27 runtime 加载 0.23 grammar 时 panic 或静默失败 | 锁定 runtime + 每个 grammar 的精确版本；CI 增加「所有 grammar 能 parse 样本文件」的冒烟测试 |
| T-03 | OpenRewrite 离线类型归属失败 | 无网络时依赖解析失败 → 退化为语法级 | 显式降级 + `PARTIAL` 标记；客户侧预生成 classpath；解析结果里记录 `classpath_complete: bool` |
| T-04 | Docling 性能 | 比 PyMuPDF 慢 5~30 倍；大 PDF 可能几分钟 | 解析预算 + 超时 + 分页并行；对「纯文本 PDF」走快速路径（pdf-extract）先判断是否需要版面分析 |
| T-05 | Neo4j 许可 | Community 版为 GPLv3（已核实发布包 LICENSE.txt），Enterprise 为商业许可。GPL 下若链接/修改并分发，会波及我方代码 | 以独立服务分发，不链接、不修改、不内嵌；法务在 Phase 1 复核 |
| T-06 | pgvector 维度与模型切换 | 换 embedding 模型后旧向量不可混用 | `embedding_model` 注册表 + 维度分表 + 索引版本；切换需重建索引（后台作业） |
| T-07 | 日语词典许可 | IPADIC / UniDic 的商用条款不同 | Phase 2 前确认所选词典许可；优先 IPADIC 系（BSD 类），UniDic 需法务确认 |
| T-08 | Playwright 离线 | 浏览器二进制需下载 | 预置镜像 + `PLAYWRIGHT_BROWSERS_PATH`；镜像体积换可靠性 |
| T-09 | 前端依赖快速演进 | 半年后文档里的版本全过期 | 不写死版本；lockfile + 每季度升级窗口 |
| T-10 | Rust 生态在 Office 解析上的缺口 | 图形/图表/复杂样式提取需手写 OOXML | 限定首期只支持「单元格 + 合并 + 图形锚点 + 公式」，图表内容标 `UNSUPPORTED` 并提示 |

---

## 6. 依赖治理

| 规则 | 执行方式 |
|---|---|
| 所有依赖锁定 | `Cargo.lock` / `pnpm-lock.yaml` / `requirements.txt`（docling 镜像固定 tag）/ Maven `dependencyManagement` |
| 禁止 `*` 版本 | CI 检查 |
| 许可证白名单 | Apache-2.0 / MIT / BSD / PostgreSQL License / EPL-2.0 允许；GPL/AGPL 需法务评估，且只允许「独立服务」形态 |
| 安全扫描 | `cargo audit` + `pnpm audit` + Trivy 镜像扫描，CI 阻断 high/critical |
| 升级节奏 | 每季度一个升级窗口；安全补丁随时；大版本升级必须写 ADR |
