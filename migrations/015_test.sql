-- 015_test.sql
-- Phase 6: Test Engineering の縦断実装。
-- 正本: docs/design/ja/detail/02-database-ddl.md §5 / detail/09-test-runner.md
--
-- 設計正本との差分（docs/phase6/acceptance.md に明記）:
--   1. 子表（scenario / step / data_set / data_row / execution）にも project_id を付与し、
--      全表で RLS を有効にした。設計 DDL は親の FK で辿る想定だが、実装では
--      「スコープを設定し忘れても 0 件になる」防御線を全表に敷く（013_code.sql と同じ方針）。
--   2. test_execution.status に 'PENDING' を追加。Runner 未接続の縦断では
--      「展開済みだが未実行」を明示し、被覆 0 として扱う（detail/09 §9.3）。
--   3. test_execution.execution_source を追加。IMPORTED / PLAYWRIGHT / SIMULATED を
--      区別し、Runner を実装していないのに実測と偽らない。
--   4. test_run.environment_id / lock_acquired と部分一意索引で環境ロックを表現
--      （detail/09 §5.3 / R-12）。advisory lock はセッション跨ぎで保持できないため、
--      永続化的な「アクティブ run は 1 つ」制約に置き換えた。
--   5. coverage_snapshot は (project_id, test_run_id) で一意。再計算は冪等
--      （detail/09 §9.4 / R-02）。

CREATE SCHEMA test;

CREATE TABLE test.validation_rule (
  id uuid PRIMARY KEY,
  project_id uuid NOT NULL,
  field_ref text NOT NULL,
  kind text NOT NULL CHECK (kind IN
    ('REQUIRED','LENGTH','RANGE','PATTERN','CROSS_FIELD','CUSTOM')),
  condition text NOT NULL,
  source text NOT NULL CHECK (source IN ('CODE','SPEC','INFERRED','CONFLICT')),
  evidence_ids uuid[] NOT NULL DEFAULT '{}',
  resolution text NOT NULL CHECK (resolution IN
    ('RESOLVED','PARTIAL','INFERRED','UNRESOLVED','CONFLICT')),
  condition_key text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX t6_rule_project_idx ON test.validation_rule (project_id, field_ref);

CREATE TABLE test.test_plan (
  id uuid PRIMARY KEY,
  project_id uuid NOT NULL,
  code text NOT NULL,
  name text NOT NULL,
  status text NOT NULL CHECK (status IN ('DRAFT','APPROVED','RUNNING','DONE')),
  scope jsonb NOT NULL DEFAULT '{}',
  impact_id uuid,
  origin text NOT NULL DEFAULT 'MANUAL'
                CHECK (origin IN ('MANUAL','Q01_DERIVED')),
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX t6_plan_project_idx ON test.test_plan (project_id, created_at DESC);

CREATE TABLE test.test_scenario (
  id uuid PRIMARY KEY,
  project_id uuid NOT NULL,
  plan_id uuid NOT NULL REFERENCES test.test_plan(id) ON DELETE CASCADE,
  code text NOT NULL,
  title text NOT NULL,
  kind text NOT NULL CHECK (kind IN
    ('NORMAL','ERROR','BOUNDARY','PERMISSION','STATE','DOUBLE_SUBMIT','CONCURRENT','ROLLBACK')),
  evidence_ids uuid[] NOT NULL DEFAULT '{}'
);
CREATE INDEX t6_scenario_plan_idx ON test.test_scenario (project_id, plan_id);

CREATE TABLE test.test_case (
  id uuid PRIMARY KEY,
  project_id uuid NOT NULL,
  scenario_id uuid NOT NULL REFERENCES test.test_scenario(id) ON DELETE CASCADE,
  code text NOT NULL,
  title text NOT NULL,
  status text NOT NULL CHECK (status IN ('DRAFT','APPROVED','DEPRECATED')),
  dsl jsonb NOT NULL DEFAULT '{}',
  preconditions jsonb NOT NULL DEFAULT '[]',
  expected jsonb NOT NULL DEFAULT '[]',
  covers_rules uuid[] NOT NULL DEFAULT '{}',
  covers_impacts uuid[] NOT NULL DEFAULT '{}',
  covers_screens text[] NOT NULL DEFAULT '{}',
  automation jsonb NOT NULL DEFAULT '{"type":"manual"}',
  destructive boolean NOT NULL DEFAULT false,
  generated_by uuid,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX t6_case_project_idx ON test.test_case (project_id, status, code);
CREATE INDEX t6_case_impacts_idx ON test.test_case USING gin (covers_impacts);

CREATE TABLE test.test_step (
  id uuid PRIMARY KEY,
  project_id uuid NOT NULL,
  case_id uuid NOT NULL REFERENCES test.test_case(id) ON DELETE CASCADE,
  ordinal integer NOT NULL,
  action text NOT NULL,
  target jsonb NOT NULL DEFAULT '{}',
  value text,
  UNIQUE (case_id, ordinal)
);

CREATE TABLE test.test_data_set (
  id uuid PRIMARY KEY,
  project_id uuid NOT NULL,
  case_id uuid NOT NULL REFERENCES test.test_case(id) ON DELETE CASCADE,
  label text NOT NULL,
  kind text NOT NULL CHECK (kind IN ('VALID','INVALID','BOUNDARY')),
  rule_id uuid
);

CREATE TABLE test.test_data_row (
  id uuid PRIMARY KEY,
  project_id uuid NOT NULL,
  data_set_id uuid NOT NULL REFERENCES test.test_data_set(id) ON DELETE CASCADE,
  field_ref text NOT NULL,
  value text NOT NULL
);

CREATE TABLE test.test_run (
  id uuid PRIMARY KEY,
  project_id uuid NOT NULL,
  plan_id uuid,
  name text NOT NULL,
  environment jsonb NOT NULL,
  environment_id text NOT NULL,
  -- READY は「実行準備完了・Runner 待ち」。Runner 未接続の縦断では
  -- ここで止まり、PASSED を捏造しない（実行結果は :import で取り込む）。
  status text NOT NULL CHECK (status IN
    ('QUEUED','RUNNING','READY','DONE','FAILED','CANCELLED')),
  case_count integer NOT NULL DEFAULT 0,
  lock_acquired boolean NOT NULL DEFAULT false,
  job_id uuid,
  started_at timestamptz,
  finished_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX t6_run_project_idx ON test.test_run (project_id, created_at DESC);
-- 同じ環境で同時に進行できる run は 1 つだけ（detail/09 R-12）。
CREATE UNIQUE INDEX t6_run_env_lock_idx
  ON test.test_run (project_id, environment_id) WHERE lock_acquired;

CREATE TABLE test.test_execution (
  id uuid PRIMARY KEY,
  project_id uuid NOT NULL,
  test_run_id uuid NOT NULL REFERENCES test.test_run(id) ON DELETE CASCADE,
  test_case_id uuid NOT NULL,
  attempt smallint NOT NULL DEFAULT 1 CHECK (attempt BETWEEN 1 AND 2),
  status text NOT NULL CHECK (status IN
    ('PENDING','PASSED','FAILED','BLOCKED','SKIPPED','ERROR')),
  execution_source text NOT NULL DEFAULT 'IMPORTED'
    CHECK (execution_source IN ('IMPORTED','PLAYWRIGHT','SIMULATED')),
  started_at timestamptz,
  finished_at timestamptz,
  duration_ms integer,
  failure_kind text CHECK (failure_kind IN
    ('ASSERTION','SELECTOR_NOT_FOUND','TIMEOUT','NETWORK','DATA','ENV','RUNNER','CANCELLED')),
  failure_step integer,
  expected text,
  actual text,
  artifacts jsonb NOT NULL DEFAULT '[]',
  step_results jsonb NOT NULL DEFAULT '[]',
  fingerprint text
);
CREATE UNIQUE INDEX t6_exec_once
  ON test.test_execution (test_run_id, test_case_id, attempt);
CREATE INDEX t6_exec_run_idx ON test.test_execution (project_id, test_run_id);
CREATE INDEX t6_exec_fingerprint_idx ON test.test_execution (project_id, fingerprint);

CREATE TABLE test.defect (
  id uuid PRIMARY KEY,
  project_id uuid NOT NULL,
  execution_id uuid,
  fingerprint text NOT NULL,
  title text NOT NULL,
  severity text NOT NULL CHECK (severity IN ('LOW','MEDIUM','HIGH','CRITICAL')),
  status text NOT NULL CHECK (status IN ('CANDIDATE','OPEN','CLOSED','DISMISSED')),
  impact_item_id uuid,
  recurrence integer NOT NULL DEFAULT 1,
  detail jsonb NOT NULL DEFAULT '{}',
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX t6_defect_fingerprint_idx
  ON test.defect (project_id, fingerprint) WHERE status IN ('CANDIDATE','OPEN');
CREATE INDEX t6_defect_project_idx ON test.defect (project_id, created_at DESC);

CREATE TABLE test.coverage_snapshot (
  id uuid PRIMARY KEY,
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
CREATE UNIQUE INDEX t6_coverage_run_idx
  ON test.coverage_snapshot (project_id, test_run_id);

-- RLS: 全表。policy 名は他 schema と衝突しないよう t6_ を付ける。
ALTER TABLE test.validation_rule ENABLE ROW LEVEL SECURITY;
CREATE POLICY t6_validation_rule_scope ON test.validation_rule
  FOR ALL USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

ALTER TABLE test.test_plan ENABLE ROW LEVEL SECURITY;
CREATE POLICY t6_test_plan_scope ON test.test_plan
  FOR ALL USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

ALTER TABLE test.test_scenario ENABLE ROW LEVEL SECURITY;
CREATE POLICY t6_test_scenario_scope ON test.test_scenario
  FOR ALL USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

ALTER TABLE test.test_case ENABLE ROW LEVEL SECURITY;
CREATE POLICY t6_test_case_scope ON test.test_case
  FOR ALL USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

ALTER TABLE test.test_step ENABLE ROW LEVEL SECURITY;
CREATE POLICY t6_test_step_scope ON test.test_step
  FOR ALL USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

ALTER TABLE test.test_data_set ENABLE ROW LEVEL SECURITY;
CREATE POLICY t6_test_data_set_scope ON test.test_data_set
  FOR ALL USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

ALTER TABLE test.test_data_row ENABLE ROW LEVEL SECURITY;
CREATE POLICY t6_test_data_row_scope ON test.test_data_row
  FOR ALL USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

ALTER TABLE test.test_run ENABLE ROW LEVEL SECURITY;
CREATE POLICY t6_test_run_scope ON test.test_run
  FOR ALL USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

ALTER TABLE test.test_execution ENABLE ROW LEVEL SECURITY;
CREATE POLICY t6_test_execution_scope ON test.test_execution
  FOR ALL USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

ALTER TABLE test.defect ENABLE ROW LEVEL SECURITY;
CREATE POLICY t6_defect_scope ON test.defect
  FOR ALL USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

ALTER TABLE test.coverage_snapshot ENABLE ROW LEVEL SECURITY;
CREATE POLICY t6_coverage_snapshot_scope ON test.coverage_snapshot
  FOR ALL USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

-- core.project_of は CREATE OR REPLACE で関数全体を置き換える。
-- 014 の 10 分支を必ず残す（漏れると既存リソース API が静かに 404 になる）。
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
    END
  $$;
