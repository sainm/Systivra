# 附录 A：开源项目评估

> 评估日期：2026-11-05。
> 标记说明：**[核]** = 本次通过官方站点 / GitHub Release / 包管理器元数据核实；**[待核]** = 基于一般认知，采用前必须在官方仓库的 LICENSE 与 Releases 页二次确认。
> Prompt 明确要求「不能凭旧经验给出结论」，因此本表把「已核」与「待核」分开，不用模糊表述掩盖不确定性。

---

## 1. 分类口径

| 分类 | 含义 | 采用方式 |
|---|---|---|
| A 架构参考 | 借鉴其分层、模块划分、扩展点设计 | 读代码与文档，不引入依赖 |
| B 数据模型参考 | 借鉴其 Schema、实体关系、版本模型 | 转译到我们的模型，不引入依赖 |
| C 直接集成 | 作为库/二进制直接使用 | 锁版本，纳入许可证白名单 |
| D 独立 Worker | 作为独立服务/进程运行，通过协议通信 | 容器化，不链接 |
| E 仅借鉴思路 | 读文章/论文，不读代码，不引入 | 无 |
| F 不建议 | 许可证/活跃度/架构不匹配 | 明确排除，记录理由 |

---

## 2. 评估结果

### 2.1 RAG / Graph

| 项目 | 语言 | 许可证 | 活跃度 | 分类 | 结论与理由 | 核查 |
|---|---|---|---|---|---|---|
| **neo4j-graphrag-python** | Python | Apache-2.0 | 高 | E | 借鉴其「检索器组合 + 图检索 + 重排」的管线抽象；它是 Python 库，与 Rust 核心不兼容。其 GraphRAG 的实体抽取流程可参考，但我们的实体来自代码分析而非 LLM 抽取，不能照搬 | [待核] |
| **Graphiti** | Python | Apache-2.0 | 高 | B | 最有价值的是**双时间轴模型**（valid_at / invalid_at + created_at），与我们 Fact 的 `valid_from/valid_to` 高度一致，可直接借鉴其边界情况处理（同一事实多次出现、矛盾事实并存）。不引入：Python + Neo4j 绑定过深 | [待核] |
| **Flexible GraphRAG** | Python | 需核实 | 中低 | F | 多后端适配的尝试，但维护节奏与文档不足以支撑企业依赖；且它解决的问题（多向量库切换）我们已用 Provider 抽象解决 | [待核] |
| **RAGFlow** | Python + TS | Apache-2.0 | 很高 | A | 借鉴其**深度文档解析 + 可视化切块**的产品形态（尤其是「原文与 chunk 对照」的交互）。不引入：它是完整应用，接入即变成二次封装，违背自研核心能力原则 | [待核] |
| **CocoIndex** | Rust + Python | Apache-2.0 | 高 | A | 借鉴其**声明式增量索引**的数据流模型（source → transform → target，变更只重算受影响部分）。我们的 `parse_run → chunk → embed` 增量设计参考了该思路 | [待核] |
| **OpenSPG / KAG** | Java + Python | Apache-2.0 | 高 | B | 借鉴其**schema 约束下的知识构建**与逻辑形式推理（把自然语言问题转成结构化查询再执行）。对我们的启示：Schema 必须先于实例存在，否则图会退化成一堆无类型的边 | [待核] |

### 2.2 代码分析

| 项目 | 语言 | 许可证 | 活跃度 | 分类 | 结论与理由 | 核查 |
|---|---|---|---|---|---|---|
| **Tree-sitter** | C / Rust | MIT | 很高 | **C** | 直接集成。语法层主力。注意：官方 grammar 集合**不含 XML**，JSP 无 grammar（见 F-04） | [核] 0.27.1 |
| **OpenRewrite** | Java | Apache-2.0（核心）；部分 recipe 为 Moderne Source Available | 很高 | **D** | 独立 Worker，只用 LST 与类型归属。**不使用 recipe 分发体系**（需 Code Genome Project 认证），避免许可证与供应链风险 | [核] 8.90.x |
| **jQAssistant** | Java | GPL-3.0 | 中低 | F | 概念（把代码结构导入 Neo4j 再用 Cypher 查询）与我们一致，但 GPL-3.0 不适合分发场景，且活跃度不足。仅读文档 | [待核] |
| **Spoon** | Java | 需核实（Apache-2.0 或 GPL 系） | 中 | E | 元编程能力强，但需要可编译的完整 classpath，内存开销大。OpenRewrite 已覆盖需求 | [待核] |
| **JavaParser** | Java | Apache-2.0 | 高 | E | 轻量语法解析，**无类型归属**。可作为 OpenRewrite 不可用时的降级方案（只到语法级），但不作为主路径 | [待核] |

### 2.3 追踪 / 测试

| 项目 | 语言 | 许可证 | 活跃度 | 分类 | 结论与理由 | 核查 |
|---|---|---|---|---|---|---|
| **Eclipse Capra** | Java | EPL-2.0 | 低（近似归档） | E | 追踪矩阵的**数据模型**（Trace Link 作为一等公民、带类型与置信度）值得借鉴。工具本身不建议依赖：Eclipse RCP 技术栈、更新停滞 | [待核] |
| **StrictDoc** | Python | Apache-2.0 | 高 | B | 借鉴其**需求-规格-测试的双向追踪**与「文档即数据」的模型（REQ/SPEC 节点 + 关系 + 稳定 UID）。我们的 `Requirement ↔ TestCase` 追踪设计参考了它 | [待核] |
| **Kiwi TCMS** | Python | GPL-2.0（需核实） | 高 | F | 只借鉴其「用例版本化 + 执行记录不可变」的思路。不引入：GPL + Django 单体，且它的核心是「登记」而非「派生」，与我们的定位相反 | [待核] |

### 2.4 Workflow / UI / 基础设施

| 项目 | 语言 | 许可证 | 活跃度 | 分类 | 结论与理由 | 核查 |
|---|---|---|---|---|---|---|
| **Dify** | TS + Python | Apache-2.0 + 附加条款 | 很高 | A | 借鉴其 **DSL 设计**（节点类型、变量引用语法 `${node.output}`、版本化）与画布交互。不引入：完整应用，接入即锁定其数据模型与 UI | [待核] |
| **React Flow** | TypeScript | MIT | 高 | **C** | 直接集成，Workflow 编辑器 | [待核] |
| **Cytoscape.js** | JavaScript | MIT | 高 | **C** | 直接集成，关系图 | [待核] |
| **Docling** | Python | MIT | 很高 | **D** | 通过 `docling-serve` 容器集成，负责 PDF/DOCX/PPTX。**不用于 XLSX**（ADR-011） | [核] 2.142.1 / serve 1.38.1 |
| **pgvector** | C | PostgreSQL License | 高 | **C** | 直接集成。HNSW 生产下限 0.8.4 | [核] 0.8.6 |
| **pg_bigm** | C | PostgreSQL License | 中 | C（可选） | 日语部分一致检索的可选增强，取决于客户环境是否允许安装扩展 | [待核] |
| **PGroonga** | C | PostgreSQL License | 中 | F（首期） | 性能好但索引大、非 crash-safe、运维负担高。首期不引入，保留为「客户已有时复用」 | [待核] |

### 2.5 AI SDK

| 项目 | 语言 | 许可证 | 活跃度 | 分类 | 结论与理由 | 核查 |
|---|---|---|---|---|---|---|
| **GitHub Copilot SDK** | TS / Python / Go / .NET / Rust / Java | 需核实（SDK 许可与 CLI 许可不同） | 很高（2026-06 GA） | C（可选） | 提供 Rust SDK，BYOK 支持 OpenAI 兼容端点。但运行时是 CLI sidecar，内网/离线场景不适用。仅作可选 Provider | [核] GA 2026-06-02 |
| **DeepSeek API** | — | 商业服务 | — | C（默认） | 通过 OpenAI 兼容接口调用，不依赖 SDK。需评估数据出境与合规（A-03） | [待核] 商务条款 |

---

## 3. 采用结论汇总

### 直接集成（C）

```text
tree-sitter 0.27.x + 各 grammar（锁版本）
pgvector 0.8.6
Cytoscape.js
React Flow
（可选）pg_bigm
（可选）GitHub Copilot SDK
```

### 独立 Worker（D）

```text
docling-serve 1.38.1      PDF / DOCX / PPTX 解析
OpenRewrite 8.90.x        Java LST 与类型归属
Playwright 1.64.1         测试执行
```

### 参考借鉴（A / B / E）

```text
CocoIndex        增量索引数据流
Graphiti         双时间轴事实模型
StrictDoc        需求-测试追踪模型
RAGFlow          文档与 chunk 对照的交互形态
Dify             Workflow DSL 与画布交互
OpenSPG/KAG      Schema 先行的知识构建
Capra            Trace Link 作为一等公民
jQAssistant      代码结构入图再查询的整体思路
```

### 明确不采用（F）

```text
RAGFlow          作为核心（会变成二次封装）
Dify             作为 Workflow 引擎（完整应用，不可嵌入）
jQAssistant      GPL-3.0 + 活跃度不足
Kiwi TCMS        GPL + 定位相反（登记 vs 派生）
PGroonga         首期运维负担过高
Flexible GraphRAG 维护与文档不足
```

---

## 4. 评估新项目的检查清单

任何新增依赖都必须过这张表，结论写入 ADR 或本文件。

```text
□ 许可证：是否在白名单（Apache-2.0 / MIT / BSD / PostgreSQL License / EPL-2.0）
          GPL/AGPL 是否只以独立进程方式使用（不链接、不分发衍生作品）
          Commons Clause / Source Available 是否影响商业模式
□ 活跃度：近 6 个月是否有 release；issue 响应；维护者数量；是否有企业背书
□ 版本：当前稳定版是多少；是否遵循 semver；大版本升级频率
□ 部署：是否需要额外中间件；资源占用；是否支持离线
□ 兼容：语言/运行时版本；与我们现有栈的冲突
□ 替代成本：如果两年后弃用，迁移要多久
□ 安全：是否有已知 CVE；是否执行任意代码；是否访问网络
□ 数据：是否会把数据传出部署边界
```

判据：**任何一项为「不可接受」即否决；三项以上为「不确定」则延后引入**。
