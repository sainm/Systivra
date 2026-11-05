# 第八部分：实施计划

## 1. 总体策略

### 1.1 三条原则

1. **每个 Phase 都必须有一条能跑通的端到端路径**。不允许「这一期只做后端，界面下期再说」——那样验收时无法判断是否真的可用。
2. **先打通，再加深**。Phase 1 就用一个 PDF 跑通「上传 → 切块 → 检索 → 证据」，哪怕切块很粗糙。后续 Phase 是替换与增强，不是从零开始。
3. **不确定性前置**。最容易翻车的四件事（XLSX 解析、OpenRewrite 离线类型归属、JSP 解析、Shift_JIS 编码）放在 Phase 0 / 2 / 4 的前段做技术验证，失败就调整方案，而不是等到联调。

### 1.2 团队配置（2026-11-05 已确认）

| 角色 | 人数 | 说明 |
|---|---|---|
| Rust 后端 | 2 | 其中 1 人兼架构 |
| 前端 | 1 | Phase 1 起介入 |
| 测试/QA | 0.5 | Phase 5 起增加到 1 |
| 设计 | 0.5 | 前 8 周集中 |
| Java（OpenRewrite Worker） | 0.5 | Phase 4 集中投入 |

人力已确认，以下工期按此配置生效（不再是「假设」）。人数变化时按比例调整，但**依赖顺序不变**。

---

## 2. Phase 划分

### Phase 0：Architecture Foundation（2 周）

| 交付物 | 内容 |
|---|---|
| 架构文档 | 本文件集评审通过 |
| ADR | ADR-001 ~ ADR-012（见 09） |
| 契约骨架 | `contracts/`：Fact / Evidence / Capability / Provider 的 JSON Schema 初版 |
| 数据模型 | 迁移脚本 001（core + kb 基础表） |
| 工程骨架 | Cargo workspace、前端脚手架、Docker Compose、CI |
| 技术验证 | XLSX 合并单元格 + 图形锚点提取（1 天）；Tree-sitter 解析 JSP 可行性（1 天）；OpenRewrite 离线 classpath（1 天）；Shift_JIS/Windows-31J 探测与字节偏移映射（1 天）；**200 册规模实测（1 天）** |

**验收**：`docker compose up` 后，API 返回 `/healthz`，前端能打开登录页；五个技术验证各有一页结论（可行/不可行/降级方案）。

---

### Phase 1：Platform Foundation + Walking Skeleton（4 周）

| 模块 | 范围 |
|---|---|
| 认证 | 本地账号 + 会话 Cookie；PAT |
| 项目 | 工作区/项目 CRUD；成员与角色 |
| 文档 | 上传（预签名）、登记、列表、软删 |
| 解析 | **仅 PDF**，走 docling-serve；产出 DocNode 树（章节 + 段落） |
| RAG（最小） | 固定大小切块（800 token）、单模型 embedding、向量检索 |
| 证据 | 证据抽屉，PDF 页码定位 + 高亮 |
| 作业 | job 表、领取、重试、SSE 进度 |
| 审计 | 写操作全部落审计 |
| 前端 | 登录、项目列表、文档列表、文档详情、检索页 |

**验收**

- 上传一份 100 页 PDF，5 分钟内完成解析与索引。
- 任意检索结果可点击跳转到 PDF 对应页并高亮。
- 杀掉 worker 进程后重启，未完成作业能续跑（从检查点）。
- 审计日志能查到「谁在什么时候上传了哪个文件」。

> 这一阶段刻意不做 XLSX、不做图、不做代码分析。目的是**先证明骨架是活的**。

---

### Phase 2：RAG Engine 完整化（5 周）

| 模块 | 范围 |
|---|---|
| XLSX 解析 | 自研：sheet / 单元格 / 合并 / 公式 / 图形锚点 / 命名区域；区域识别与切块 |
| DOCX / PPTX | docling-serve 接入，章节与幻灯片结构 |
| 结构树 | 统一 DocNode 模型，locator 规范化（PDF/DOCX/XLSX/MD/CODE） |
| 切块 | 结构感知，按格式差异化；父子 chunk（表块 → 行） |
| 检索 | 混合检索 + RRF + 可选 rerank；元数据/项目/revision 过滤 |
| 日语 | Lindera(IPADIC) 分词 + tsvector；全角半角/旧字体归一；pg_bigm 可用时启用 L2 并为部分一致做查询路由 |
| 索引版本 | `index_build` + 模型切换重建 |
| 大规模导入 | 清单（CSV/JSON）批量导入；部分成功、失败清单、断点续跑 |
| 分区 | `kb.chunk` / `kb.embedding` 按 `project_id` 分 32~64 区，每区独立 HNSW |
| 评测 | 60 题金标集（词级 / 部分一致 / 标识符外字 各 20），分三类统计 recall@10 / MRR / P95 |
| 前端 | Chunk Explorer、检索检查页（分数拆解）、索引管理 |

**验收**

- 50 sheet 的 Excel 设计书：随机抽 30 个单元格区域，检索能在 top-3 命中对应 chunk（≥ 80%）。
- 任一 chunk 可反查到 `parse_run` 与解析器版本。
- 切换 embedding 模型时，旧索引仍可查（按 `index_build` 隔离），新索引构建完成后再切换。
- 金标集三类查询各自 recall@10 ≥ 0.85（不取平均掩盖短板）；每次改动 CI 自动跑并出报告。
- 1,000 册批量导入在部分失败下能跑完并可续跑；1,200 万 chunk 规模检索 P95 < 2.0s（`ja/reference/scale-model.md` §8）。

---

### Phase 3：Graph + Ontology（4 周）

| 模块 | 范围 |
|---|---|
| Ontology | 版本化注册表；默认 v1（7 域、约 60 实体类型、20 关系类型） |
| 实体/关系 | PG 权威 + 发件箱 + Neo4j 投影；幂等、顺序保证 |
| 查询 | 实体搜索、邻居、路径、子图、追踪矩阵 |
| 版本 | 实体/关系的 revision 与 valid_to |
| 漂移检测 | 每日抽样比对 PG 与 Neo4j |
| 前端 | Ontology 编辑器（只读 + 发布）、图浏览器、追踪矩阵 |
| lite 模式 | PgGraphProvider（递归 CTE，3 跳内） |

**验收**

- 用 Phase 1/2 的文档数据构建 Document/Section/Chunk 图，节点数与 PG 一致（误差 0）。
- 任意实体的邻居查询 P95 < 200ms（10 万节点规模）。
- 手动停掉投影器 10 分钟后恢复，数据最终一致，且无重复边。
- 删除文档后，图上相关节点在 1 分钟内消失。

---

### Phase 4：Code Analysis（5 周）

| 模块 | 范围 |
|---|---|
| 仓库同步 | Git（本地/远程）、目录、快照；revision 与 tree digest |
| 扫描 | 语言识别、ignore 规则、增量（hash + diff）；**编码探测与转码（UTF-8 / Shift_JIS / Windows-31J），保留原始字节偏移** |
| Tree-sitter | Java / JS / TS / HTML / CSS / JSON / SQL（尽力）；符号与调用 |
| XML | quick-xml：web.xml、struts-config.xml、dicon、Spring bean、MyBatis mapper |
| JSP | 混合解析：占位符等长替换 → HTML 解析 → 回填 scriptlet/EL/tag |
| OpenRewrite Worker | Java 21 + LST；类型归属；NDJSON 输出；离线 classpath |
| Framework Adapter | Struts1 / SAStruts / Seasar2(S2JDBC) / Spring MVC / MyBatis / 自定义 Tag |
| SQL | JSqlParser：表/列引用、语句类型 |
| Fact | 统一模型、五态 resolution、冲突分组、增量 supersede |
| 前端 | 代码分析概览、依赖图、未解析清单 |

**验收**

- SAMPLE 工程（见 §5）：类/方法识别率 ≥ 98%；方法级调用关系 precision ≥ 98%（对 RESOLVED 边）。
- 反射/动态代理案例 30 个，全部标为 `INFERRED` 或 `UNRESOLVED`，无一条被误标为 `RESOLVED`。
- 增量分析：改 1 个文件，重分析时间 < 全量的 10%。
- 无 classpath 场景：降级为语法级，`classpath_complete=false` 出现在报告里。

---

### Phase 5：Impact Analysis（3 周）

| 模块 | 范围 |
|---|---|
| 变更集 | Git diff / 手工 / 文档修订 / DDL diff |
| 种子解析 | 文件+行 → 符号；字段变更 → 读写方法闭包 |
| 遍历 | 反向依赖、数据流、共享资源闭包、hub 截断 |
| 证据 | 文档 RAG + 代码 Fact + 图路径合并 |
| 风险 | 确定性打分 + 因子拆解 |
| 输出 | ImpactReport（JSON/Markdown）、影响图 |
| 前端 | A-04 三栏工作台、A-05 详情、图形化路径 |

**验收**

- 对 SAMPLE 工程的 20 个预设变更，影响项召回 ≥ 90%、精确率 ≥ 70%（对比人工金标；过度包含必须标注理由）。
- 任一影响项可展开完整路径与证据；无证据的项数量为 0。
- 同一输入重复执行，结果完全一致（diff 为空）。
- 截断情况（hub/depth/budget）100% 在 UI 上可见。

---

### Phase 6：Test Engineering（5 周）

| 模块 | 范围 |
|---|---|
| 规则 | 从 Java/JSP/JS/设计书抽取 `ValidationRule`；冲突标 CONFLICT |
| 场景 | 按业务流 + 规则 + 异常分支派生 |
| 用例 | 结构化步骤，绑定 covers 与 evidence |
| 数据 | 等价类/边界/日语特例（全角、外字、サロゲート） |
| 执行 | Playwright Runner + JSON DSL；截图/trace/log 采集 |
| 覆盖 | 四维覆盖计算（需求/规则/画面/影响） |
| 缺陷 | 失败 → 缺陷，关联影响项 |
| 前端 | 计划、场景、用例、数据、执行看板、覆盖矩阵 |

**验收**

- SAMPLE 工程的 300 条规则，自动生成 ≥ 900 组数据，人工抽检有效率 ≥ 80%。
- 派生用例首次执行通过率 ≥ 60%（说明「能跑」）。
- 覆盖快照可复算：给定 test_run，任意人重算得到同一数字。
- 「生成但未执行」的用例在覆盖里计为 0，界面上有明确说明。

---

### Phase 7：AI Agent / MCP / Workflow（4 周）

| 模块 | 范围 |
|---|---|
| AI Provider | `OpenAiCompatProvider`（DeepSeek/vLLM/Ollama）+ `CopilotRuntimeProvider`（可选） |
| Agent | 注册、版本、Prompt、能力白名单、预算、Draft |
| Capability Gateway | Schema 校验、权限交集、审计、超时、取消 |
| 审批 | 审批请求、决策、留痕 |
| MCP Client | 注册、发现、调用、健康、schema_hash 变更检测 |
| MCP Server | 12 个工具，对外暴露 |
| Workflow | YAML 定义、状态机、检查点、审批节点、React Flow 编辑器 |
| 前端 | Agent 配置、运行详情、MCP 管理、Workflow 设计器 |

**验收**

- 关闭所有 AI Provider，平台核心功能（Phase 1~6）不受影响，UI 上相关入口显示「AI 未設定」。
- Agent 无法访问未授权项目：构造越权 Prompt，被网关拒绝并记审计。
- 换一个 MCP Server（同名工具、不同实现），业务 Agent 定义不需修改。
- Workflow 在审批节点暂停 24 小时后仍能恢复执行。

---

### Phase 8：Enterprise Readiness（持续）

多租户、SSO（SAML/OIDC）、细粒度 RBAC、备份恢复演练、性能优化、安全加固、HA 部署、运维手册。

不设固定结束时间，按客户需求排优先级。

---

## 3. 工程目录

```text
systivra/
├── Cargo.toml                  # workspace
├── crates/
│   ├── domain/                 # 领域类型、纯逻辑、无 IO
│   ├── contracts/              # DTO、Capability/Provider/Fact/Evidence Schema
│   ├── store/                  # sqlx、迁移、发件箱、作业队列
│   ├── providers/              # Provider trait 实现与管理
│   ├── engine-rag/
│   ├── engine-graph/           # 含 ontology
│   ├── engine-code/
│   ├── engine-impact/
│   ├── engine-test/
│   ├── runtime/                # capability gateway、agent、workflow、mcp
│   ├── app/                    # 用例编排（api 与 worker 共用）
│   ├── api/                    # axum 路由与中间件
│   └── worker/                 # 作业执行进程
├── apps/
│   └── web/                    # React + Vite
├── workers/
│   ├── rewrite/                # Java 21 + OpenRewrite（Gradle）
│   └── runner/                 # Node + Playwright
│   # 用 docling-serve 镜像，不自研 doc-worker
├── contracts/
│   ├── api/openapi.json        # 由 utoipa 生成，CI 校验
│   ├── capabilities/*.schema.json
│   ├── providers/*.schema.json
│   ├── facts/fact.schema.json
│   ├── evidence/evidence.schema.json
│   ├── ontology/v1/*.yaml
│   └── mcp/tools/*.schema.json
├── migrations/                 # sqlx migrations（按 schema 编号）
├── deployment/
│   ├── docker-compose/
│   │   ├── docker-compose.yml
│   │   ├── docker-compose.lite.yml
│   │   └── .env.example
│   └── offline/                # 离线镜像包脚本
├── docs/
│   ├── design/                 # 本文件集
│   ├── architecture/
│   ├── database/
│   ├── ontology/
│   ├── api/
│   ├── adr/
│   ├── operation/
│   └── ui/
├── samples/
│   └── sample-java-web/        # SAMPLE 工程（明确标记）
└── tests/
    ├── integration/
    ├── contract/
    ├── e2e/
    └── golden/                 # 金标集与评测脚本
```

### 3.1 为什么是 13 个 crate，而不是 20 个

Prompt 给出的初始结构有 16 个 crate。合并理由：

| 原建议 | 处理 | 理由 |
|---|---|---|
| `ontology` | 并入 `engine-graph` | 本体是图的类型系统，两者接口高度耦合；分开会导致循环依赖 |
| `audit` | 并入 `store` | 审计就是一张只追加的表 + 查询，独立 crate 只会增加一层转发 |
| `provider-core` + `provider-manager` | 合并为 `providers` | 分开后 trait 与注册表互相引用，边界不清晰 |
| `capability-contracts` | 并入 `contracts` | 契约应集中，便于生成与版本管理 |
| `application` | 独立为 `app` | api 与 worker 共用，必须独立 |

判据：**当某个 crate 的编译时间超过 90 秒，或它的公开 API 稳定到可以单独发版时，才拆**。否则拆了只是增加 `pub use` 的工作量。

---

## 4. 依赖顺序与开发顺序

### 4.1 编译依赖（决定先后）

```text
domain ──▶ contracts ──▶ store ──▶ providers ──▶ engines ──▶ runtime ──▶ app ──▶ api
                                                                    └──▶ worker
```

`engine-impact` 依赖 `engine-graph` + `engine-rag`；`engine-test` 依赖 `engine-code` + `engine-impact`。这是 Phase 顺序的硬约束。

### 4.2 开发顺序（每个 Phase 内部）

```text
1. 契约（Schema / trait / migration）
2. 存储层 + 单元测试
3. 引擎核心算法 + 单元测试（用合成数据，不依赖真实项目）
4. Capability 包装 + 集成测试（testcontainers）
5. API + OpenAPI
6. 前端（先用 mock，接口就绪后切换）
7. E2E + 金标集
8. 文档与 ADR 更新
```

第 3 步「用合成数据」很关键：引擎不应该等到真实数据才能开发。例：影响引擎用 200 个节点的合成图开发遍历与评分，Phase 4 结束后再换成真实图。

### 4.3 并行机会

| 可并行 | 说明 |
|---|---|
| Phase 2 前端 与 Phase 3 后端 | 追踪矩阵前端可在图 API 就绪前用 mock |
| Phase 4 Java Worker 与 Rust 扫描器 | 以 NDJSON 契约解耦，两边可同时开发 |
| Phase 6 测试 DSL 与 Phase 5 影响引擎 | Runner 只需要 case schema |

---

## 5. SAMPLE 工程

### 5.1 规格

| 项 | 数量 | 说明 |
|---|---|---|
| 画面 | 24 | 一覧 8 / 登録編集 8 / 参照 4 / バッチ 4 |
| 业务模块 | 6 | 用户、订单、库存、请求、审批、通知 |
| Java 类 | ~180 | Action 24、Service 30、Dao 40、Form 24、Validator 15、Util 20、其他 |
| JSP | 60 | 含 3 个自定义 tag |
| DB 表 | 42 | 含主外键、视图 3、存储过程 2 |
| 校验规则 | 300+ | 单项目 180、跨项目 60、业务规则 60 |
| 配置 | 5 类 | web.xml、struts-config、dicon、Spring bean、MyBatis mapper |
| 设计书 | 6 份 | 基本设计 PDF、详细设计 XLSX（12 sheet）、画面设计 XLSX、API 规格、DB 定义、测试规格 |
| 版本 | 3 个 revision | 用于影响分析与 diff 演示 |

### 5.2 技术特征（必须覆盖的「难」点）

- Struts1 风格 Action + `struts-config.xml` 映射
- SAStruts 注解风格（与 Struts1 混合，模拟迁移残留 → 制造 `CONFLICT`）
- Seasar2 Dicon 注入 + S2JDBC
- JSP 自定义 Tag（`<my:input>`）+ taglib
- 反射调用（1 处 `Class.forName`）
- 字符串拼装 SQL（2 处，制造 `UNRESOLVED`）
- 动态代理（Spring AOP 风格）
- 跨字段校验（开始日 < 结束日）
- 多表事务（订单 + 库存 + 请求）
- 外部 API 调用（HTTP 客户端）
- 日语特有输入（全角、外字、サロゲート）
- **编码混在**：Java 用 UTF-8，JSP 用 Shift_JIS，CSV 用 Windows-31J（模拟真实遗留系统的历史遗留问题）

### 5.3 明确标记

- 数据库 `project.is_sample = true`
- UI 顶部常驻横幅「サンプルプロジェクト」
- 所有文档页脚水印 `SAMPLE`
- 导出的报告带 `SAMPLE` 标记

**不得**在任何演示、宣传、交付物中把示例数据表述为真实客户数据。

---

## 6. 测试策略

### 6.1 分层

| 层 | 工具 | 覆盖对象 | 目标 |
|---|---|---|---|
| 单元 | `cargo nextest` + `proptest` | domain 规则、遍历算法、打分、切块 | 行覆盖 ≥ 75%，核心算法 ≥ 90% |
| 集成 | `testcontainers`（PG + Neo4j）+ wiremock | store、provider、capability | 关键路径全覆盖 |
| 契约 | 自建契约套件 | 每个 Provider 实现 | 8 项必过（见 04.10） |
| E2E | Playwright（平台自身 UI） | 登录→上传→检索→影响→测试 | 主流程 1 条，冒烟 10 条 |
| 金标 | 自定义 runner | RAG / 代码分析 / 影响 | 每次改动出指标报告 |
| 性能 | `criterion` + `k6` | 遍历、检索、投影 | 见 6.3 |
| 安全 | `cargo audit` / Trivy / ZAP | 依赖与 API | 无 high/critical |

### 6.2 金标集（Golden Set）

没有金标集的「智能」功能等于没有质量。三套必须建：

| 金标集 | 规模 | 标注内容 | 用途 |
|---|---|---|---|
| RAG | 60 问（词级 / 部分一致 / 标识符外字 各 20） | 正确答案的 chunk_id 集合 | recall@10、MRR、证据正确率 |
| 代码分析 | 200 关系 + 30 动态案例 | 真实关系、预期 resolution | precision / recall |
| 影响分析 | 20 变更 | 人工列出的影响项（含理由） | 召回率、精确率、截断率 |

金标集随代码提交，CI 每次跑，指标下降超过阈值则阻断合并。

### 6.3 性能目标（Phase 2/4 用真实数据校准）

> 下表是**验收值（可对客户承诺的上限）**。设计正确时能达到的目标值见 `ja/detail/03-performance.md` §1（检索 1.2s、影响 90s、首次导入 24h）。

| 场景 | 验收值（P95） | 条件 |
|---|---|---|
| 检索 P95 | < 2.0s | 1,200 万 chunk，混合检索 + rerank |
| 邻居查询 P95 | < 300ms | 84 万节点，单跳 |
| 影响分析（6 跳） | < 120s | 250 万 fact，37 个影响项 |
| 文件解析吞吐 | ≥ 20 页/秒 | PDF 文本层，docling |
| Excel 解析 | ≥ 5000 单元格/秒/核 | 合并单元格 + 公式 |
| 首次导入 | < 48 小时 | 60,000 册，16 个 worker |
| 日增量 | < 40 分钟 | 100 册 + 50 个 Java 文件 |
| 增量代码分析 | < 全量 10% | 改 1 个文件 |

### 6.4 CI 门禁

```text
每个 PR：
  cargo fmt --check
  cargo clippy -- -D warnings
  cargo nextest run            （单元 + 集成）
  contract tests               （受影响 Provider）
  openapi diff                 （breaking 需人工批准）
  pnpm lint && pnpm test
  license check
  cargo audit / pnpm audit
  docker build（仅 main）

每晚：
  golden set（RAG / code / impact）
  性能基准（criterion 对比 main）
  漂移检测（图 vs PG）
  E2E 全流程
```

---

## 7. 每阶段验收总表

| Phase | 一句话验收 | 硬指标 |
|---|---|---|
| 0 | 骨架能跑，三个技术风险有结论 | compose 起得来；3 份验证报告 |
| 1 | 一个 PDF 能从上传走到证据 | 100 页 / 5 分钟 / 可定位 |
| 2 | Excel 设计书能被正确切块与检索 | 三类查询 recall@10 ≥ 0.85 |
| 3 | 文档图能建起来且可重建 | 节点误差 0；重建成功 |
| 4 | 真实 Java Web 能解析出可信关系 | precision ≥ 0.98（RESOLVED） |
| 5 | 改一个方法能算出影响范围 | 召回 ≥ 0.90；可复现 |
| 6 | 影响项能变成可执行测试并算覆盖 | 首执通过率 ≥ 0.60；覆盖可复算 |
| 7 | AI 能用平台能力，且越权被挡住 | 关闭 AI 不影响核心；越权 0 成功 |
| 8 | 能交给客户运维 | 备份恢复演练通过；SSO 可用 |

---

## 8. 里程碑风险

| 里程碑 | 最大风险 | 触发信号 | 预案 |
|---|---|---|---|
| Phase 2 末 | Excel 解析质量不达标 | 抽检 30 个区域，命中 < 60% | 降级为「sheet 级 chunk」并明确告知；Phase 5 前补齐 |
| Phase 4 中 | OpenRewrite 离线不可用 | classpath 构建失败率 > 30% | 全面降级为语法级；影响分析标注 `PARTIAL`；把「类型归属」列为客户环境前置条件 |
| Phase 4 末 | JSP 解析错位 | 证据行号偏差 > 2 行 | 回退到「文件级」定位，牺牲精度保正确 |
| Phase 5 | 影响项爆炸（几百条） | 单次 > 200 项 | 收紧 hub 阈值 + 按业务域预过滤 + 分级展示 |
| Phase 6 | Playwright 在选择器上不稳定 | 首执通过率 < 40% | 引入 `data-testid` 约定；对客户系统提出埋点要求 |
| Phase 7 | Copilot SDK 在企业网络不可用 | CLI 启动失败 | 只用 OpenAiCompatProvider；Copilot 作为可选项保留 |
