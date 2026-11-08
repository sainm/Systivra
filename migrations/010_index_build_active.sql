ALTER TABLE corpus.index_build
  ADD COLUMN is_active boolean NOT NULL DEFAULT false;

CREATE UNIQUE INDEX index_build_active_idx
  ON corpus.index_build (project_id)
  WHERE is_active;

INSERT INTO corpus.embedding_model
  (id, name, dim, distance, provider, revision)
VALUES
  ('00000000-0000-4000-8000-000000000002', 'local-hash-1024-v2', 1024, 'COSINE', 'local', '2')
ON CONFLICT (id) DO NOTHING;
