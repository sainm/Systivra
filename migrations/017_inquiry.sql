-- 017_inquiry.sql
-- Phase 5 残: Q-01 変更影響問合せ（detail/08-change-inquiry.md）。
--
-- 設計との差分:
--   1. 自然言語のシード解決は RAG + entity_search の代わりに、グラフ実体名との
--      決定的な部分一致で行う（AI を使わない）。曖昧なら NEEDS_CLARIFICATION。
--   2. DIFF 入力は未対応（ENTITY / NATURAL_LANGUAGE のみ）。API で 400 を返す。
--   3. 5 セクションは impact.assessment_draft.report から投影し、本表に保存する。
--      新しい走査はしない（detail/08 §5）。

CREATE TABLE impact.change_inquiry (
  id uuid PRIMARY KEY,
  project_id uuid NOT NULL,
  question text NOT NULL,
  input_kind text NOT NULL CHECK (input_kind IN ('NATURAL_LANGUAGE','ENTITY','DIFF')),
  status text NOT NULL CHECK (status IN
    ('QUEUED','RUNNING','NEEDS_CLARIFICATION','SUCCEEDED','FAILED')),
  impact_id uuid,
  report jsonb,
  candidates jsonb NOT NULL DEFAULT '[]',
  error jsonb,
  created_at timestamptz NOT NULL DEFAULT now(),
  finished_at timestamptz
);

CREATE INDEX t5_inquiry_project_idx
  ON impact.change_inquiry (project_id, created_at DESC);

ALTER TABLE impact.change_inquiry ENABLE ROW LEVEL SECURITY;
CREATE POLICY t5_inquiry_scope ON impact.change_inquiry
  FOR ALL USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

-- 015 / 016 の 19 分支を保持し、change_inquiry を追加する。
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
      WHEN 'change_inquiry' THEN (
        SELECT project_id FROM impact.change_inquiry WHERE id = p_id
      )
    END
  $$;
