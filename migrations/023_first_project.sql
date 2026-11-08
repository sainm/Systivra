-- 023_first_project.sql
-- SCIM / IdP グループ同期は、テナントスコープ確立前に「唯一のプロジェクト」を
-- 解決する必要がある。core.project は RLS 対象なので、SECURITY DEFINER の
-- 単一目的関数だけを用意する（複数プロジェクトには使わない）。

CREATE OR REPLACE FUNCTION core.first_project() RETURNS uuid
  LANGUAGE sql STABLE SECURITY DEFINER
  SET search_path = pg_catalog
  AS $$
    SELECT id FROM core.project
     WHERE deleted_at IS NULL
     ORDER BY code
     LIMIT 1
  $$;

DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'systivra_app') THEN
    GRANT EXECUTE ON FUNCTION core.first_project() TO systivra_app;
  END IF;
END
$$;
