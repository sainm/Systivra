-- 024_code_schema_repair.sql
-- 013 を一度適用した古い DB を、現在の 013 の列・索引に合わせる。
-- 新規 DB では no-op。

ALTER TABLE code.code_revision
  ADD COLUMN IF NOT EXISTS snapshot_key text NOT NULL DEFAULT '';
ALTER TABLE code.code_revision ALTER COLUMN snapshot_key DROP DEFAULT;

ALTER TABLE code.analysis_run
  ADD COLUMN IF NOT EXISTS created_at timestamptz NOT NULL DEFAULT now();

ALTER TABLE code.source_file
  ADD COLUMN IF NOT EXISTS exact_offsets boolean NOT NULL DEFAULT false;

ALTER TABLE code.symbol
  ADD COLUMN IF NOT EXISTS byte_start bigint NOT NULL DEFAULT 0;
ALTER TABLE code.symbol
  ADD COLUMN IF NOT EXISTS byte_end bigint NOT NULL DEFAULT 0;

CREATE UNIQUE INDEX IF NOT EXISTS repository_project_kind_url_idx
  ON code.repository (project_id, kind, url) WHERE url IS NOT NULL;

DROP INDEX IF EXISTS code.classpath_revision_artifact_idx;
CREATE UNIQUE INDEX IF NOT EXISTS classpath_revision_artifact_idx
  ON code.classpath_index (revision_id, artifact_key);

CREATE INDEX IF NOT EXISTS fact_evidence_project_idx
  ON code.fact_evidence (project_id, fact_id);
