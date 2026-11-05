-- roles.sql
-- 正本: docs/design/ja/detail/02-database-ddl.md §0.1
-- マイグレーションの前でも後でもよい。冪等。存在する schema / 表にだけ適用する
-- （kb / code / graph_meta / test / ai / ops は Phase 2 以降に増えるため）。

\set ON_ERROR_STOP on

-- ─────────────────────────────────────────────────────────────
-- 1. ロール
-- ─────────────────────────────────────────────────────────────

DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'systivra_app') THEN
    CREATE ROLE systivra_app LOGIN;
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'systivra_worker') THEN
    CREATE ROLE systivra_worker LOGIN;
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'systivra_scheduler') THEN
    -- 全プロジェクト横断でジョブを取り出すためだけのロール。
    CREATE ROLE systivra_scheduler LOGIN BYPASSRLS;
  END IF;
END
$$;

-- ─────────────────────────────────────────────────────────────
-- 2. schema と既存表への権限（存在するものだけ）
-- ─────────────────────────────────────────────────────────────

DO $$
DECLARE
  s text;
BEGIN
  FOREACH s IN ARRAY ARRAY['core','kb','code','graph_meta','test','ai','ops'] LOOP
    CONTINUE WHEN to_regnamespace(s) IS NULL;

    EXECUTE format('GRANT USAGE ON SCHEMA %I TO systivra_app, systivra_worker, systivra_scheduler', s);

    -- app: 読み書き。ただし core.job の UPDATE は後で剥がす。
    EXECUTE format(
      'GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA %I TO systivra_app', s);
    EXECUTE format(
      'GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA %I TO systivra_app', s);

    -- 今後この schema に増える表にも同じ既定を効かせる（付け忘れ防止）。
    EXECUTE format(
      'ALTER DEFAULT PRIVILEGES IN SCHEMA %I GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO systivra_app', s);
  END LOOP;
END
$$;

-- ─────────────────────────────────────────────────────────────
-- 3. 役割ごとの調整
-- ─────────────────────────────────────────────────────────────

-- app はジョブ行を書き換えない（取り出しは scheduler / 進行は worker）。
DO $$
BEGIN
  IF to_regclass('core.job') IS NOT NULL THEN
    REVOKE UPDATE ON core.job FROM systivra_app;

    GRANT SELECT ON core.job TO systivra_worker, systivra_scheduler;
    GRANT UPDATE ON core.job TO systivra_worker, systivra_scheduler;
    GRANT SELECT, INSERT, UPDATE ON core.job_checkpoint,
          core.outbox_event, core.outbox_dead_letter TO systivra_worker;
  END IF;
END
$$;

-- 監査ログは追記専用。誰にも UPDATE / DELETE を渡さない。
DO $$
BEGIN
  IF to_regclass('core.audit_log') IS NOT NULL THEN
    EXECUTE 'REVOKE UPDATE, DELETE, TRUNCATE ON core.audit_log FROM systivra_app, systivra_worker, systivra_scheduler';
  END IF;
END
$$;
