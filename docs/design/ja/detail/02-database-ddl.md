# 詳細設計 02：データベース DDL

対象：PostgreSQL 18.6、`core` / `corpus` / `code` / `graph_meta` / `test` / `ai` / `ops` の 7 schema。
方針：`migrations/` の SQL を正とし、本稿は実装時の参照版。全テーブルを列挙する。

---

## 0. 共通規約

| 項目 | 規約 |
|---|---|
| 主キー | `uuid`（`gen_random_uuid()`）。監査ログのみ `bigserial` |
| 時刻 | `timestamptz`。UTC 保存 |
| 論理削除 | `deleted_at timestamptz`。物理削除は別ジョブ |
| 列挙 | `text` + `CHECK`。PG enum は使わない（マイグレーションが重い） |
| JSON | `jsonb`。スキーマは `contracts/` の JSON Schema で検証 |
| 命名 | テーブルは複数形にしない（`document`, `chunk`）。中間表は `a_b` |
| 索引名 | `{table}_{column}_{kind}`。例 `chunk_fts_idx` |
| パーティション | `HASH(id)`（chunk）、`HASH(chunk_id)`（embedding）、`RANGE(occurred_at)` 月次（audit）。**`project_id` はパーティションキーにしない**（§6）。`fact` / `test_execution` は分区しない |

### 0.1 DB ロール（3 つ。API と worker を混ぜない）

`SELECT … FOR UPDATE SKIP LOCKED` は **UPDATE 権限を要求する**。API に UPDATE を渡すとジョブ行を書き換えられてしまい、worker に渡さないとジョブを取り出せない。したがって役割を分ける。

| ロール | 用途 | 主な権限 | 持たせない権限 |
|---|---|---|---|
| `systivra_app` | API プロセス | `SELECT` / `INSERT`（全 `core` 表）、`UPDATE` は業務表のみ | `core.job` の `UPDATE`（取り出しは worker の仕事） |
| `systivra_worker` | worker / projector | `core.job` / `job_checkpoint` / `outbox_event` / `outbox_dead_letter` の `UPDATE` | 業務表の書き込み（store 経由に限る） |
| `systivra_scheduler` | ジョブ取り出しのみ | `core.job` の `UPDATE` + `scheduler_scope` 方針（横断閲覧） | それ以外の全テーブル |

- `core.audit_log` は**どれにも `UPDATE` / `DELETE` を渡さない**。追記専用を権限で担保する。
- RLS はロールを問わず効く。`systivra_app` / `systivra_worker` は必ず `SET LOCAL app.project_id` を設定する。全プロジェクト横断で取り出す必要があるのは**取り出しクエリだけ**なので、その範囲を `systivra_scheduler` に閉じる（`detail/09` §3 の 2 プロセス構成と対応）。
- **横断閲覧は `BYPASSRLS` ではなく RLS 方針で表現する。** `deployment/postgres/roles.sql` が `scheduler_scope`（`FOR ALL TO systivra_scheduler USING (true)`）を許可リストの 4 表（`core.job` / `core.project` / `ai.approval_request` / `test.test_execution`）に張る。
  - 理由 1: **マネージド PG では属性を付けられない場合がある。** Azure Database for PostgreSQL は PG16 未満、admin（`azure_pg_admin`）が非管理者ロールに `BYPASSRLS` を付与できない。RDS / Cloud SQL も制約が異なる。
  - 理由 2: 方針は `pg_policies` に並ぶので、**「誰が何を横断できるか」が一覧できる**。ロール属性は `\du` しないと見えない。
  - 方針は権限を付けない。RLS はフィルタのみで、`GRANT` は roles.sql の第 2 / 3 節が管轄。`USING (true)` を書いても元の `GRANT` 以上は触れない。
  - 縦覧したい表が増えたら **roles.sql の許可リストに追加する**（`verify-db.sh` が件数を検査する）。
- ロール名は環境で変わりうるため、マイグレーションの `REVOKE` は `pg_roles` に存在するときだけ適用する（`002_core_ops.sql` の `DO` ブロック）。

---

## 1. core

```sql
CREATE SCHEMA core;

CREATE TABLE core.workspace (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  code text NOT NULL UNIQUE,
  name text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE core.app_user (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  login_id text NOT NULL UNIQUE,
  display_name text NOT NULL,
  email text,
  password_hash text,                 -- argon2id。SSO のみなら NULL
  is_active boolean NOT NULL DEFAULT true,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE core.project (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  workspace_id uuid NOT NULL REFERENCES core.workspace(id),
  code text NOT NULL,
  name text NOT NULL,
  is_sample boolean NOT NULL DEFAULT false,
  default_locale text NOT NULL DEFAULT 'ja',
  source_kind text NOT NULL CHECK (source_kind IN ('GIT','SVN','DIR','SNAPSHOT')),
  settings jsonb NOT NULL DEFAULT '{}',
  created_at timestamptz NOT NULL DEFAULT now(),
  deleted_at timestamptz,
  UNIQUE (workspace_id, code)
);

CREATE TABLE core.project_member (
  project_id uuid NOT NULL REFERENCES core.project(id),
  user_id uuid NOT NULL REFERENCES core.app_user(id),
  role text NOT NULL CHECK (role IN ('OWNER','ARCHITECT','DEVELOPER','TESTER','VIEWER')),
  granted_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (project_id, user_id)
);

CREATE TABLE core.api_token (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id uuid NOT NULL REFERENCES core.app_user(id),
  token_hash bytea NOT NULL UNIQUE,
  name text NOT NULL,
  scopes text[] NOT NULL DEFAULT '{}',
  expires_at timestamptz,
  revoked_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now()
);

-- 018_sso。認証前に引くため RLS の対象外（app_user と同じ）。
-- subject は IdP の不変 ID。未知の subject を自動プロビジョニングしない。
CREATE TABLE core.external_identity (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id uuid NOT NULL REFERENCES core.app_user(id) ON DELETE CASCADE,
  provider text NOT NULL,
  subject text NOT NULL,
  email text,
  created_at timestamptz NOT NULL DEFAULT now(),
  last_login_at timestamptz,
  UNIQUE (provider, subject),
  UNIQUE (user_id, provider)
);

-- 019_session。認証中に引くため RLS の対象外。
-- Cookie は session id を含み、失効・期限はこの表で判定する。
CREATE TABLE core.session (
  id uuid PRIMARY KEY,
  user_id uuid NOT NULL REFERENCES core.app_user(id) ON DELETE CASCADE,
  created_at timestamptz NOT NULL DEFAULT now(),
  last_seen_at timestamptz NOT NULL DEFAULT now(),
  expires_at timestamptz NOT NULL,
  revoked_at timestamptz,
  ip_hash bytea,
  user_agent text,
  provider text,                 -- 021: IdP の issuer
  subject text,                  -- 021: IdP の sub
  idp_sid text                   -- 021: IdP の sid
);
CREATE INDEX session_user_idx ON core.session (user_id, created_at DESC);
CREATE INDEX session_live_idx ON core.session (expires_at) WHERE revoked_at IS NULL;

-- 022_scim。SCIM Groups と IdP グループ→ロールの対応。
CREATE TABLE core.external_group (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  provider text NOT NULL,
  external_id text NOT NULL,
  display_name text NOT NULL,
  role text NOT NULL CHECK (role IN ('OWNER','ARCHITECT','DEVELOPER','TESTER','VIEWER')),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (provider, external_id)
);

CREATE TABLE core.external_group_member (
  group_id uuid NOT NULL REFERENCES core.external_group(id) ON DELETE CASCADE,
  user_id uuid NOT NULL REFERENCES core.app_user(id) ON DELETE CASCADE,
  PRIMARY KEY (group_id, user_id)
);

CREATE TABLE core.job (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid NOT NULL,
  kind text NOT NULL,
  priority smallint NOT NULL DEFAULT 5,
  status text NOT NULL CHECK (status IN
    ('PENDING','QUEUED','RUNNING','WAITING_APPROVAL','SUCCEEDED','SUCCEEDED_WITH_ERRORS','FAILED','CANCELLED','TIMEOUT')),
  idempotency_key text NOT NULL,
  input jsonb NOT NULL,
  progress jsonb NOT NULL DEFAULT '{"done":0,"total":0,"phase":null}',
  attempt smallint NOT NULL DEFAULT 0,
  max_attempts smallint NOT NULL DEFAULT 3,
  locked_by text,
  heartbeat_at timestamptz,
  error jsonb,
  created_at timestamptz NOT NULL DEFAULT now(),
  started_at timestamptz,
  finished_at timestamptz,
  UNIQUE (project_id, idempotency_key)
);
CREATE INDEX job_claim_idx ON core.job (status, priority DESC, created_at)
  WHERE status IN ('PENDING','QUEUED');

CREATE TABLE core.job_checkpoint (
  job_id uuid NOT NULL REFERENCES core.job(id) ON DELETE CASCADE,
  seq integer NOT NULL,
  cursor jsonb NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (job_id, seq)
);

CREATE TABLE core.job_event (
  id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  job_id uuid NOT NULL REFERENCES core.job(id) ON DELETE CASCADE,
  event text NOT NULL,
  data jsonb NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX job_event_job_idx ON core.job_event (job_id, id);
-- RLS は job 経由。SSE の id はこの id を使う。

CREATE TABLE core.outbox_event (
  event_id uuid PRIMARY KEY DEFAULT gen_random_uuid(),  -- 冪等キー。投影側の last_event_id と対応
  aggregate text NOT NULL,
  aggregate_id uuid NOT NULL,
  seq bigint GENERATED ALWAYS AS IDENTITY NOT NULL,     -- 進捗表示用。正しさの根拠にはしない
  event_type text NOT NULL,
  payload jsonb NOT NULL,
  occurred_at timestamptz NOT NULL DEFAULT now(),
  published_at timestamptz,
  attempts smallint NOT NULL DEFAULT 0,
  last_error jsonb,
  dead_letter boolean NOT NULL DEFAULT false,
  UNIQUE (seq)
);
CREATE INDEX outbox_pending_idx ON core.outbox_event (seq)
  WHERE published_at IS NULL AND NOT dead_letter;

-- 部分失敗の退避。1 件の失敗でバッチを止めない（02-アーキテクチャ §4.3）
CREATE TABLE core.outbox_dead_letter (
  event_id uuid PRIMARY KEY REFERENCES core.outbox_event(event_id),
  failed_at timestamptz NOT NULL DEFAULT now(),
  attempts smallint NOT NULL,
  error jsonb NOT NULL,
  resolved_at timestamptz
);

CREATE TABLE core.audit_log (
  id bigserial,
  project_id uuid,
  actor_type text NOT NULL CHECK (actor_type IN ('USER','AGENT','SYSTEM')),
  actor_id text NOT NULL,
  action text NOT NULL,
  target_type text,
  target_id text,
  input_digest bytea,
  result text NOT NULL,
  detail jsonb NOT NULL DEFAULT '{}',
  trace_id text,
  occurred_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (id, occurred_at)          -- パーティションキーを含める
) PARTITION BY RANGE (occurred_at);
-- 月次パーティションは 24 か月先まで生成。UPDATE/DELETE は REVOKE。

CREATE TABLE core.secret (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  name text NOT NULL UNIQUE,
  ciphertext bytea NOT NULL,
  key_version integer NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);
```

---

## 2. corpus

```sql
CREATE SCHEMA corpus;

CREATE TABLE corpus.document (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid NOT NULL,
  doc_type text NOT NULL CHECK (doc_type IN (
    'REQUIREMENT','BASIC_DESIGN','DETAILED_DESIGN','UI_SPEC','DB_SPEC',
    'TEST_SPEC','API_SPEC','MANUAL','OTHER')),  -- 正は detail/04 §0.3
  title text NOT NULL,
  source_path text,
  business_domain text,
  acl_groups text[] NOT NULL DEFAULT '{}',
  current_revision_id uuid,
  created_at timestamptz NOT NULL DEFAULT now(),
  deleted_at timestamptz
);
CREATE INDEX document_project_idx ON corpus.document (project_id, doc_type)
  WHERE deleted_at IS NULL;

CREATE TABLE corpus.document_revision (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  document_id uuid NOT NULL REFERENCES corpus.document(id),
  source_revision text,
  content_hash bytea NOT NULL,
  object_key text NOT NULL,                 -- 原本
  parse_artifact_key text,                  -- 圧縮セル格子
  file_size bigint NOT NULL,
  mime_type text NOT NULL,
  status text NOT NULL CHECK (status IN ('CURRENT','SUPERSEDED','FAILED')),
  source_encoding text NOT NULL DEFAULT 'UTF-8',
  encoding_confidence real NOT NULL DEFAULT 1.0,
  parser text,
  parser_version text,
  acquired_at timestamptz NOT NULL,
  parsed_at timestamptz,
  UNIQUE (document_id, content_hash)
);
CREATE UNIQUE INDEX doc_current_rev ON corpus.document_revision (document_id)
  WHERE status = 'CURRENT';

CREATE TABLE corpus.parse_run (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  revision_id uuid NOT NULL REFERENCES corpus.document_revision(id) ON DELETE CASCADE,
  parser text NOT NULL,
  parser_version text NOT NULL,
  status text NOT NULL CHECK (status IN ('PENDING','RUNNING','SUCCEEDED','FAILED','PARTIAL')),
  started_at timestamptz,
  finished_at timestamptz,
  stats jsonb NOT NULL DEFAULT '{}',        -- sheets, cells, regions, chunks
  error jsonb
);

CREATE TABLE corpus.doc_node (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  revision_id uuid NOT NULL REFERENCES corpus.document_revision(id) ON DELETE CASCADE,
  parent_id uuid REFERENCES corpus.doc_node(id),
  node_type text NOT NULL CHECK (node_type IN
    ('DOCUMENT','SHEET','SECTION','TABLE','REGION','FIGURE','PARAGRAPH')),
  locator jsonb NOT NULL,
  ordinal integer NOT NULL,
  text text,
  meta jsonb NOT NULL DEFAULT '{}'
);
CREATE INDEX doc_node_parent_idx ON corpus.doc_node (revision_id, parent_id, ordinal);
-- セル単位のノードは作らない。Phase 1 は非分区（revision_id 索引）。
-- Phase 2 で 05-データモデル §1.4 の revision_id 分割を検討する。

CREATE TABLE corpus.chunk (
  id uuid NOT NULL,
  project_id uuid NOT NULL,
  document_id uuid NOT NULL,
  revision_id uuid NOT NULL,
  chunk_type text NOT NULL CHECK (chunk_type IN
    ('TEXT','TABLE','TABLE_ROWS','CODE','FIGURE','SHEET_REGION','SUMMARY')),
  ordinal integer NOT NULL,
  text text NOT NULL,
  tokenized text NOT NULL,
  token_count integer NOT NULL,
  char_count integer NOT NULL,
  content_hash bytea NOT NULL,
  locator jsonb NOT NULL,
  node_path text[] NOT NULL,
  metadata jsonb NOT NULL DEFAULT '{}',
  fts tsvector GENERATED ALWAYS AS (to_tsvector('simple', tokenized)) STORED,
  is_current boolean NOT NULL DEFAULT true,
  superseded_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (id, project_id)          -- HASH パーティションキー id を主キーに含める
) PARTITION BY HASH (id);
-- 32〜64 パーティション。各パーティションに fts GIN と scope index。
-- project_id をパーティションキーにしない：A-02 は単一プロジェクトなので
-- HASH(project_id) は全 1,200 万行を 1 区に集中させる。スコープは
-- (project_id, is_current) 索引 + RLS で強制し、pruning に依存しない。

CREATE TABLE corpus.embedding_model (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  name text NOT NULL,
  dim integer NOT NULL,
  distance text NOT NULL CHECK (distance IN ('COSINE','L2','IP')),
  provider text NOT NULL,
  revision text NOT NULL,
  enabled boolean NOT NULL DEFAULT true,
  UNIQUE (name, revision)
);
-- 001/005 で local-hash-1024（revision 1）、010 で local-hash-1024-v2（revision 2）を投入する。

CREATE TABLE corpus.embedding_1024 (
  chunk_id uuid NOT NULL,
  project_id uuid NOT NULL,          -- chunk.project_id と一致をアプリで保証（RLS / スコープ用）
  model_id uuid NOT NULL REFERENCES corpus.embedding_model(id),
  vec vector(1024) NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (chunk_id, model_id)
) PARTITION BY HASH (chunk_id);
-- corpus.chunk と分区数を揃えると、同一 UUID は同じ番号の分区に入る。
-- パーティションごとに HNSW。model_id は partial index の条件。
-- 分区キーは chunk_id なので project pruning は効かない。全分区へ並列 fan-out し、
-- 各分区で project_id = $1 を RLS + 索引で絞る（detail/03-performance.md §2）。

CREATE TABLE corpus.index_build (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid NOT NULL,
  model_id uuid NOT NULL,
  status text NOT NULL CHECK (status IN ('BUILDING','READY','FAILED','RETIRED')),
  chunk_count bigint NOT NULL DEFAULT 0,
  started_at timestamptz,
  finished_at timestamptz,
  error jsonb,
  is_active boolean NOT NULL DEFAULT false
);

CREATE UNIQUE INDEX index_build_active_idx
  ON corpus.index_build (project_id)
  WHERE is_active;

CREATE TABLE corpus.evidence (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid NOT NULL,
  source_type text NOT NULL,
  document_id uuid,
  revision_id uuid,
  chunk_id uuid,
  locator jsonb NOT NULL,
  quote text NOT NULL,
  quote_hash bytea NOT NULL,
  resolution text NOT NULL,
  confidence real NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX evidence_doc_idx ON corpus.evidence (document_id, revision_id);

CREATE TABLE corpus.retrieval_log (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid NOT NULL,
  query text NOT NULL,
  query_tokens text[] NOT NULL,
  filters jsonb NOT NULL,
  lexical_mode text NOT NULL CHECK (lexical_mode IN ('TOKENIZED','BIGRAM','BOTH')),
  candidates jsonb NOT NULL,
  selected uuid[] NOT NULL,
  rerank_used boolean NOT NULL,
  latency_ms integer NOT NULL,
  user_feedback smallint,
  created_at timestamptz NOT NULL DEFAULT now()
);
```

`core.project_of(kind, id)` は、パスに `{pid}` を持たない資源 API の認可で
「その資源が属するプロジェクト」を返す Phase 1 追加関数である。
`SECURITY DEFINER` で RLS を迂回するが、返すのは project_id だけに限定する。

```sql
CREATE FUNCTION core.project_of(kind text, p_id uuid) RETURNS uuid
  LANGUAGE sql STABLE SECURITY DEFINER
  SET search_path = pg_catalog
AS $$
  SELECT CASE kind
    WHEN 'document' THEN (SELECT project_id FROM corpus.document WHERE id = p_id)
    WHEN 'revision' THEN (
      SELECT d.project_id
        FROM corpus.document_revision r
        JOIN corpus.document d ON d.id = r.document_id
       WHERE r.id = p_id
    )
    WHEN 'chunk' THEN (SELECT project_id FROM corpus.chunk WHERE id = p_id LIMIT 1)
    WHEN 'evidence' THEN (SELECT project_id FROM corpus.evidence WHERE id = p_id)
    WHEN 'job' THEN (SELECT project_id FROM core.job WHERE id = p_id)
  END
$$;
```

---

## 3. code

```sql
CREATE SCHEMA code;

CREATE TABLE code.repository (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid NOT NULL,
  kind text NOT NULL CHECK (kind IN ('GIT','SVN','DIR','SNAPSHOT')),
  url text,
  default_branch text,
  settings jsonb NOT NULL DEFAULT '{}'
);

CREATE TABLE code.code_revision (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  repository_id uuid NOT NULL REFERENCES code.repository(id),
  revision text NOT NULL,
  parent_id uuid REFERENCES code.code_revision(id),
  tree_digest bytea NOT NULL,
  committed_at timestamptz,
  is_baseline boolean NOT NULL DEFAULT false,
  UNIQUE (repository_id, revision)
);

CREATE TABLE code.source_file (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  revision_id uuid NOT NULL REFERENCES code.code_revision(id) ON DELETE CASCADE,
  path text NOT NULL,
  language text NOT NULL,
  content_hash bytea NOT NULL,
  size_bytes bigint NOT NULL,
  encoding text NOT NULL DEFAULT 'UTF-8',
  loc integer NOT NULL,
  line_offsets bytea,
  UNIQUE (revision_id, path)
);

CREATE TABLE code.symbol (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  revision_id uuid NOT NULL,
  file_id uuid NOT NULL,
  kind text NOT NULL CHECK (kind IN ('CLASS','METHOD','FIELD','INTERFACE','ENUM')),
  fqn text NOT NULL,
  simple_name text NOT NULL,
  signature text,
  line_start integer NOT NULL,
  line_end integer NOT NULL,
  modifiers text[] NOT NULL DEFAULT '{}',
  metadata jsonb NOT NULL DEFAULT '{}'
);
CREATE INDEX symbol_fqn_idx ON code.symbol (revision_id, fqn);

CREATE TABLE code.analysis_run (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid NOT NULL,
  revision_id uuid NOT NULL,
  status text NOT NULL CHECK (status IN ('PENDING','RUNNING','SUCCEEDED','FAILED','PARTIAL')),
  engine_versions jsonb NOT NULL DEFAULT '{}',
  -- 互換用。最終設計では classpath 型解決を行わないため常に false。
  classpath_complete boolean NOT NULL DEFAULT false,
  stats jsonb NOT NULL DEFAULT '{}',
  started_at timestamptz,
  finished_at timestamptz,
  error jsonb
);

-- 互換用 schema。最終設計では登録・参照しない（ADR-017）。
CREATE TABLE code.classpath_index (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid NOT NULL,
  revision_id uuid NOT NULL,
  artifact_key text NOT NULL,
  jar_count integer NOT NULL DEFAULT 0,
  complete boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE code.fact (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid NOT NULL,
  revision_id uuid NOT NULL,
  subject_kind text NOT NULL,
  subject_key text NOT NULL,
  relation text NOT NULL,
  object_kind text NOT NULL,
  object_key text NOT NULL,
  resolution text NOT NULL CHECK (resolution IN
    ('RESOLVED','INFERRED','PARTIAL','UNRESOLVED','CONFLICT')),
  confidence real NOT NULL CHECK (confidence BETWEEN 0 AND 1),
  analysis_engine text NOT NULL,
  engine_version text NOT NULL,
  conflict_group uuid,
  unresolved_reason text,
  candidates jsonb NOT NULL DEFAULT '[]',
  fact_hash bytea NOT NULL,
  valid_from timestamptz NOT NULL DEFAULT now(),
  valid_to timestamptz,
  superseded_by uuid REFERENCES code.fact(id),
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX fact_subject_idx ON code.fact (project_id, revision_id, subject_key, relation);
CREATE INDEX fact_object_idx  ON code.fact (project_id, revision_id, object_key, relation);
CREATE UNIQUE INDEX fact_unique_live ON code.fact
  (project_id, revision_id, subject_key, relation, object_key, analysis_engine)
  WHERE valid_to IS NULL;

-- 運用規約（ADR-004）：
-- 増分再解析では「旧行を supersede してから新行を INSERT」を **1 トランザクション**で行う。
--   1) UPDATE code.fact SET valid_to = now(), superseded_by = $new_id
--        WHERE <同一キー> AND valid_to IS NULL;
--   2) INSERT INTO code.fact (...) VALUES (...);
-- 順序を逆にすると fact_unique_live に衝突する。repository 層の
-- `supersede_then_insert(tx, facts)` 以外から code.fact へ INSERT させない。
-- 統合テスト：同一キーの再解析 2 回 / 並列 2 セッション / ロールバック後の
-- live 行数が 1 であることを CI で検証する。

CREATE TABLE code.fact_evidence (
  fact_id uuid NOT NULL REFERENCES code.fact(id) ON DELETE CASCADE,
  evidence_id uuid NOT NULL,
  role text NOT NULL DEFAULT 'PRIMARY',
  PRIMARY KEY (fact_id, evidence_id)
);
```

---

## 4. graph_meta

```sql
CREATE SCHEMA graph_meta;

CREATE TABLE graph_meta.ontology_version (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  version text NOT NULL UNIQUE,
  status text NOT NULL CHECK (status IN ('DRAFT','PUBLISHED','DEPRECATED')),
  published_at timestamptz,
  published_by uuid
);

CREATE TABLE graph_meta.entity_type (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  ontology_version_id uuid NOT NULL REFERENCES graph_meta.ontology_version(id),
  name text NOT NULL,
  display_name_ja text NOT NULL,
  parent_type text,
  is_abstract boolean NOT NULL DEFAULT false,
  properties_schema jsonb NOT NULL DEFAULT '{}',
  UNIQUE (ontology_version_id, name)
);

CREATE TABLE graph_meta.relation_type (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  ontology_version_id uuid NOT NULL REFERENCES graph_meta.ontology_version(id),
  name text NOT NULL,
  display_name_ja text NOT NULL,
  source_types text[] NOT NULL,
  target_types text[] NOT NULL,
  cardinality text NOT NULL DEFAULT 'N:M',
  is_transitive boolean NOT NULL DEFAULT false,
  properties_schema jsonb NOT NULL DEFAULT '{}',
  UNIQUE (ontology_version_id, name)
);

CREATE TABLE graph_meta.entity (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),   -- API / グラフ上の entity_id
  project_id uuid NOT NULL,
  entity_type text NOT NULL,
  natural_key text NOT NULL,
  name text NOT NULL,
  qualified_name text,
  properties jsonb NOT NULL DEFAULT '{}',
  ontology_version text NOT NULL,
  revision text NOT NULL,
  resolution text NOT NULL,
  first_seen_rev text NOT NULL,
  last_seen_rev text NOT NULL,
  projection_seq bigint NOT NULL DEFAULT 0,   -- 順序逆転防止（Neo4j 側の e.projection_seq と同値）
  deleted_at timestamptz,
  UNIQUE (project_id, entity_type, natural_key)
);
CREATE INDEX entity_name_trgm ON graph_meta.entity USING gin (name gin_trgm_ops);

CREATE TABLE graph_meta.relationship (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid NOT NULL,
  src_id uuid NOT NULL REFERENCES graph_meta.entity(id),
  dst_id uuid NOT NULL REFERENCES graph_meta.entity(id),
  rel_type text NOT NULL,
  resolution text NOT NULL,
  confidence real NOT NULL,
  fact_id uuid,
  revision text NOT NULL,
  valid_to timestamptz,
  projection_seq bigint NOT NULL DEFAULT 0,   -- 順序逆転防止（Neo4j 側の r.projection_seq と同値）
  UNIQUE (project_id, src_id, dst_id, rel_type, revision)
);
CREATE INDEX rel_src_idx ON graph_meta.relationship (src_id, rel_type) WHERE valid_to IS NULL;
CREATE INDEX rel_dst_idx ON graph_meta.relationship (dst_id, rel_type) WHERE valid_to IS NULL;
CREATE INDEX rel_scope_idx ON graph_meta.relationship (project_id) WHERE valid_to IS NULL;

CREATE TABLE graph_meta.projection_state (
  project_id uuid PRIMARY KEY,
  last_event_seq bigint NOT NULL DEFAULT 0,
  last_event_id uuid,
  updated_at timestamptz NOT NULL DEFAULT now()
);

-- 全件再構築は「影ラベル」に書いてから切替える（detail/07-graph-projection.md §5）
CREATE TABLE graph_meta.projection_build (
  build_id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid NOT NULL,
  status text NOT NULL CHECK (status IN
    ('BUILDING','VERIFYING','READY','SWITCHED','FAILED','ABORTED')),
  shadow_label text NOT NULL,             -- 例 'Entity__b7c1'
  base_event_seq bigint NOT NULL,
  entities_written bigint NOT NULL DEFAULT 0,
  relationships_written bigint NOT NULL DEFAULT 0,
  drift_rate numeric(6,5),
  started_at timestamptz NOT NULL DEFAULT now(),
  finished_at timestamptz
);
```

---

## 5. test / ai / ops（主要列のみ）

> **2026-11-07 の実装差分**（正本は下の DDL、実ファイルは `migrations/015_test.sql`）。
> 差分の理由と受入結果は `docs/phase6/acceptance.md` に記録する。
>
> - 子表（scenario / step / data_set / data_row / execution）にも `project_id` を付与し、
>   全 11 表で RLS を有効化した。スコープ設定漏れを「0 件」にするため。
> - `test_case` に `dsl` / `covers_screens` / `destructive` を追加。
> - `test_run` に `environment_id` / `lock_acquired` / `job_id` と、活性 run を 1 つに
>   限る部分一意索引を追加。`status` に `READY`（Runner 待ち）を追加。
> - `test_execution` に `execution_source` / `expected` / `actual` / `fingerprint` と
>   `failure_kind` の 8 値 CHECK、`status` の `PENDING` を追加。
> - `defect` に `fingerprint` と部分一意索引（再発を 1 件に集約）を追加。
> - `coverage_snapshot` を `(project_id, test_run_id)` で一意にし、再計算を冪等にした。
> - `ai` / `ops` schema は未作成。

```sql
CREATE SCHEMA test;

CREATE TABLE test.validation_rule (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid NOT NULL,
  field_ref text NOT NULL,
  kind text NOT NULL CHECK (kind IN ('REQUIRED','LENGTH','RANGE','PATTERN','CROSS_FIELD','CUSTOM')),
  condition text NOT NULL,
  source text NOT NULL CHECK (source IN ('CODE','SPEC','INFERRED','CONFLICT')),
  evidence_ids uuid[] NOT NULL DEFAULT '{}',
  resolution text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE test.test_plan (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid NOT NULL,
  code text NOT NULL,
  name text NOT NULL,
  status text NOT NULL CHECK (status IN ('DRAFT','APPROVED','RUNNING','DONE')),
  scope jsonb NOT NULL DEFAULT '{}',
  impact_id uuid,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE test.test_scenario (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  plan_id uuid NOT NULL REFERENCES test.test_plan(id),
  code text NOT NULL,
  title text NOT NULL,
  kind text NOT NULL,
  evidence_ids uuid[] NOT NULL DEFAULT '{}'
);

CREATE TABLE test.test_case (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid NOT NULL,
  scenario_id uuid NOT NULL REFERENCES test.test_scenario(id),
  code text NOT NULL,
  title text NOT NULL,
  status text NOT NULL CHECK (status IN ('DRAFT','APPROVED','DEPRECATED')),
  preconditions jsonb NOT NULL DEFAULT '[]',
  expected jsonb NOT NULL DEFAULT '[]',
  covers_rules uuid[] NOT NULL DEFAULT '{}',
  covers_impacts uuid[] NOT NULL DEFAULT '{}',
  automation jsonb NOT NULL DEFAULT '{"type":"manual"}',
  generated_by uuid,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE test.test_step (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  case_id uuid NOT NULL REFERENCES test.test_case(id) ON DELETE CASCADE,
  ordinal integer NOT NULL,
  action text NOT NULL,
  target jsonb NOT NULL DEFAULT '{}',
  value text,
  UNIQUE (case_id, ordinal)
);

CREATE TABLE test.test_data_set (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  case_id uuid NOT NULL REFERENCES test.test_case(id) ON DELETE CASCADE,
  label text NOT NULL,
  kind text NOT NULL CHECK (kind IN ('VALID','INVALID','BOUNDARY')),
  rule_id uuid
);

CREATE TABLE test.test_data_row (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  data_set_id uuid NOT NULL REFERENCES test.test_data_set(id) ON DELETE CASCADE,
  field_ref text NOT NULL,
  value text NOT NULL
);

CREATE TABLE test.test_run (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid NOT NULL,
  name text NOT NULL,
  environment jsonb NOT NULL,
  status text NOT NULL CHECK (status IN ('QUEUED','RUNNING','DONE','FAILED','CANCELLED')),
  case_count integer NOT NULL DEFAULT 0,
  started_at timestamptz,
  finished_at timestamptz
);

CREATE TABLE test.test_execution (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  test_run_id uuid NOT NULL REFERENCES test.test_run(id),
  test_case_id uuid NOT NULL,
  attempt smallint NOT NULL DEFAULT 1,
  status text NOT NULL CHECK (status IN ('PASSED','FAILED','BLOCKED','SKIPPED','ERROR')),
  started_at timestamptz NOT NULL,
  finished_at timestamptz,
  duration_ms integer,
  failure_kind text,
  artifacts jsonb NOT NULL DEFAULT '[]',
  step_results jsonb NOT NULL DEFAULT '[]'
);
CREATE UNIQUE INDEX exec_once ON test.test_execution (test_run_id, test_case_id, attempt);

CREATE TABLE test.defect (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid NOT NULL,
  execution_id uuid,
  title text NOT NULL,
  severity text NOT NULL,
  status text NOT NULL,
  impact_item_id uuid,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE test.coverage_snapshot (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid NOT NULL,
  test_run_id uuid NOT NULL,
  scope jsonb NOT NULL,
  requirement numeric(5,4) NOT NULL,
  rule numeric(5,4) NOT NULL,
  screen numeric(5,4) NOT NULL,
  impact numeric(5,4) NOT NULL,
  regression numeric(5,4) NOT NULL,
  detail jsonb NOT NULL,
  computed_at timestamptz NOT NULL DEFAULT now()
);

CREATE SCHEMA ai;

CREATE TABLE ai.agent (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid,
  code text NOT NULL,
  enabled boolean NOT NULL DEFAULT true,
  current_version integer NOT NULL DEFAULT 1
);

CREATE TABLE ai.agent_version (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  agent_id uuid NOT NULL REFERENCES ai.agent(id),
  version integer NOT NULL,
  model_profile text NOT NULL,
  prompt_ref text NOT NULL,
  capabilities text[] NOT NULL,
  budgets jsonb NOT NULL,
  output_schema text,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (agent_id, version)
);

CREATE TABLE ai.prompt_template (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid,
  ref text NOT NULL,
  version integer NOT NULL,
  body text NOT NULL,
  published_at timestamptz,
  UNIQUE (project_id, ref, version)
);

CREATE TABLE ai.agent_run (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid NOT NULL,
  agent_version_id uuid NOT NULL,
  status text NOT NULL CHECK (status IN ('RUNNING','SUCCEEDED','FAILED','CANCELLED','BUDGET_EXCEEDED')),
  input jsonb NOT NULL,
  output jsonb,
  cost jsonb NOT NULL DEFAULT '{}',
  started_at timestamptz NOT NULL DEFAULT now(),
  finished_at timestamptz,
  trace_id text
);

CREATE TABLE ai.agent_step (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  run_id uuid NOT NULL REFERENCES ai.agent_run(id) ON DELETE CASCADE,
  ordinal integer NOT NULL,
  kind text NOT NULL CHECK (kind IN ('LLM','CAPABILITY','APPROVAL')),
  name text NOT NULL,
  input_digest bytea,
  output_digest bytea,
  status text NOT NULL,
  error jsonb,
  UNIQUE (run_id, ordinal)
);

CREATE TABLE ai.approval_request (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid NOT NULL,
  run_id uuid,
  action text NOT NULL,
  payload_digest bytea NOT NULL,
  status text NOT NULL CHECK (status IN ('PENDING','APPROVED','REJECTED','EXPIRED')),
  requested_by text NOT NULL,
  decided_by text,
  decided_at timestamptz,
  comment text
);

CREATE TABLE ai.workflow_def (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid,
  code text NOT NULL,
  version integer NOT NULL,
  yaml text NOT NULL,
  status text NOT NULL CHECK (status IN ('DRAFT','PUBLISHED','DEPRECATED')),
  UNIQUE (project_id, code, version)
);

CREATE TABLE ai.workflow_run (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  workflow_def_id uuid NOT NULL,
  status text NOT NULL CHECK (status IN ('PENDING','RUNNING','SUCCEEDED','FAILED','WAITING_APPROVAL','CANCELLED')),
  input jsonb NOT NULL,
  started_at timestamptz,
  finished_at timestamptz
);

CREATE TABLE ai.node_run (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  run_id uuid NOT NULL REFERENCES ai.workflow_run(id) ON DELETE CASCADE,
  node_id text NOT NULL,
  attempt smallint NOT NULL DEFAULT 1,
  status text NOT NULL,
  output jsonb,
  error jsonb,
  started_at timestamptz,
  finished_at timestamptz,
  UNIQUE (run_id, node_id, attempt)
);

CREATE TABLE ai.mcp_server (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  name text NOT NULL UNIQUE,
  transport text NOT NULL CHECK (transport IN ('stdio','http')),
  config jsonb NOT NULL,
  enabled boolean NOT NULL DEFAULT true,
  schema_hash text,
  health_status text NOT NULL DEFAULT 'UNKNOWN',
  last_discovery_at timestamptz
);

CREATE TABLE ai.capability_binding (
  capability text NOT NULL,
  provider_id uuid NOT NULL,
  priority integer NOT NULL DEFAULT 10,
  fallback_order integer NOT NULL DEFAULT 0,
  PRIMARY KEY (capability, provider_id)
);

CREATE SCHEMA ops;

CREATE TABLE ops.provider (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  kind text NOT NULL,
  impl_name text NOT NULL,
  version text NOT NULL,
  priority integer NOT NULL DEFAULT 10,
  enabled boolean NOT NULL DEFAULT true,
  config jsonb NOT NULL DEFAULT '{}',
  secret_refs text[] NOT NULL DEFAULT '{}'
);

CREATE TABLE ops.provider_health (
  provider_id uuid NOT NULL REFERENCES ops.provider(id),
  checked_at timestamptz NOT NULL,
  status text NOT NULL,
  latency_ms integer,
  message text,
  PRIMARY KEY (provider_id, checked_at)
);
```

---

## 6. パーティション運用

| テーブル | 方式 | 作成 | 入れ替え |
|---|---|---|---|
| `corpus.chunk` | HASH(id) 32〜64 | マイグレーションで一括作成 | 不要（ハッシュ固定） |
| `corpus.embedding_1024` | HASH(chunk_id) 32〜64（**`corpus.chunk` と同数**） | 同上。各パーティションに HNSW | モデル切替は `model_id` 条件で別索引 |
| `code.fact` | パーティションしない | 250 万行は単一表で十分。`(project_id, revision_id)` 索引。古い revision はエクスポート後に削除 |
| `core.audit_log` | RANGE(occurred_at) 月次 | 24 か月先まで事前作成 | 3 年でアーカイブ |
| `test.test_execution` | **パーティションしない**（本文 DDL どおり） | — | 10 万行規模。180 日で DELETE + アーカイブ |

> **`project_id` をパーティションキーにしない**。A-02 は単一プロジェクト 60,000 冊であり、`HASH(project_id)` では全行が 1 区に集中して 32〜64 分割が機能しない。`HASH(id)` / `HASH(chunk_id)` で均等分散し、`project_id` は列 + 索引 + RLS で強制する。検索は全分区へ並列 fan-out（`detail/03-performance.md` §2）。`05-データモデル.md` §1.4 と本文 DDL はこの方針で統一済み。

---

## 7. マイグレーション順

```text
001_core             workspace, app_user, project, member, api_token
002_core_ops         job, checkpoint, outbox, outbox_dead_letter, audit, secret
003_authz            project / project_member の自己所属参照ポリシー
004_corpus_document  document, document_revision, parse_run
005_corpus_chunk     doc_node, chunk（パーティション）, embedding_model, embedding_1024
006_corpus_evidence  evidence, retrieval_log, index_build
007_job_events       job_event, job.status 制約
008_project_of       core.project_of()
009_rename_corpus    旧 kb → corpus の互換リネーム（新規 DB では no-op）
010_index_build_active  index_build.is_active + 一意索引 + local-hash-1024-v2
011_graph            ontology, entity, relationship, projection_state, projection_build
012_graph_relationship_seq  relationship.projection_seq（順序逆転防止）
013_code             repository, revision, source_file, symbol, analysis_run, fact
014_impact           impact.assessment_draft（変更影響 Draft）
015_test             validation_rule, plan, scenario, case, step, data, run, execution, defect, coverage
016_ai               agent, agent_version, prompt, agent_run, agent_step, approval, workflow, mcp
017_inquiry          impact.change_inquiry（Q-01 の質問・候補・5 セクション）
018_sso              core.external_identity（OIDC の issuer + subject と app_user の紐付け）
019_session          core.session（失効可能なサーバー側セッション）
020_ops              （未作成。将来の provider / capability_binding 用に予約）
021_session_identity session.provider / subject / idp_sid（IdP 主導ログアウト）
022_scim             external_group / external_group_member（SCIM・グループ→ロール）
023_first_project    core.first_project()（SCIM 同期用の単一プロジェクト解決）
024_code_schema_repair  旧 013 適用 DB の code 列・索引を現行 schema へ冪等修復
```

`009_rename_corpus` が通し番号 009、`010_index_build_active` が 010、`011_graph` / `012_graph_relationship_seq`
が 011 / 012 を使用するため、Phase 4 の code schema は `013_code` から開始する。
2026-11-07 時点で 014 は影響分析、015 はテスト工学、016 は AI / MCP / Workflow、
017 は Q-01 変更影響問合せ、018 は OIDC SSO、019 はサーバー側セッション、
021 は IdP 紐付け、022 は SCIM、023 は `core.first_project()`、024 は旧 code schema の
前進修復に割り当て済み。020 は将来の ops 用に予約。**次のマイグレーションは 025 以降**。
旧表の `014_test` / `015_ai` / `016_ops` は実装と
食い違っていたため、実ファイルの並びに合わせて修正した。
旧 `kb` DB を持つ環境は `docs/phase1/handoff.md` の手順で再作成する（`009` は新規 DB では no-op）。

`015_test.sql` は `core.project_of` を `CREATE OR REPLACE` で全体書換えする。
014 の 10 分支を保持したうえで test の 5 分支を追加しており、
`scripts/verify-db.sh` に旧分支の回帰検査がある。

各マイグレーションは forward-only。ロールバックが必要な場合は、新しいマイグレーションで打ち消す。

---

## 8. 保持と削除

| データ | 保持 | 方法 |
|---|---|---|
| 原本 / artifact | 永久（顧客削除時） | 論理削除 → 30 日後に S3 削除 |
| chunk / embedding | revision に追従 | supersede 後 30 日で物理削除 |
| fact | 永久 | revision 単位でアーカイブ |
| 監査 | 3 年 | 月次パーティションをデタッチ |
| 実行結果 | 180 日 | パーティションをデタッチ、S3 はライフサイクル |
| agent run | 1 年 | 同上 |
