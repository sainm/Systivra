CREATE EXTENSION IF NOT EXISTS vector;

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
  PRIMARY KEY (id, project_id)
) PARTITION BY HASH (id);

DO $$
BEGIN
  FOR i IN 0..31 LOOP
    EXECUTE format(
      'CREATE TABLE corpus.chunk_p%s PARTITION OF corpus.chunk FOR VALUES WITH (MODULUS 32, REMAINDER %s)',
      lpad(i::text, 2, '0'), i
    );
  END LOOP;
END
$$;

CREATE INDEX chunk_scope_idx ON corpus.chunk (project_id, is_current);
CREATE INDEX chunk_document_idx ON corpus.chunk (document_id, revision_id);
CREATE INDEX chunk_fts_idx ON corpus.chunk USING gin (fts);

CREATE TABLE corpus.embedding_model (
  id uuid PRIMARY KEY,
  name text NOT NULL,
  dim integer NOT NULL,
  distance text NOT NULL CHECK (distance IN ('COSINE','L2','IP')),
  provider text NOT NULL,
  revision text NOT NULL,
  enabled boolean NOT NULL DEFAULT true,
  UNIQUE (name, revision)
);

INSERT INTO corpus.embedding_model (id, name, dim, distance, provider, revision)
VALUES ('00000000-0000-4000-8000-000000000001', 'local-hash-1024', 1024, 'COSINE', 'local', '1');

CREATE TABLE corpus.embedding_1024 (
  chunk_id uuid NOT NULL,
  project_id uuid NOT NULL,
  model_id uuid NOT NULL REFERENCES corpus.embedding_model(id),
  vec vector(1024) NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (chunk_id, model_id)
) PARTITION BY HASH (chunk_id);

DO $$
BEGIN
  FOR i IN 0..31 LOOP
    EXECUTE format(
      'CREATE TABLE corpus.embedding_1024_p%s PARTITION OF corpus.embedding_1024 FOR VALUES WITH (MODULUS 32, REMAINDER %s)',
      lpad(i::text, 2, '0'), i
    );
    EXECUTE format(
      'CREATE INDEX embedding_1024_p%s_hnsw ON corpus.embedding_1024_p%s USING hnsw (vec vector_cosine_ops) WITH (m = 16, ef_construction = 64)',
      lpad(i::text, 2, '0'), lpad(i::text, 2, '0')
    );
  END LOOP;
END
$$;

CREATE INDEX embedding_scope_idx ON corpus.embedding_1024 (project_id, model_id);

CREATE TABLE corpus.index_build (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid NOT NULL,
  model_id uuid NOT NULL,
  status text NOT NULL CHECK (status IN ('BUILDING','READY','FAILED','RETIRED')),
  chunk_count bigint NOT NULL DEFAULT 0,
  started_at timestamptz,
  finished_at timestamptz,
  error jsonb
);

ALTER TABLE corpus.doc_node ENABLE ROW LEVEL SECURITY;
CREATE POLICY doc_node_scope ON corpus.doc_node
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

ALTER TABLE corpus.chunk ENABLE ROW LEVEL SECURITY;
CREATE POLICY chunk_scope ON corpus.chunk
  FOR ALL
  USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

ALTER TABLE corpus.embedding_1024 ENABLE ROW LEVEL SECURITY;
CREATE POLICY embedding_scope ON corpus.embedding_1024
  FOR ALL
  USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

ALTER TABLE corpus.index_build ENABLE ROW LEVEL SECURITY;
CREATE POLICY index_build_scope ON corpus.index_build
  FOR ALL
  USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());
