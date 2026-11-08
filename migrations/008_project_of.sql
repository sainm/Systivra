CREATE FUNCTION core.project_of(kind text, p_id uuid) RETURNS uuid
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
    END
  $$;

REVOKE ALL ON FUNCTION core.project_of(text, uuid) FROM PUBLIC;

DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'systivra_app') THEN
    GRANT EXECUTE ON FUNCTION core.project_of(text, uuid)
      TO systivra_app, systivra_worker, systivra_scheduler;
  END IF;
END
$$;
