-- 022_scim.sql
-- SCIM 2.0 のユーザー / グループと、IdP グループ→ロールの対応。
-- 認証・認可の前後で参照するため RLS の対象外。

CREATE TABLE core.external_group (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  provider text NOT NULL,
  external_id text NOT NULL,
  display_name text NOT NULL,
  role text NOT NULL CHECK (role IN ('OWNER','ARCHITECT','DEVELOPER','TESTER','VIEWER')),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (provider, external_id)
);

CREATE TABLE core.external_group_member (
  group_id uuid NOT NULL REFERENCES core.external_group(id) ON DELETE CASCADE,
  user_id uuid NOT NULL REFERENCES core.app_user(id) ON DELETE CASCADE,
  PRIMARY KEY (group_id, user_id)
);

CREATE INDEX external_group_provider_idx ON core.external_group (provider);
CREATE INDEX external_group_member_user_idx ON core.external_group_member (user_id);

DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'systivra_app') THEN
    GRANT SELECT, INSERT, UPDATE, DELETE ON core.external_group TO systivra_app;
    GRANT SELECT, INSERT, UPDATE, DELETE ON core.external_group_member TO systivra_app;
  END IF;
END
$$;
