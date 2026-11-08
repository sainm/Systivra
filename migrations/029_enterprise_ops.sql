-- 029: Enterprise 残件 — 共有レート制限 / SCIM ロールの由来 / グループ解除の降格。
-- 対象: core.rate_limit_bucket、core.project_member.role_source。

CREATE TABLE core.rate_limit_bucket (
  bucket_key text PRIMARY KEY,
  window_start timestamptz NOT NULL,
  count integer NOT NULL
);

DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'systivra_app') THEN
    EXECUTE 'GRANT SELECT, INSERT, UPDATE ON core.rate_limit_bucket TO systivra_app';
  END IF;
END $$;

ALTER TABLE core.project_member
  ADD COLUMN role_source text NOT NULL DEFAULT 'MANUAL'
  CHECK (role_source IN ('MANUAL', 'SCIM'));