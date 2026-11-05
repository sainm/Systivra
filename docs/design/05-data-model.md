# 第五部分：统一数据模型

## 1. PostgreSQL 关系模型

### 1.1 Schema 划分

| Schema | 内容 | 特点 |
|---|---|---|
| `core` | 工作区、项目、用户、权限、审计、作业、发件箱 | 事务核心 |
| `kb` | 文档、修订、解析树、chunk、向量、证据 | 读多写少，可重建部分多 |
| `code` | 仓库、修订、文件、符号、Fact | **只追加**，历史不可变 |
| `graph_meta` | Ontology、实体、关系、投影状态 | 图权威源 |
| `test` | 规则、计划、场景、用例、数据、执行、覆盖 | 事务 + 大量读 |
| `ai` | Agent、Prompt、运行、审批、Workflow、MCP | 配置 + 运行记录 |
| `ops` | Provider、健康、密钥引用 | 运维面 |

分 schema 而不是分库：单库事务仍然是最大的工程优势；schema 提供逻辑边界，未来可平滑拆库。

### 1.2 ER 总览

```text
workspace 1─┬─* project 1─┬─* document 1─* document_revision 1─* doc_node 1─* chunk
            │             │                                        └─* chunk_embedding
            │             ├─* repository 1─* code_revision 1─* source_file 1─* symbol
            │             │                                        └─* fact ─* fact_evidence
            │             ├─* entity 1─* relationship
            │             ├─* validation_rule
            │             ├─* test_plan 1─* test_scenario 1─* test_case 1─* test_step
            │             │                                      └─* test_data_set ─* test_data_row
            │             ├─* test_run 1─* test_execution 1─* test_result
            │             ├─* impact_report 1─* impact_item
            │             ├─* agent 1─* agent_run 1─* agent_step
            │             └─* workflow_def 1─* workflow_run 1─* node_run
            └─* app_user 1─* project_member
```

### 1.3 核心 DDL（节选，非全量）

```sql
-- ============ core ============
CREATE TABLE core.workspace (
  id          uuid PRIMARY KEY,
  code        text NOT NULL UNIQUE,
  name        text NOT NULL,
  created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE core.project (
  id             uuid PRIMARY KEY,
  workspace_id   uuid NOT NULL REFERENCES core.workspace(id),
  code           text NOT NULL,
  name           text NOT NULL,
  is_sample      boolean NOT NULL DEFAULT false,   -- D-06：示例工程必须有标记
  default_locale text NOT NULL DEFAULT 'ja',
  source_kind    text NOT NULL CHECK (source_kind IN ('GIT','SVN','DIR','SNAPSHOT')),
  settings       jsonb NOT NULL DEFAULT '{}',
  created_at     timestamptz NOT NULL DEFAULT now(),
  deleted_at     timestamptz,
  UNIQUE (workspace_id, code)
);

CREATE TABLE core.project_member (
  project_id  uuid NOT NULL REFERENCES core.project(id),
  user_id     uuid NOT NULL,
  role        text NOT NULL CHECK (role IN ('OWNER','ARCHITECT','DEVELOPER','TESTER','VIEWER')),
  granted_at  timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (project_id, user_id)
);

-- 作业：所有异步执行的唯一入口
CREATE TABLE core.job (
  id               uuid PRIMARY KEY,
  project_id       uuid NOT NULL,
  kind             text NOT NULL,           -- PARSE_DOC / PROJECT_GRAPH / ANALYZE_CODE / IMPACT / TEST_RUN ...
  priority         smallint NOT NULL DEFAULT 5,
  status           text NOT NULL CHECK (status IN
                     ('PENDING','QUEUED','RUNNING','WAITING_APPROVAL',
                      'SUCCEEDED','FAILED','CANCELLED','TIMEOUT')),
  idempotency_key  text NOT NULL,
  input            jsonb NOT NULL,
  progress         jsonb NOT NULL DEFAULT '{"done":0,"total":0,"phase":null}',
  attempt          smallint NOT NULL DEFAULT 0,
  max_attempts     smallint NOT NULL DEFAULT 3,
  locked_by        text,
  heartbeat_at     timestamptz,
  error            jsonb,
  created_at       timestamptz NOT NULL DEFAULT now(),
  started_at       timestamptz,
  finished_at      timestamptz,
  UNIQUE (project_id, idempotency_key)
);
CREATE INDEX job_claim_idx ON core.job (status, priority DESC, created_at)
  WHERE status IN ('PENDING','QUEUED');

CREATE TABLE core.job_checkpoint (
  job_id      uuid NOT NULL REFERENCES core.job(id) ON DELETE CASCADE,
  seq         integer NOT NULL,
  cursor      jsonb NOT NULL,
  created_at  timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (job_id, seq)
);

-- 发件箱：与业务写入同事务
CREATE TABLE core.outbox_event (
  id            uuid PRIMARY KEY,
  aggregate     text NOT NULL,
  aggregate_id  uuid NOT NULL,
  seq           bigint NOT NULL,
  event_type    text NOT NULL,
  payload       jsonb NOT NULL,
  occurred_at   timestamptz NOT NULL DEFAULT now(),
  published_at  timestamptz,
  dead_letter   boolean NOT NULL DEFAULT false
);
CREATE INDEX outbox_pending_idx ON core.outbox_event (seq)
  WHERE published_at IS NULL AND NOT dead_letter;

CREATE TABLE core.audit_log (
  id            bigserial PRIMARY KEY,
  project_id    uuid,
  actor_type    text NOT NULL CHECK (actor_type IN ('USER','AGENT','SYSTEM')),
  actor_id      text NOT NULL,
  action        text NOT NULL,
  target_type   text,
  target_id     text,
  input_digest  bytea,
  result        text NOT NULL,
  detail        jsonb NOT NULL DEFAULT '{}',
  trace_id      text,
  occurred_at   timestamptz NOT NULL DEFAULT now()
);
-- 只追加：REVOKE UPDATE, DELETE

-- ============ kb ============
CREATE TABLE kb.document (
  id           uuid PRIMARY KEY,
  project_id   uuid NOT NULL,
  doc_type     text NOT NULL,     -- REQUIREMENT/BASIC_DESIGN/DETAILED_DESIGN/.../OTHER
  title        text NOT NULL,
  source_path  text,
  business_domain text,
  acl_groups   text[] NOT NULL DEFAULT '{}',
  current_revision_id uuid,
  created_at   timestamptz NOT NULL DEFAULT now(),
  deleted_at   timestamptz
);

CREATE TABLE kb.document_revision (
  id              uuid PRIMARY KEY,
  document_id     uuid NOT NULL REFERENCES kb.document(id),
  source_revision text,                    -- git sha / svn rev / snapshot id
  content_hash    bytea NOT NULL,
  object_key      text NOT NULL,           -- 原始文件在对象存储的位置
  file_size       bigint NOT NULL,
  mime_type       text NOT NULL,
  status          text NOT NULL CHECK (status IN ('CURRENT','SUPERSEDED','FAILED')),
  source_encoding text NOT NULL DEFAULT 'UTF-8',   -- 探测结果：UTF-8 / Shift_JIS / Windows-31J ...
  encoding_confidence real NOT NULL DEFAULT 1.0,
  parser          text,
  parser_version  text,
  acquired_at     timestamptz NOT NULL,
  parsed_at       timestamptz,
  UNIQUE (document_id, content_hash)
);
-- 同一文档只允许一个 CURRENT
CREATE UNIQUE INDEX doc_current_rev ON kb.document_revision (document_id)
  WHERE status = 'CURRENT';

CREATE TABLE kb.doc_node (
  id          uuid PRIMARY KEY,
  revision_id uuid NOT NULL REFERENCES kb.document_revision(id) ON DELETE CASCADE,
  parent_id   uuid REFERENCES kb.doc_node(id),
  node_type   text NOT NULL,
  locator     jsonb NOT NULL,
  ordinal     integer NOT NULL,
  text        text,
  meta        jsonb NOT NULL DEFAULT '{}'
);
CREATE INDEX doc_node_parent_idx ON kb.doc_node (revision_id, parent_id, ordinal);

CREATE TABLE kb.chunk (
  id            uuid PRIMARY KEY,
  project_id    uuid NOT NULL,
  document_id   uuid NOT NULL,
  revision_id   uuid NOT NULL,
  chunk_type    text NOT NULL CHECK (chunk_type IN
                  ('TEXT','TABLE','TABLE_ROWS','CODE','FIGURE','SHEET_REGION','SUMMARY')),
  ordinal       integer NOT NULL,
  text          text NOT NULL,
  tokenized     text NOT NULL,     -- Lindera 分词后以空格连接
  token_count   integer NOT NULL,
  content_hash  bytea NOT NULL,
  locator       jsonb NOT NULL,    -- 证据定位的唯一依据
  node_path     text[] NOT NULL,   -- 结构路径，用于邻域扩展
  metadata      jsonb NOT NULL DEFAULT '{}',
  fts           tsvector GENERATED ALWAYS AS (to_tsvector('simple', tokenized)) STORED,
  is_current    boolean NOT NULL DEFAULT true,
  superseded_at timestamptz,
  created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX chunk_fts_idx   ON kb.chunk USING gin (fts) WHERE is_current;
CREATE INDEX chunk_scope_idx ON kb.chunk (project_id, document_id, is_current);

CREATE TABLE kb.embedding_model (
  id          uuid PRIMARY KEY,
  name        text NOT NULL,          -- 如 bge-m3 / text-embedding-3-large
  dim         integer NOT NULL,
  distance    text NOT NULL CHECK (distance IN ('COSINE','L2','IP')),
  provider    text NOT NULL,
  revision    text NOT NULL,
  enabled     boolean NOT NULL DEFAULT true,
  UNIQUE (name, revision)
);

-- 按维度分表；同表内不同模型用 partial HNSW index 隔离
CREATE TABLE kb.embedding_1024 (
  chunk_id   uuid NOT NULL REFERENCES kb.chunk(id) ON DELETE CASCADE,
  model_id   uuid NOT NULL REFERENCES kb.embedding_model(id),
  vec        vector(1024) NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (chunk_id, model_id)
);
-- 每个模型一条，由迁移脚本生成（model_id 必须是字面量）
-- CREATE INDEX ON kb.embedding_1024 USING hnsw (vec vector_cosine_ops)
--   WHERE model_id = '<uuid>';

CREATE TABLE kb.evidence (
  id           uuid PRIMARY KEY,
  project_id   uuid NOT NULL,
  source_type  text NOT NULL,   -- PDF_PAGE / DOCX_SECTION / EXCEL_RANGE / CODE_LINE / CHUNK
  document_id  uuid,
  revision_id  uuid,
  chunk_id     uuid,
  locator      jsonb NOT NULL,
  quote        text NOT NULL,
  quote_hash   bytea NOT NULL,
  resolution   text NOT NULL,
  confidence   real NOT NULL,
  created_at   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX evidence_doc_idx ON kb.evidence (document_id, revision_id);

-- ============ code ============
CREATE TABLE code.repository (
  id          uuid PRIMARY KEY,
  project_id  uuid NOT NULL,
  kind        text NOT NULL CHECK (kind IN ('GIT','SVN','DIR','SNAPSHOT')),
  url         text,
  default_branch text,
  settings    jsonb NOT NULL DEFAULT '{}'
);

CREATE TABLE code.code_revision (
  id           uuid PRIMARY KEY,
  repository_id uuid NOT NULL REFERENCES code.repository(id),
  revision     text NOT NULL,          -- sha / svn rev / tree digest
  parent_id    uuid REFERENCES code.code_revision(id),
  tree_digest  bytea NOT NULL,
  committed_at timestamptz,
  is_baseline  boolean NOT NULL DEFAULT false,
  UNIQUE (repository_id, revision)
);

CREATE TABLE code.source_file (
  id            uuid PRIMARY KEY,
  revision_id   uuid NOT NULL REFERENCES code.code_revision(id) ON DELETE CASCADE,
  path          text NOT NULL,
  language      text NOT NULL,
  content_hash  bytea NOT NULL,
  size_bytes    bigint NOT NULL,
  encoding      text NOT NULL DEFAULT 'UTF-8',
  loc           integer NOT NULL,      -- 行数（按解码后文本计）
  line_offsets  bytea,                 -- 每行起始的原始字节偏移表，用于精确定位
  UNIQUE (revision_id, path)
);

CREATE TABLE code.symbol (
  id           uuid PRIMARY KEY,
  revision_id  uuid NOT NULL,
  file_id      uuid NOT NULL,
  kind         text NOT NULL,          -- CLASS/METHOD/FIELD/INTERFACE/ENUM
  fqn          text NOT NULL,          -- com.example.UserEditAction#update
  simple_name  text NOT NULL,
  signature    text,
  line_start   integer NOT NULL,
  line_end     integer NOT NULL,
  modifiers    text[] NOT NULL DEFAULT '{}',
  metadata     jsonb NOT NULL DEFAULT '{}'
);
CREATE INDEX symbol_fqn_idx ON code.symbol (revision_id, fqn);

CREATE TABLE code.fact (
  id                uuid PRIMARY KEY,
  project_id        uuid NOT NULL,
  revision_id       uuid NOT NULL,
  subject_kind      text NOT NULL,
  subject_key       text NOT NULL,
  relation          text NOT NULL,
  object_kind       text NOT NULL,
  object_key        text NOT NULL,
  resolution        text NOT NULL CHECK (resolution IN
                      ('RESOLVED','INFERRED','PARTIAL','UNRESOLVED','CONFLICT')),
  confidence        real NOT NULL CHECK (confidence BETWEEN 0 AND 1),
  analysis_engine   text NOT NULL,
  engine_version    text NOT NULL,
  conflict_group    uuid,
  unresolved_reason text,
  candidates        jsonb NOT NULL DEFAULT '[]',
  fact_hash         bytea NOT NULL,
  valid_from        timestamptz NOT NULL DEFAULT now(),
  valid_to          timestamptz,
  superseded_by     uuid REFERENCES code.fact(id),
  created_at        timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX fact_subject_idx ON code.fact (project_id, revision_id, subject_key, relation);
CREATE INDEX fact_object_idx  ON code.fact (project_id, revision_id, object_key, relation);
-- 同一 revision 内同一三元组只保留一条有效 fact
CREATE UNIQUE INDEX fact_unique_live ON code.fact
  (project_id, revision_id, subject_key, relation, object_key, analysis_engine)
  WHERE valid_to IS NULL;

CREATE TABLE code.fact_evidence (
  fact_id     uuid NOT NULL REFERENCES code.fact(id) ON DELETE CASCADE,
  evidence_id uuid NOT NULL,
  role        text NOT NULL DEFAULT 'PRIMARY',
  PRIMARY KEY (fact_id, evidence_id)
);

-- ============ graph_meta ============
CREATE TABLE graph_meta.ontology_version (
  id           uuid PRIMARY KEY,
  version      text NOT NULL UNIQUE,
  status       text NOT NULL CHECK (status IN ('DRAFT','PUBLISHED','DEPRECATED')),
  published_at timestamptz,
  published_by uuid
);

CREATE TABLE graph_meta.entity (
  id                uuid PRIMARY KEY,
  project_id        uuid NOT NULL,
  entity_type       text NOT NULL,
  natural_key       text NOT NULL,     -- 业务唯一键，如 fqn 或 screen code
  name              text NOT NULL,
  qualified_name    text,
  properties        jsonb NOT NULL DEFAULT '{}',
  ontology_version  text NOT NULL,
  revision          text NOT NULL,
  resolution        text NOT NULL,
  first_seen_rev    text NOT NULL,
  last_seen_rev     text NOT NULL,
  deleted_at        timestamptz,
  UNIQUE (project_id, entity_type, natural_key)
);
CREATE INDEX entity_name_trgm ON graph_meta.entity USING gin (name gin_trgm_ops);

CREATE TABLE graph_meta.relationship (
  id                uuid PRIMARY KEY,
  project_id        uuid NOT NULL,
  src_id            uuid NOT NULL REFERENCES graph_meta.entity(id),
  dst_id            uuid NOT NULL REFERENCES graph_meta.entity(id),
  rel_type          text NOT NULL,
  resolution        text NOT NULL,
  confidence        real NOT NULL,
  fact_id           uuid,
  revision          text NOT NULL,
  valid_to          timestamptz,
  UNIQUE (project_id, src_id, dst_id, rel_type, revision)
);
CREATE INDEX rel_src_idx ON graph_meta.relationship (src_id, rel_type) WHERE valid_to IS NULL;
CREATE INDEX rel_dst_idx ON graph_meta.relationship (dst_id, rel_type) WHERE valid_to IS NULL;

-- ============ test ============
CREATE TABLE test.validation_rule (
  id           uuid PRIMARY KEY,
  project_id   uuid NOT NULL,
  field_ref    text NOT NULL,
  kind         text NOT NULL CHECK (kind IN
                 ('REQUIRED','LENGTH','RANGE','PATTERN','CROSS_FIELD','CUSTOM')),
  condition    text NOT NULL,
  source       text NOT NULL CHECK (source IN ('CODE','SPEC','INFERRED','CONFLICT')),
  evidence_ids uuid[] NOT NULL DEFAULT '{}',
  resolution   text NOT NULL,
  created_at   timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE test.test_case (
  id            uuid PRIMARY KEY,
  project_id    uuid NOT NULL,
  scenario_id   uuid NOT NULL,
  code          text NOT NULL,          -- TC-2026-0118
  title         text NOT NULL,
  status        text NOT NULL CHECK (status IN ('DRAFT','APPROVED','DEPRECATED')),
  preconditions jsonb NOT NULL DEFAULT '[]',
  expected      jsonb NOT NULL DEFAULT '[]',
  covers_rules  uuid[] NOT NULL DEFAULT '{}',
  covers_impacts uuid[] NOT NULL DEFAULT '{}',
  automation    jsonb NOT NULL DEFAULT '{"type":"manual"}',
  generated_by  uuid,                   -- agent_run_id，人工创建为 NULL
  created_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE test.test_execution (
  id            uuid PRIMARY KEY,
  test_run_id   uuid NOT NULL,
  test_case_id  uuid NOT NULL,
  attempt       smallint NOT NULL DEFAULT 1,
  status        text NOT NULL CHECK (status IN
                  ('PASSED','FAILED','BLOCKED','SKIPPED','ERROR')),
  started_at    timestamptz NOT NULL,
  finished_at   timestamptz,
  duration_ms   integer,
  failure_kind  text,                   -- ASSERTION / TIMEOUT / ENV / SCRIPT
  artifacts     jsonb NOT NULL DEFAULT '[]',  -- s3 引用：截图/trace/log
  step_results  jsonb NOT NULL DEFAULT '[]'
);
CREATE UNIQUE INDEX exec_once ON test.test_execution (test_run_id, test_case_id, attempt);

CREATE TABLE test.coverage_snapshot (
  id             uuid PRIMARY KEY,
  project_id     uuid NOT NULL,
  test_run_id    uuid NOT NULL,
  scope          jsonb NOT NULL,        -- 影响范围 / 版本范围
  requirement    numeric(5,4) NOT NULL,
  rule           numeric(5,4) NOT NULL,
  screen         numeric(5,4) NOT NULL,
  impact         numeric(5,4) NOT NULL,
  regression     numeric(5,4) NOT NULL,
  detail         jsonb NOT NULL,
  computed_at    timestamptz NOT NULL DEFAULT now()
);
```

其余表（`test_plan / test_scenario / test_step / test_data_set / test_data_row / test_run / defect / impact_report / impact_item / agent / agent_version / prompt_template / agent_run / agent_step / approval_request / workflow_def / workflow_run / node_run / mcp_server / capability_binding / provider / provider_config / provider_health / secret`）结构从略，字段定义见 `docs/database/` 中的迁移脚本。

### 1.4 索引与分区策略

| 表 | 策略 | 理由 |
|---|---|---|
| `kb.chunk` | 按 `project_id` 哈希分区（> 500 万行时启用） | 单项目查询占绝大多数 |
| `code.fact` | 按 `revision_id` 分区（每 revision 一个分区） | 历史 revision 只读，可整体归档 |
| `core.audit_log` | 按 `occurred_at` 月分区 | 合规保留 + 冷热分离 |
| `test.test_execution` | 按 `started_at` 月分区 | 执行记录增长最快 |
| `kb.evidence` | 不分区，但按 `document_id` 聚簇 | 证据查询总是按文档 |

### 1.5 数据保留

| 数据 | 保留期 | 清理方式 |
|---|---|---|
| 原始文档 | 永久（客户可删） | 软删 → 30 天后物理删 |
| Chunk / 向量 | 跟随 revision | superseded 后 30 天 GC |
| Fact | 永久 | 按 revision 归档到冷存储 |
| 执行产物（截图/trace） | 180 天 | 对象存储生命周期 |
| 审计日志 | 3 年（可配） | 归档 |
| Agent 运行记录 | 1 年 | 归档 |

---

## 2. Neo4j 图模型

### 2.1 节点

```cypher
(:Entity {
  uid: "8f2c...:a91b...",   // project_id + ":" + entity_id，全局唯一键
  entity_id: "a91b...",     // 与 PG 一致
  project_id: "8f2c...",
  entity_type: "Method",
  name: "update",
  qualified_name: "com.example.UserEditAction#update",
  revision: "def456",
  resolution: "RESOLVED"
})
```

**为什么用 `uid` 而不是复合唯一约束**：Neo4j 的复合唯一约束 / 节点键在 Community 版上支持有限（节点键属 Enterprise 特性）。用 `uid` 单属性唯一约束可在所有版本工作，且查询计划更简单。

**不存**：正文、证据全文、大 JSON。这些留在 PG。

可选二级标签（仅高频类型，用于加速）：`:Screen` `:Table` `:Method` `:BusinessRule`。由投影器按 `entity_type` 附加。

### 2.2 关系

```cypher
(a:Entity)-[:CALLS {
  rel_id: "uuid",
  resolution: "RESOLVED",
  confidence: 1.0,
  fact_id: "uuid",
  revision: "def456"
}]->(b:Entity)
```

关系类型固定集合（由 Ontology 注册表约束），不允许动态创建。

### 2.3 约束与索引

```cypher
CREATE CONSTRAINT entity_uid IF NOT EXISTS
FOR (e:Entity) REQUIRE e.uid IS UNIQUE;

CREATE INDEX entity_lookup IF NOT EXISTS
FOR (e:Entity) ON (e.project_id, e.entity_type, e.name);

CREATE INDEX entity_qname IF NOT EXISTS
FOR (e:Entity) ON (e.project_id, e.qualified_name);

CREATE INDEX rel_revision IF NOT EXISTS
FOR ()-[r:CALLS]-() ON (r.revision);
```

> 关系属性索引（`FOR ()-[r:CALLS]-() ON ...`）在 Neo4j 5.x 可用；若客户使用更早版本，去掉该索引，遍历时以节点索引为主。

### 2.4 查询模板（只允许参数化值）

```cypher
// 邻居
MATCH (a:Entity {project_id:$pid, entity_id:$eid})-[r]->(b:Entity)
WHERE type(r) IN $rel_types AND r.revision = $rev
RETURN b.entity_id AS id, b.entity_type AS kind, b.name AS name,
       type(r) AS rel, r.resolution AS resolution, r.confidence AS confidence
LIMIT $limit;

// 最短路径（关系类型白名单由服务端枚举校验后拼接）
MATCH (a:Entity {project_id:$pid, entity_id:$from}),
      (b:Entity {project_id:$pid, entity_id:$to}),
      p = shortestPath((a)-[:CALLS|IMPLEMENTS|EXTENDS*..8]->(b))
RETURN [n IN nodes(p) | n.entity_id] AS node_ids,
       [r IN relationships(p) | type(r)] AS rels
LIMIT 1;
```

### 2.5 投影状态

```cypher
(:ProjectionState {
  project_id: "uuid",
  last_event_seq: 123456,
  last_event_id: "uuid",
  updated_at: datetime()
})
```

用于断点续投与漂移检测。

---

## 3. RAG 数据模型（逻辑视图）

```text
Document
 ├─ id, project_id, doc_type, title, business_domain, acl_groups
 └─ Revision (1..n)
     ├─ id, source_revision, content_hash, parser, parser_version, status
     └─ NodeTree
         ├─ node_id, parent_id, node_type, locator, ordinal
         └─ Chunk (1..n)
             ├─ chunk_id, chunk_type, text, token_count, content_hash
             ├─ locator          ← 证据定位
             ├─ node_path[]      ← 邻域扩展
             ├─ metadata{lang, screen_id, rule_hints[]}
             ├─ embedding[model_id] → vector
             └─ fts tsvector
```

### 3.1 Chunk ID 生成

```text
chunk_id = uuid_v5(
  namespace = project_id,
  name = f"{revision_id}:{locator_canonical}:{content_hash_hex[:16]}"
)
```

内容寻址的好处：同样的内容重复导入得到同一 ID，天然幂等；内容变了 ID 就变，不会出现「同一个 ID 指向不同文本」。

### 3.2 检索日志（用于评测与调优）

```sql
CREATE TABLE kb.retrieval_log (
  id            uuid PRIMARY KEY,
  project_id    uuid NOT NULL,
  query         text NOT NULL,
  query_tokens  text[] NOT NULL,
  filters       jsonb NOT NULL,
  mode          text NOT NULL,           -- SEMANTIC / FULLTEXT / HYBRID / GRAPH_ASSIST
  candidates    jsonb NOT NULL,          -- [{chunk_id, rank, score, source}]
  selected      uuid[] NOT NULL,
  rerank_used   boolean NOT NULL,
  latency_ms    integer NOT NULL,
  user_feedback smallint,                -- -1 / 0 / +1
  created_at    timestamptz NOT NULL DEFAULT now()
);
```

这张表是 RAG 能否持续改进的关键。没有它，「检索效果不好」永远只能靠感觉。

---

## 4. Analysis Fact Schema

### 4.1 JSON Schema（`contracts/facts/fact.schema.json`）

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "schema://fact@1",
  "type": "object",
  "required": ["fact_id","project_id","revision","subject","relation","object",
               "resolution","confidence","analysis_engine","engine_version","evidence"],
  "properties": {
    "fact_id":        { "type": "string", "format": "uuid" },
    "project_id":     { "type": "string", "format": "uuid" },
    "revision":       { "type": "string" },
    "subject":        { "$ref": "#/$defs/entityRef" },
    "relation":       { "type": "string", "pattern": "^[A-Z_]+$" },
    "object":         { "$ref": "#/$defs/entityRef" },
    "resolution":     { "enum": ["RESOLVED","INFERRED","PARTIAL","UNRESOLVED","CONFLICT"] },
    "confidence":     { "type": "number", "minimum": 0, "maximum": 1 },
    "analysis_engine":{ "type": "string" },
    "engine_version": { "type": "string" },
    "evidence":       { "type": "array", "minItems": 1, "items": { "$ref": "schema://evidence@1" } },
    "conflict_group": { "type": ["string","null"] },
    "unresolved_reason": { "type": ["string","null"] },
    "candidates":     { "type": "array", "items": { "$ref": "#/$defs/entityRef" } },
    "derived_from":   { "type": "array", "items": { "type": "string" } }
  },
  "$defs": {
    "entityRef": {
      "type": "object",
      "required": ["kind","key"],
      "properties": {
        "kind": { "type": "string" },
        "key":  { "type": "string" },
        "locator": { "type": ["object","null"] }
      }
    }
  }
}
```

### 4.2 五态语义（写死，不允许扩展解释）

| 状态 | 含义 | 是否可用于自动决策 | UI 表现 |
|---|---|---|---|
| `RESOLVED` | 解析器确定，唯一解 | 是 | 实线 |
| `INFERRED` | 有依据的推断，可能有其他解 | 否（需人工确认） | 虚线 + 提示 |
| `PARTIAL` | 部分解析（如只知道类，不知方法） | 否 | 点线 |
| `UNRESOLVED` | 无法确定 | 否 | 灰色虚线 + 问号 |
| `CONFLICT` | 多个来源给出互斥结论 | 否（需裁决） | 红色双线 |

`confidence` 与 `resolution` **不是一回事**：`INFERRED + confidence 0.95` 仍然不能当事实用。前者是「有多像」，后者是「是否确定」。

### 4.3 冲突表达

```json
{
  "conflict_group": "CG-2026-0042",
  "relation": "HANDLES",
  "subject": {"kind":"Screen","key":"USER_EDIT"},
  "candidates": [
    {"kind":"Action","key":"com.example.UserEditAction"},
    {"kind":"Action","key":"com.example.UserEditActionV2"}
  ],
  "resolution": "CONFLICT",
  "unresolved_reason": "struts-config.xml 与注解声明指向不同 Action，可能为迁移残留"
}
```

冲突必须展示给用户，而不是让系统「选一个看起来对的」。

---

## 5. Evidence Schema

```json
{
  "$id": "schema://evidence@1",
  "type": "object",
  "required": ["evidence_id","source_type","locator","quote","quote_hash","resolution"],
  "properties": {
    "evidence_id": { "type": "string" },
    "source_type": {
      "enum": ["PDF_PAGE","DOCX_SECTION","EXCEL_RANGE","MARKDOWN_SECTION",
               "CODE_LINE","CONFIG_PATH","TEST_RESULT","CHUNK"]
    },
    "document_id": { "type": ["string","null"] },
    "revision_id": { "type": ["string","null"] },
    "locator":     { "type": "object" },
    "quote":       { "type": "string", "maxLength": 2000 },
    "quote_hash":  { "type": "string" },
    "context_before": { "type": ["string","null"], "maxLength": 300 },
    "context_after":  { "type": ["string","null"], "maxLength": 300 },
    "resolution":  { "enum": ["RESOLVED","INFERRED","PARTIAL","UNRESOLVED","CONFLICT"] },
    "confidence":  { "type": "number" }
  }
}
```

三条约束：

1. `quote` 必须是原文的**精确子串**，服务端校验；不匹配则拒绝（防止 AI 编造引用）。
2. `locator` 必须能定位到可高亮的范围。
3. `quote_hash = sha256(normalize(quote))`，用于检测原文是否被篡改。

---

## 6. Test Asset Schema

### 6.1 用例（YAML 表示，存储为 JSONB）

见 04.6 的示例。关键字段约束：

| 字段 | 约束 |
|---|---|
| `steps[].action` | 枚举：`goto / fill / click / select / check / upload / wait / assert` |
| `steps[].target` | 必须用 role/label/text 定位，禁止裸 CSS（易碎） |
| `expected[]` | 必须可判定（文本可见 / 元素存在 / 值相等 / URL 匹配） |
| `covers` | 至少一项，否则不计入覆盖 |
| `evidence` | 至少一条，指向设计或代码 |

### 6.2 覆盖快照

```json
{
  "test_run_id": "RUN-2026-0112",
  "scope": {"impact_id": "IMP-2026-0007"},
  "requirement": 0.82,
  "rule": 0.64,
  "screen": 0.91,
  "impact": 0.73,
  "regression": 0.88,
  "detail": {
    "impact": {
      "total": 37, "covered": 27,
      "uncovered": ["II-014","II-019","II-022","II-025","II-027",
                    "II-029","II-031","II-033","II-035","II-036"]
    },
    "excluded": {"draft_cases": 4, "never_executed": 7, "skipped": 2}
  },
  "computed_at": "2026-11-05T15:20:00Z"
}
```

`uncovered` 必须列出具体 ID，而不是只给一个百分比。百分比不能指导行动。

---

## 7. Provider Schema

```json
{
  "provider_id": "prov-native-rag",
  "kind": "RAG",
  "impl": "native",
  "version": "0.3.1",
  "priority": 10,
  "enabled": true,
  "config": {
    "embedding_model_id": "uuid",
    "top_k": 60,
    "rrf_k": 60
  },
  "secret_refs": ["secret://openai/api_key"],
  "capabilities": {
    "semantic": "supported",
    "fulltext": "supported",
    "hybrid": "supported",
    "rerank": "supported",
    "metadata_filter": "supported",
    "graph_assist": "supported",
    "evidence_locator": "supported"
  },
  "limits": { "timeout_ms": 30000, "max_batch": 256 }
}
```

能力取值只能是 `supported / partial / unsupported / unknown`。`unknown` 不允许用于自动路由。

---

## 8. MCP Contract Schema

### 8.1 工具描述（对外）

见 04.9。补充约束：

| 字段 | 约束 |
|---|---|
| `name` | `^[a-z][a-z0-9_]{2,40}$`，全局唯一 |
| `version` | semver；破坏性变更必须升 major |
| `input_schema` / `output_schema` | 必须是可解析的 JSON Schema 2020-12 |
| `permission` | 必须存在于权限注册表 |
| `side_effects` | 必填，用于调用方决策 |
| `timeout_ms` | ≤ 300000 |

### 8.2 Server 注册

```json
{
  "server_id": "mcp-git-01",
  "name": "git-tools",
  "transport": "stdio",
  "command": ["git-mcp", "--repo", "/workspace"],
  "enabled": true,
  "tool_count": 7,
  "schema_hash": "sha256:...",
  "health": {"status": "HEALTHY", "checked_at": "2026-11-05T12:00:00Z"},
  "last_discovery_at": "2026-11-05T11:59:00Z"
}
```

`schema_hash` 变化即触发 `DEGRADED`，不会自动适配（见 04.9）。

### 8.3 内部 Capability 与 MCP Tool 的映射

```text
capability: knowledge.search
  ├─ mcp_tool: platform.knowledge_search      (对外)
  ├─ rest: POST /api/v1/knowledge/search      (对内/前端)
  └─ agent_tool: knowledge.search             (Agent)
```

三者共用同一个 Capability 实现，只是协议适配不同。**不允许一个能力有三套实现**。
