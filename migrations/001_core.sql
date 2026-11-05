-- 001_core.sql
-- 正本: docs/design/ja/detail/02-database-ddl.md §1、順序は §7
-- forward-only。ロールバックは新しいマイグレーションで打ち消す。
-- sqlx が 1 マイグレーション = 1 トランザクションで実行するため、明示的な BEGIN は置かない。

CREATE SCHEMA core;

-- ─────────────────────────────────────────────────────────────
-- セッション変数の取り出し
--
-- `SET LOCAL` はコミット時に「未設定」ではなく**空文字**へ戻る。
-- そのため `current_setting('app.project_id', true)::uuid` は、同じ接続で
-- 2 回目以降のリクエストを処理したときに `''::uuid` で失敗する。
-- （接続プールでは必ず起きる。実測で踏んだ。）
-- ポリシーごとに NULLIF を書くと必ずどれか漏れるので、関数に閉じる。
-- ─────────────────────────────────────────────────────────────

CREATE FUNCTION core.current_project() RETURNS uuid
  LANGUAGE sql STABLE
  AS $$ SELECT NULLIF(current_setting('app.project_id', true), '')::uuid $$;

CREATE FUNCTION core.current_user() RETURNS uuid
  LANGUAGE sql STABLE
  AS $$ SELECT NULLIF(current_setting('app.user_id', true), '')::uuid $$;

GRANT EXECUTE ON FUNCTION core.current_project(), core.current_user() TO PUBLIC;

-- ─────────────────────────────────────────────────────────────
-- テナント境界
-- ─────────────────────────────────────────────────────────────

CREATE TABLE core.workspace (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  code text NOT NULL UNIQUE,
  name text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE core.app_user (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  login_id text NOT NULL UNIQUE,
  display_name text NOT NULL,
  email text,
  password_hash text,                 -- argon2id。SSO のみなら NULL
  is_active boolean NOT NULL DEFAULT true,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE core.project (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  workspace_id uuid NOT NULL REFERENCES core.workspace(id),
  code text NOT NULL,
  name text NOT NULL,
  is_sample boolean NOT NULL DEFAULT false,
  default_locale text NOT NULL DEFAULT 'ja',
  source_kind text NOT NULL CHECK (source_kind IN ('GIT','SVN','DIR','SNAPSHOT')),
  settings jsonb NOT NULL DEFAULT '{}',
  created_at timestamptz NOT NULL DEFAULT now(),
  deleted_at timestamptz,
  UNIQUE (workspace_id, code)
);

CREATE TABLE core.project_member (
  project_id uuid NOT NULL REFERENCES core.project(id),
  user_id uuid NOT NULL REFERENCES core.app_user(id),
  role text NOT NULL CHECK (role IN ('OWNER','ARCHITECT','DEVELOPER','TESTER','VIEWER')),
  granted_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (project_id, user_id)
);

CREATE TABLE core.api_token (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id uuid NOT NULL REFERENCES core.app_user(id),
  token_hash bytea NOT NULL UNIQUE,
  name text NOT NULL,
  scopes text[] NOT NULL DEFAULT '{}',
  expires_at timestamptz,
  revoked_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now()
);

-- ─────────────────────────────────────────────────────────────
-- RLS
--   アプリはトランザクション開始時に SET LOCAL app.project_id = '<uuid>' を
--   サーバー側コンテキストから設定する。リクエスト本文の値は使わない
--   （detail/05 §5.1、detail/04 §4.2）。
--   current_setting(..., true) は未設定なら NULL を返す。述語が NULL になり
--   0 行になる（エラーにしない）。設定漏れは「見えない」で止まる。
-- ─────────────────────────────────────────────────────────────

ALTER TABLE core.project ENABLE ROW LEVEL SECURITY;
CREATE POLICY project_scope ON core.project
  USING (id = core.current_project());

ALTER TABLE core.project_member ENABLE ROW LEVEL SECURITY;
CREATE POLICY project_member_scope ON core.project_member
  USING (project_id = core.current_project());

-- workspace / app_user / api_token はテナント横断（ログイン前に引く）。
-- ここに RLS を張ると認証が成立しない。権限はアプリ層で絞る。
