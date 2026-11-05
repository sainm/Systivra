# Systivra — Software Engineering Intelligence Platform

## 总体设计文档集 · v0.1（Phase 0 交付物）

| 项目 | 内容 |
|---|---|
| 文档集版本 | 0.1 |
| 编制日期 | 2026-11-05 |
| 对应输入 | 《项目总体设计与开发 Prompt》第 22 节 |
| 状态 | 待评审。评审通过后进入 Phase 1，编码前不再修改本文档集中的架构决策 |
| 技术基线 | Rust（stable, edition 2024）/ PostgreSQL 18 / Neo4j / React + TypeScript |
| 首个验证对象 | 日企 Java Web 遗留系统（Struts1 / SAStruts / Seasar2 风格） |

> **正式版为日语版**：`ja/` 目录（12 个文件，约 4,500 行）。本目录的中文版保留作内部参考，后续更新以日语版为准。

---

## 1. 文档地图

原始 Prompt 要求「严格按照以下顺序输出」九个部分，本文件集与之一一对应。

| 顺序 | 原始要求 | 文档 | 核心产出 |
|---|---|---|---|
| 第一部分 | 产品定位 | `01-product.md` | 定位、使用者、场景、业务闭环、与三类工具的差异 |
| 第二部分 | 总体架构 | `02-architecture.md` | 上下文图、组件图、模块边界、数据流、运行时、部署 |
| 第三部分 | 技术选型 | `03-tech-stack.md` | 技术栈、职责、理由、替代、风险、版本核查结果 |
| 第四部分 | 核心架构设计 | `04-core-engines.md` | RAG / Graph / Ontology / Code / Impact / Test / Agent / Workflow / MCP / Provider |
| 第五部分 | 统一数据模型 | `05-data-model.md` | PG 模型、Neo4j Ontology、RAG 模型、Fact / Evidence / Test / Provider / MCP Schema |
| 第六部分 | API 设计 | `06-api.md` | 约定、端点清单、请求响应示例、错误模型、权限 |
| 第七部分 | UI 设计 | `07-ui.md` | 画面清单、层级、跳转、线框图、可视化、日语 UI 原则 |
| 第八部分 | 实施计划 | `08-roadmap.md` | Phase 划分、目录结构、依赖顺序、测试策略、验收标准 |
| 第九部分 | 风险与决策 | `09-risks-adr.md` | 风险登记册、ADR、待决事项 |
| 附录 A | 开源项目评估 | `reference/open-source-evaluation.md` | 分类、许可证、活跃度、采用结论 |
| 附录 B | 版本核查记录 | `03-tech-stack.md` §2 | 核查日期、来源、对设计的影响 |

阅读建议：评审会前读 `01` + `02` + `09`；开发启动前读 `04` + `05` + `08`；前端启动前读 `06` + `07`。

---

## 2. 本次核查改变设计的六个事实

原始 Prompt 中有若干处属于「实施前必须核实」的判断。以下是本次（2026-11-05）核查后**需要修正 Prompt 原始假设**的部分。这不是补充说明，而是设计输入。

| # | 事实（已核实） | 对设计的强制影响 |
|---|---|---|
| F-01 | GitHub Copilot SDK 已于 2026-06-02 GA，提供 Node/Python/Go/.NET/**Rust**/Java 六种 SDK；BYOK 支持任意 OpenAI 兼容端点（含自定义 `baseUrl`），因此 DeepSeek 可经 BYOK 接入 | 但 Copilot SDK 的运行时是 **Copilot CLI sidecar 进程**（JSON-RPC）。私有化内网、无 GitHub 账号的部署场景不应强制依赖它。**结论：AI Provider 必须双实现**——`OpenAiCompatProvider`（默认，直连 DeepSeek/vLLM/Ollama）与 `CopilotRuntimeProvider`（可选，用其 agent loop）。业务代码只依赖 `AiProvider` trait |
| F-02 | Neo4j 自 2025 年起改用**日历版本号**（2026.10.1 为当前版本），**2026.x 不是 LTS**：日历版本只 hotfix 到下一月度版本发布为止（约 1 个月）。唯一 LTS 是 5.26（2024-12-06 发布，hotfix 至 2028-06-06）。Community 版许可为 **GPLv3**（已核实发布包 `LICENSE.txt`），Enterprise 为商业许可 | Neo4j **不得内嵌进我们的二进制或以库形式链接**，只能作为独立服务由客户部署或随部署包以独立进程分发。这反向强化了「PostgreSQL 为权威源、Neo4j 仅为可重建投影」的决策（ADR-002）；版本基线锁定 5.26（ADR-013） |
| F-03 | OpenRewrite 当前为 8.90.x（Apache-2.0），但**部分 recipe 制品已迁至 Code Genome Project 仓库并需要认证**，migrate 系列 recipe 采用 Moderne Source Available License | 我们**只使用 OpenRewrite 的 LST 解析与类型归属能力**（`rewrite-java` 核心，Apache-2.0），不使用其 recipe 分发体系，不把 OpenRewrite 当作产品依赖链的一环。Worker 镜像自行构建并锁定版本 |
| F-04 | Tree-sitter 当前 0.27.1；官方 grammar 组织**不包含 XML**（有 HTML、JSON、Java、JS、TS、CSS、SQL 等，无 XML） | XML / Dicon / web.xml 解析**不能**指望 Tree-sitter，改用 Rust `quick-xml` + 自建语义层；JSP 无现成 grammar，必须自建「HTML grammar + scriptlet 区域重解析」的混合解析器。这是 Code Analysis 工作量的主要低估点（见 R-07） |
| F-05 | pgvector 当前 0.8.6（2026-07-29）。0.8.3 修复了 HNSW vacuum 导致的**索引损坏**，0.8.4 修复相关插入错误 | 生产环境 HNSW 的**最低版本为 0.8.4**，目标 0.8.6。同时启用 0.8 引入的 iterative index scan，否则「项目过滤 + 向量检索」会因过度过滤而召回崩塌 |
| F-06 | Docling 2.142.1（MIT）、Playwright 1.64.1（Apache-2.0，Node ≥ 20）、PostgreSQL 18.6 | Excel 设计书**不走 Docling**：Docling 的 XLSX 支持给出 cell provenance，但会丢失合并单元格语义、图形对象与公式/缓存值区分。XLSX 用 Rust `calamine` + 直接解析 `xl/drawings`、`xl/worksheets` 自建解析器；Docling 只负责 PDF / DOCX / PPTX |

> 核查方法：官方站点、GitHub Release、crates.io API、npm registry、Maven Central 索引。核查日期 2026-11-05。前端依赖版本未纳入本次核查，在 Phase 0 结束时以 lockfile 固定（见 D-07）。

---

## 3. 关键假设（Assumptions）

设计按以下假设成立。任一假设被推翻时，需要回到对应章节复核。

| ID | 假设 | 影响章节 | 推翻后的后果 |
|---|---|---|---|
| A-01 | 首期验证对象为 Java 17 为主的 Java Web 系统，允许存在 Java 8 语法与 Struts1 风格遗留代码 | 04.4 / 08 | Java 分析器需增加旧语法降级策略 |
| A-02 | 单项目规模上限：50 万行 Java、2 万文件、5 万页文档、20 万 chunk、100 万 fact | 02.6 / 09 R-09 | 超出后需引入分片与增量投影，Postgres 单库可能不足 |
| A-03 | 默认部署在客户内网，无公网出口；AI Provider 可完全关闭且平台仍可用 | 02.7 / 04.7 | 若必须公网，需补充出网审计与数据脱敏 |
| A-04 | 单实例用户数 < 200，多租户为逻辑隔离（行级 + schema 级），非物理隔离 | 05 / 09 R-12 | 需改为库级隔离与独立密钥体系 |
| A-05 | 设计文档以日语为主，UI 默认日语，中文/英文为副 | 07 | 分词、字体、文案规范需重做 |
| A-06 | 测试执行环境由客户提供，平台只负责编排与结果采集，不负责环境供给 | 04.6 | 需增加环境管理模块 |
| A-07 | 平台不自动修改源代码，只输出建议与 diff 预览，人工确认后由客户侧提交 | 04.7 / 09 | 需增加写权限、回滚与代码托管集成 |
| A-08 | 版本锚点统一为 Git/SVN revision；无版本控制时以 snapshot（tree digest）兜底 | 04.4 / 05 | 增量分析与复现能力退化 |
| A-09 | 客户接受「Graph 为派生数据、可重建」这一前提，不要求把 Neo4j 当唯一事实源 | 02.4 / ADR-002 | 需改为双写并承担一致性成本 |
| A-10 | 首期不做跨项目知识复用；项目间数据严格隔离 | 02.3 | 需引入共享本体与跨项目实体对齐 |

---

### 3.1 A-05 展开：日文语料的六个特殊处理

「文档是日文的」不只是一个 i18n 问题。它影响解析、存储、检索、代码分析、测试数据、报告排版六个层面，任何一层按英文假设实现都会在真实项目上返工。

| # | 问题 | 设计应对 | 落点 |
|---|---|---|---|
| J-01 | **编码**：遗留系统常见 Shift_JIS / Windows-31J，JSP、Java、SQL、CSV 都可能是。按 UTF-8 读会乱码，且行号/字节偏移全错 | 入库前做编码探测（`chardetng`）+ 转码（`encoding_rs`）；同时保存「原始字节哈希 + 原始编码 + 解码后文本」；代码证据定位以**原始字节偏移**为准 | 04.4 / 05 |
| J-02 | **分词**：无词间空格，`to_tsvector` 直接失效 | Lindera(IPADIC) 预分词；索引与查询共用同一分词器与词典版本，版本写入索引元数据 | 04.1 / ADR-007 |
| J-03 | **归一化**：旧字体（髙/﨑）、全半角、半角カナ、送り仮名 | **双轨制**：匹配用 NFKC + 旧字体映射；存储与证据用原文（NFC），`quote` 校验比对原始字节，绝不用归一化后的文本做证据 | 04.1 / 05 |
| J-04 | **PDF 版式**：縦書き、ルビ（振り仮名）、スキャン件 | Docling 版面分析 + 日文 OCR；縦書き在 `locator` 标记 `writing_mode: vertical`，前端按列高亮；ルビ作为独立文本层存储，不混入正文 | 04.1 |
| J-05 | **排版**：禁則処理、和暦、全角括弧 | PDF 报告用日文字体嵌入 + 禁则处理；日期同时支持西历与和历（令和），存储仍用 ISO 8601 | 07 |
| J-06 | **测试数据**：全角数字、全角空格、外字、サロゲートペア、結合文字、和历 | 测试数据生成器内置日语边界集；`length` 判定区分「文字数 / バイト数 / コードポイント数」 | 04.6 |

> J-01 是最容易被低估、后果最严重的一条：编码错了，后面的行号、证据、diff 全部失效，而且表面上「能打开文件」，不会立刻报错。

---

## 4. 决策记录（2026-11-05 已确认）

以下七项在 2026-11-05 的评审中确认，**直接影响 Phase 1 的开工范围**。原始 Prompt 要求「不要反复询问已经明确的信息」，因此这里只记录结论与理由，不再列为待决。

| ID | 决策点 | 结论 | 理由 | 状态 |
|---|---|---|---|---|
| D-01 | Neo4j 版本线 | **5.26 LTS（5.26.32）**，支持至 2028-06-06；2026.x 仅作为可选兼容目标，不作为基线 | **Neo4j 没有「2026.x LTS」**。2026.x 是日历版本，只 hotfix 到下一月度版本发布为止（约 1 个月）；唯一的 LTS 是 5.26，也是 5.x 的最后一个版本。私有化产品不能把「每月必须升级」当作前提 | 已确认 |
| D-02 | 日语全文检索 | **分层：L1 Lindera + tsvector 恒开；L2 pg_bigm 扩展可用时启用；L3 PGroonga 仅复用客户既有**；配查询路由与 RRF 融合 | 「以词为单位」与「以字为单位」是两种范式，各自覆盖一半查询。只选一种，另一类一定难用。详见 `reference/japanese-search.md` | 已确认 |
| D-03 | Graph 权威源 | **PostgreSQL 为 SoR，Neo4j 为可重建投影**（ADR-002） | Provider 可替换的前提是「换掉 Neo4j 不丢数据」；同时避免双写不一致 | 已接受 |
| D-04 | Workflow 引擎 | **自研**（Rust 状态机 + YAML 定义），保留 `WorkflowRuntime` port | 首期流程节点少，且需与 Capability/审批深度耦合；引入 Temporal 会多一整套运维面 | 已确认 |
| D-05 | 图形库分工 | **Cytoscape.js 管关系图，React Flow 管流程图**，不混用 | 数据模型与交互范式不同，混用两边都做不好 | 已确认 |
| D-06 | 演示工程 | **自建 SAMPLE 工程**，24 画面，`is_sample=true` + UI 水印 + 报告标记 | 客户资料周期不可控，会阻塞 Phase 4；示例必须可识别，不得冒充真实数据 | 已确认 |
| D-07 | 前端依赖版本 | Phase 0 结束时锁定并写入 `pnpm-lock.yaml` | 前端生态发版快，文档写死版本会在两周内过期 | 已确认 |

同时确认的两项前提：

| 项 | 结论 |
|---|---|
| A-07 首期不自动改代码 | **确认**。平台只输出建议与 diff 预览，人工确认后由客户侧提交。Phase 7 不实现代码写入权限 |
| 团队投入 | **2 Rust + 1 前端 + 0.5 设计**（Phase 1~5）；Phase 6 起 QA 增至 1 人，Phase 4 增加 0.5 Java。`08-roadmap.md` 的工期估算按此生效 |

---

## 5. 交付边界（本次不做什么）

本文件集是 Phase 0 的**设计交付物**，不是代码。以下内容明确不在本次范围：

- 不生成 Cargo workspace 与业务代码（Phase 1 起逐阶段交付）。
- 不输出 29 份完整设计书的全文（Prompt 第 20 节），本次交付的是其中 9 个部分的**可开发版本**；剩余文档（Security Design、Monitoring Design、Deployment Design 详版等）在对应 Phase 启动前补齐，列入 `08-roadmap.md` 的交付物清单。
- 不进行 UI 视觉稿设计（高保真），只给线框与交互规则；视觉在 Phase 1 与前端框架一起落地。
- 不对客户真实系统做承诺性评估；所有性能指标均为目标值，需在 Phase 2/4 用真实数据校准。
