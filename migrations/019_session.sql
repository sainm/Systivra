-- 019_session.sql
-- Phase 8: サーバー側で失効できるセッション。
--
-- 認証前・認証中に引く表なので RLS の対象外（app_user と同じ）。
-- Cookie は session id と user id を含み、HMAC で改竄を防ぐ。
-- 実体の有効期限・失効はこの表で判定する。

CREATE TABLE core.session (
  id uuid PRIMARY KEY,
  user_id uuid NOT NULL REFERENCES core.app_user(id) ON DELETE CASCADE,
  created_at timestamptz NOT NULL DEFAULT now(),
  last_seen_at timestamptz NOT NULL DEFAULT now(),
  expires_at timestamptz NOT NULL,
  revoked_at timestamptz,
  ip_hash bytea,
  user_agent text
);

CREATE INDEX session_user_idx ON core.session (user_id, created_at DESC);
CREATE INDEX session_live_idx ON core.session (expires_at) WHERE revoked_at IS NULL;

DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'systivra_app') THEN
    GRANT SELECT, INSERT, UPDATE, DELETE ON core.session TO systivra_app;
  END IF;
  IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'systivra_worker') THEN
    GRANT SELECT ON core.session TO systivra_worker;
  END IF;
END
$$;
