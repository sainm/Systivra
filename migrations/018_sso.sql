-- 018_sso.sql
-- Phase 8: OIDC SSO の外部 identity 紐付け。
--
-- app_user は認証前に引く表なので RLS を掛けない。external_identity も同じ。
-- 紐付けは管理者が事前に行い、未知の subject は自動プロビジョニングしない。

CREATE TABLE core.external_identity (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id uuid NOT NULL REFERENCES core.app_user(id) ON DELETE CASCADE,
  provider text NOT NULL,
  subject text NOT NULL,
  email text,
  created_at timestamptz NOT NULL DEFAULT now(),
  last_login_at timestamptz,
  UNIQUE (provider, subject),
  UNIQUE (user_id, provider)
);

CREATE INDEX external_identity_user_idx ON core.external_identity (user_id);

DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'systivra_app') THEN
    GRANT SELECT, INSERT, UPDATE, DELETE ON core.external_identity TO systivra_app;
  END IF;
  IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'systivra_worker') THEN
    GRANT SELECT ON core.external_identity TO systivra_worker;
  END IF;
END
$$;
