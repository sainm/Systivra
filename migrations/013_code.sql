CREATE SCHEMA code;

CREATE TABLE code.repository (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid NOT NULL,
  kind text NOT NULL CHECK (kind IN ('GIT','SVN','DIR','SNAPSHOT')),
  url text,
  default_branch text,
  settings jsonb NOT NULL DEFAULT '{}',
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX repository_project_idx ON code.repository (project_id, created_at DESC);
CREATE UNIQUE INDEX repository_project_kind_url_idx
  ON code.repository (project_id, kind, url) WHERE url IS NOT NULL;

CREATE TABLE code.code_revision (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid NOT NULL,
  repository_id uuid NOT NULL REFERENCES code.repository(id),
  revision text NOT NULL,
  parent_id uuid REFERENCES code.code_revision(id),
  tree_digest bytea NOT NULL,
  snapshot_key text NOT NULL,
  committed_at timestamptz,
  is_baseline boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (repository_id, revision)
);
CREATE INDEX code_revision_project_idx ON code.code_revision (project_id, created_at DESC);

CREATE TABLE code.source_file (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid NOT NULL,
  revision_id uuid NOT NULL REFERENCES code.code_revision(id) ON DELETE CASCADE,
  path text NOT NULL,
  language text NOT NULL,
  content_hash bytea NOT NULL,
  size_bytes bigint NOT NULL,
  encoding text NOT NULL DEFAULT 'UTF-8',
  loc integer NOT NULL,
  line_offsets bytea,
  exact_offsets boolean NOT NULL DEFAULT false,
  UNIQUE (revision_id, path)
);
CREATE INDEX source_file_project_idx ON code.source_file (project_id, path);

CREATE TABLE code.symbol (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid NOT NULL,
  revision_id uuid NOT NULL REFERENCES code.code_revision(id) ON DELETE CASCADE,
  file_id uuid NOT NULL REFERENCES code.source_file(id) ON DELETE CASCADE,
  kind text NOT NULL CHECK (kind IN ('CLASS','METHOD','FIELD','INTERFACE','ENUM')),
  fqn text NOT NULL,
  simple_name text NOT NULL,
  signature text,
  line_start integer NOT NULL,
  line_end integer NOT NULL,
  byte_start bigint NOT NULL DEFAULT 0,
  byte_end bigint NOT NULL DEFAULT 0,
  modifiers text[] NOT NULL DEFAULT '{}',
  metadata jsonb NOT NULL DEFAULT '{}'
);
CREATE INDEX symbol_fqn_idx ON code.symbol (revision_id, fqn);
CREATE INDEX symbol_project_idx ON code.symbol (project_id, revision_id, kind);

CREATE TABLE code.analysis_run (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid NOT NULL,
  revision_id uuid NOT NULL,
  status text NOT NULL CHECK (status IN ('PENDING','RUNNING','SUCCEEDED','FAILED','PARTIAL')),
  engine_versions jsonb NOT NULL DEFAULT '{}',
  classpath_complete boolean NOT NULL DEFAULT false,
  stats jsonb NOT NULL DEFAULT '{}',
  started_at timestamptz,
  finished_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  error jsonb
);
CREATE INDEX analysis_run_project_idx ON code.analysis_run (project_id, started_at DESC);

CREATE TABLE code.classpath_index (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid NOT NULL,
  revision_id uuid NOT NULL,
  artifact_key text NOT NULL,
  jar_count integer NOT NULL DEFAULT 0,
  complete boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX classpath_revision_artifact_idx
  ON code.classpath_index (revision_id, artifact_key);

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
  superseded_by uuid REFERENCES code.fact(id) DEFERRABLE INITIALLY DEFERRED,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX fact_subject_idx ON code.fact (project_id, revision_id, subject_key, relation);
CREATE INDEX fact_object_idx  ON code.fact (project_id, revision_id, object_key, relation);
CREATE UNIQUE INDEX fact_unique_live ON code.fact
  (project_id, revision_id, subject_key, relation, object_key, analysis_engine)
  WHERE valid_to IS NULL;

CREATE TABLE code.fact_evidence (
  project_id uuid NOT NULL,
  fact_id uuid NOT NULL REFERENCES code.fact(id) ON DELETE CASCADE,
  evidence_id uuid NOT NULL REFERENCES corpus.evidence(id) ON DELETE CASCADE,
  role text NOT NULL DEFAULT 'PRIMARY',
  PRIMARY KEY (fact_id, evidence_id)
);
CREATE INDEX fact_evidence_project_idx ON code.fact_evidence (project_id, fact_id);

ALTER TABLE code.repository ENABLE ROW LEVEL SECURITY;
CREATE POLICY repository_scope ON code.repository
  FOR ALL USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

ALTER TABLE code.code_revision ENABLE ROW LEVEL SECURITY;
CREATE POLICY code_revision_scope ON code.code_revision
  FOR ALL USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

ALTER TABLE code.source_file ENABLE ROW LEVEL SECURITY;
CREATE POLICY source_file_scope ON code.source_file
  FOR ALL USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

ALTER TABLE code.symbol ENABLE ROW LEVEL SECURITY;
CREATE POLICY symbol_scope ON code.symbol
  FOR ALL USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

ALTER TABLE code.analysis_run ENABLE ROW LEVEL SECURITY;
CREATE POLICY analysis_run_scope ON code.analysis_run
  FOR ALL USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

ALTER TABLE code.classpath_index ENABLE ROW LEVEL SECURITY;
CREATE POLICY classpath_index_scope ON code.classpath_index
  FOR ALL USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

ALTER TABLE code.fact ENABLE ROW LEVEL SECURITY;
CREATE POLICY fact_scope ON code.fact
  FOR ALL USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

ALTER TABLE code.fact_evidence ENABLE ROW LEVEL SECURITY;
CREATE POLICY fact_evidence_scope ON code.fact_evidence
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
    END
  $$;
