-- 016_ai.sql
-- Phase 7: Agent Runtime / MCP / Workflow の縦断実装。
-- 正本: docs/design/ja/detail/02-database-ddl.md §5（ai schema）、
--       04-コアエンジン.md §4.7（Agent）/ §4.8（Workflow）/ §4.9（MCP）。
--
-- 設計正本との差分（docs/phase7/acceptance.md に明記）:
--   1. 全表に project_id を付与し RLS を敷く（015_test と同じ方針）。
--   2. workflow_def は YAML ではなく JSON 定義を保存する（serde_yaml を増やさない）。
--   3. approval_request に payload / created_at を追加し、run からの参照を辿れるようにした。
--   4. mcp_server は project スコープ。設計は name 全体一意だが、単一プロジェクト前提に合わせる。

CREATE SCHEMA ai;

CREATE TABLE ai.agent (
  id uuid PRIMARY KEY,
  project_id uuid NOT NULL,
  code text NOT NULL,
  enabled boolean NOT NULL DEFAULT true,
  current_version integer NOT NULL DEFAULT 1,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (project_id, code)
);

CREATE TABLE ai.agent_version (
  id uuid PRIMARY KEY,
  project_id uuid NOT NULL,
  agent_id uuid NOT NULL REFERENCES ai.agent(id) ON DELETE CASCADE,
  version integer NOT NULL,
  model_profile text NOT NULL CHECK (model_profile IN ('fast', 'reasoning', 'long_context')),
  prompt_ref text NOT NULL,
  capabilities text[] NOT NULL DEFAULT '{}',
  budgets jsonb NOT NULL DEFAULT '{}',
  output_schema text,
  approval_required boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (agent_id, version)
);

CREATE TABLE ai.prompt_template (
  id uuid PRIMARY KEY,
  project_id uuid NOT NULL,
  ref text NOT NULL,
  version integer NOT NULL,
  body text NOT NULL,
  published_at timestamptz,
  UNIQUE (project_id, ref, version)
);

CREATE TABLE ai.agent_run (
  id uuid PRIMARY KEY,
  project_id uuid NOT NULL,
  agent_version_id uuid NOT NULL REFERENCES ai.agent_version(id),
  status text NOT NULL CHECK (status IN
    ('PENDING','RUNNING','SUCCEEDED','FAILED','WAITING_APPROVAL','CANCELLED','BUDGET_EXCEEDED')),
  input jsonb NOT NULL,
  output jsonb,
  cost jsonb NOT NULL DEFAULT '{}',
  error jsonb,
  started_at timestamptz NOT NULL DEFAULT now(),
  finished_at timestamptz,
  trace_id text
);
CREATE INDEX t7_agent_run_project_idx ON ai.agent_run (project_id, started_at DESC);

CREATE TABLE ai.agent_step (
  id uuid PRIMARY KEY,
  project_id uuid NOT NULL,
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
  id uuid PRIMARY KEY,
  project_id uuid NOT NULL,
  run_id uuid,
  workflow_run_id uuid,
  action text NOT NULL,
  payload_digest bytea NOT NULL,
  payload jsonb NOT NULL DEFAULT '{}',
  status text NOT NULL CHECK (status IN ('PENDING','APPROVED','REJECTED','EXPIRED')),
  requested_by text NOT NULL,
  decided_by text,
  decided_at timestamptz,
  comment text,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX t7_approval_project_idx ON ai.approval_request (project_id, created_at DESC);

CREATE TABLE ai.workflow_def (
  id uuid PRIMARY KEY,
  project_id uuid NOT NULL,
  code text NOT NULL,
  version integer NOT NULL,
  definition jsonb NOT NULL,
  status text NOT NULL CHECK (status IN ('DRAFT','PUBLISHED','DEPRECATED')),
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (project_id, code, version)
);

CREATE TABLE ai.workflow_run (
  id uuid PRIMARY KEY,
  project_id uuid NOT NULL,
  workflow_def_id uuid NOT NULL REFERENCES ai.workflow_def(id),
  status text NOT NULL CHECK (status IN
    ('PENDING','RUNNING','SUCCEEDED','FAILED','WAITING_APPROVAL','CANCELLED')),
  input jsonb NOT NULL,
  output jsonb,
  current_node text,
  started_at timestamptz,
  finished_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX t7_workflow_run_project_idx ON ai.workflow_run (project_id, created_at DESC);

CREATE TABLE ai.node_run (
  id uuid PRIMARY KEY,
  project_id uuid NOT NULL,
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
  id uuid PRIMARY KEY,
  project_id uuid NOT NULL,
  name text NOT NULL,
  transport text NOT NULL CHECK (transport IN ('stdio','http')),
  config jsonb NOT NULL DEFAULT '{}',
  enabled boolean NOT NULL DEFAULT true,
  schema_hash text,
  health_status text NOT NULL DEFAULT 'UNKNOWN'
    CHECK (health_status IN ('UNKNOWN','UP','DEGRADED','DOWN')),
  last_discovery_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (project_id, name)
);

ALTER TABLE ai.agent ENABLE ROW LEVEL SECURITY;
CREATE POLICY t7_agent_scope ON ai.agent
  FOR ALL USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

ALTER TABLE ai.agent_version ENABLE ROW LEVEL SECURITY;
CREATE POLICY t7_agent_version_scope ON ai.agent_version
  FOR ALL USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

ALTER TABLE ai.prompt_template ENABLE ROW LEVEL SECURITY;
CREATE POLICY t7_prompt_scope ON ai.prompt_template
  FOR ALL USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

ALTER TABLE ai.agent_run ENABLE ROW LEVEL SECURITY;
CREATE POLICY t7_agent_run_scope ON ai.agent_run
  FOR ALL USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

ALTER TABLE ai.agent_step ENABLE ROW LEVEL SECURITY;
CREATE POLICY t7_agent_step_scope ON ai.agent_step
  FOR ALL USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

ALTER TABLE ai.approval_request ENABLE ROW LEVEL SECURITY;
CREATE POLICY t7_approval_scope ON ai.approval_request
  FOR ALL USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

ALTER TABLE ai.workflow_def ENABLE ROW LEVEL SECURITY;
CREATE POLICY t7_workflow_def_scope ON ai.workflow_def
  FOR ALL USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

ALTER TABLE ai.workflow_run ENABLE ROW LEVEL SECURITY;
CREATE POLICY t7_workflow_run_scope ON ai.workflow_run
  FOR ALL USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

ALTER TABLE ai.node_run ENABLE ROW LEVEL SECURITY;
CREATE POLICY t7_node_run_scope ON ai.node_run
  FOR ALL USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

ALTER TABLE ai.mcp_server ENABLE ROW LEVEL SECURITY;
CREATE POLICY t7_mcp_scope ON ai.mcp_server
  FOR ALL USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

-- 014 / 015 の 15 分支を保持し、ai の 4 分支を追加する。
CREATE OR REPLACE FUNCTION core.project_of(kind text, p_id uuid) RETURNS uuid
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
      WHEN 'code_repository' THEN (SELECT project_id FROM code.repository WHERE id = p_id)
      WHEN 'code_revision' THEN (SELECT project_id FROM code.code_revision WHERE id = p_id)
      WHEN 'code_analysis' THEN (SELECT project_id FROM code.analysis_run WHERE id = p_id)
      WHEN 'code_source_file' THEN (SELECT project_id FROM code.source_file WHERE id = p_id)
      WHEN 'impact_assessment' THEN (
        SELECT project_id FROM impact.assessment_draft WHERE id = p_id
      )
      WHEN 'test_plan' THEN (SELECT project_id FROM test.test_plan WHERE id = p_id)
      WHEN 'test_case' THEN (SELECT project_id FROM test.test_case WHERE id = p_id)
      WHEN 'test_run' THEN (SELECT project_id FROM test.test_run WHERE id = p_id)
      WHEN 'test_execution' THEN (SELECT project_id FROM test.test_execution WHERE id = p_id)
      WHEN 'defect' THEN (SELECT project_id FROM test.defect WHERE id = p_id)
      WHEN 'agent_run' THEN (SELECT project_id FROM ai.agent_run WHERE id = p_id)
      WHEN 'workflow_run' THEN (SELECT project_id FROM ai.workflow_run WHERE id = p_id)
      WHEN 'approval' THEN (SELECT project_id FROM ai.approval_request WHERE id = p_id)
      WHEN 'mcp_server' THEN (SELECT project_id FROM ai.mcp_server WHERE id = p_id)
    END
  $$;
