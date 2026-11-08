CREATE SCHEMA impact;

CREATE TABLE impact.assessment_draft (
  id uuid PRIMARY KEY,
  project_id uuid NOT NULL,
  input jsonb NOT NULL,
  status text NOT NULL CHECK (status IN
    ('PENDING','RUNNING','SUCCEEDED','PARTIAL','FAILED')),
  report jsonb,
  engine_version text,
  ontology_version text,
  created_at timestamptz NOT NULL DEFAULT now(),
  started_at timestamptz,
  finished_at timestamptz,
  error jsonb
);

CREATE INDEX assessment_draft_project_idx
  ON impact.assessment_draft (project_id, created_at DESC);

ALTER TABLE impact.assessment_draft ENABLE ROW LEVEL SECURITY;
CREATE POLICY assessment_draft_scope ON impact.assessment_draft
  FOR ALL USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

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
    END
  $$;
