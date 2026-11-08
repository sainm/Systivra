CREATE SCHEMA corpus;

CREATE TABLE corpus.document (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid NOT NULL,
  doc_type text NOT NULL CHECK (doc_type IN (
    'REQUIREMENT','BASIC_DESIGN','DETAILED_DESIGN','UI_SPEC','DB_SPEC',
    'TEST_SPEC','API_SPEC','MANUAL','OTHER')),
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
  object_key text NOT NULL,
  parse_artifact_key text,
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

CREATE INDEX revision_content_hash_idx ON corpus.document_revision (content_hash);

CREATE TABLE corpus.parse_run (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  revision_id uuid NOT NULL REFERENCES corpus.document_revision(id) ON DELETE CASCADE,
  parser text NOT NULL,
  parser_version text NOT NULL,
  status text NOT NULL CHECK (status IN ('PENDING','RUNNING','SUCCEEDED','FAILED','PARTIAL')),
  started_at timestamptz,
  finished_at timestamptz,
  stats jsonb NOT NULL DEFAULT '{}',
  error jsonb
);

CREATE INDEX parse_run_revision_idx ON corpus.parse_run (revision_id, started_at DESC);

ALTER TABLE corpus.document ENABLE ROW LEVEL SECURITY;
CREATE POLICY document_scope ON corpus.document
  FOR ALL
  USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

ALTER TABLE corpus.document_revision ENABLE ROW LEVEL SECURITY;
CREATE POLICY revision_scope ON corpus.document_revision
  FOR ALL
  USING (EXISTS (
    SELECT 1 FROM corpus.document d
     WHERE d.id = document_id AND d.project_id = core.current_project()
  ))
  WITH CHECK (EXISTS (
    SELECT 1 FROM corpus.document d
     WHERE d.id = document_id AND d.project_id = core.current_project()
  ));

ALTER TABLE corpus.parse_run ENABLE ROW LEVEL SECURITY;
CREATE POLICY parse_run_scope ON corpus.parse_run
  FOR ALL
  USING (EXISTS (
    SELECT 1
      FROM corpus.document_revision r
      JOIN corpus.document d ON d.id = r.document_id
     WHERE r.id = revision_id AND d.project_id = core.current_project()
  ))
  WITH CHECK (EXISTS (
    SELECT 1
      FROM corpus.document_revision r
      JOIN corpus.document d ON d.id = r.document_id
     WHERE r.id = revision_id AND d.project_id = core.current_project()
  ));
