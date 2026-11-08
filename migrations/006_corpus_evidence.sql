CREATE TABLE corpus.evidence (
  id uuid PRIMARY KEY,
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
CREATE INDEX evidence_chunk_idx ON corpus.evidence (chunk_id);

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

CREATE INDEX retrieval_log_project_idx ON corpus.retrieval_log (project_id, created_at DESC);

ALTER TABLE corpus.evidence ENABLE ROW LEVEL SECURITY;
CREATE POLICY evidence_scope ON corpus.evidence
  FOR ALL
  USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

ALTER TABLE corpus.retrieval_log ENABLE ROW LEVEL SECURITY;
CREATE POLICY retrieval_log_scope ON corpus.retrieval_log
  FOR ALL
  USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());
