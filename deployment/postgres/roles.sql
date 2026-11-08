-- roles.sql
-- 正本: docs/design/ja/detail/02-database-ddl.md §0.1
-- **マイグレーションの後に流す。** 先に流すと schema が無く、USAGE を付与できず
-- 「permission denied for schema core」で全 API が 500 になる。
-- 冪等なので、マイグレーション追加のたびに再実行してよい。
-- 存在する schema / 表にだけ適用する（corpus / code / graph_meta / test / ai / ops は
-- Phase 2 以降に増えるため）。

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
    -- **BYPASSRLS は付けない。** Azure Database for PostgreSQL では PG16 未満、
    -- admin（azure_pg_admin）が非管理者に BYPASSRLS を付与できない。RDS /
    -- Cloud SQL も制約が違う。横断閲覧は下の `scheduler_scope` ポリシーで
    -- 表現するので、属性に頼ると移植できなくなる。
    CREATE ROLE systivra_scheduler LOGIN;
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
  FOREACH s IN ARRAY ARRAY['core','corpus','code','graph_meta','impact','test','ai','ops'] LOOP
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

-- impact は API が Draft を起票し、worker が結果を書く。
DO $$
BEGIN
  IF to_regnamespace('impact') IS NOT NULL THEN
    EXECUTE 'GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA impact TO systivra_worker, systivra_scheduler';
    EXECUTE 'ALTER DEFAULT PRIVILEGES IN SCHEMA impact GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO systivra_worker, systivra_scheduler';
  END IF;
END
$$;

-- test も impact と同じく API が起票し、worker が run / execution を更新する。
DO $$
BEGIN
  IF to_regnamespace('test') IS NOT NULL THEN
    EXECUTE 'GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA test TO systivra_worker, systivra_scheduler';
    EXECUTE 'ALTER DEFAULT PRIVILEGES IN SCHEMA test GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO systivra_worker, systivra_scheduler';
  END IF;
END
$$;

-- ai は API が起票し、worker が run / step / node_run を更新する。
DO $$
BEGIN
  IF to_regnamespace('ai') IS NOT NULL THEN
    EXECUTE 'GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA ai TO systivra_worker, systivra_scheduler';
    EXECUTE 'ALTER DEFAULT PRIVILEGES IN SCHEMA ai GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO systivra_worker, systivra_scheduler';
  END IF;
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

    GRANT SELECT, INSERT, UPDATE ON core.job TO systivra_worker, systivra_scheduler;
    GRANT SELECT, INSERT, UPDATE ON core.job_checkpoint,
          core.outbox_event, core.outbox_dead_letter TO systivra_worker, systivra_scheduler;
    GRANT SELECT, INSERT ON core.outbox_event TO systivra_app;
  END IF;
END
$$;

-- 期限切れセッションの掃除はプロジェクト横断（core.session は RLS 対象外）。
DO $$
BEGIN
  IF to_regclass('core.session') IS NOT NULL THEN
    GRANT SELECT, UPDATE, DELETE ON core.session TO systivra_scheduler;
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

-- corpus は worker も書く（解析結果・chunk・embedding・証拠）。
DO $$
BEGIN
  IF to_regnamespace('corpus') IS NOT NULL THEN
    EXECUTE 'GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA corpus TO systivra_worker, systivra_scheduler';
    EXECUTE 'GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA corpus TO systivra_worker, systivra_scheduler';
    EXECUTE 'ALTER DEFAULT PRIVILEGES IN SCHEMA corpus GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO systivra_worker, systivra_scheduler';
  END IF;
END
$$;

-- graph_meta は worker / scheduler も投影と再構築で書く。
DO $$
BEGIN
  IF to_regnamespace('graph_meta') IS NOT NULL THEN
    EXECUTE 'GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA graph_meta TO systivra_worker, systivra_scheduler';
    EXECUTE 'GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA graph_meta TO systivra_worker, systivra_scheduler';
    EXECUTE 'ALTER DEFAULT PRIVILEGES IN SCHEMA graph_meta GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO systivra_worker, systivra_scheduler';
  END IF;
END
$$;

-- code の内容表と分析表を worker / scheduler が書く。API はユースケースから
-- repository / analysis_run の投入だけを行い、Fact の唯一の書き手は worker。
DO $$
BEGIN
  IF to_regnamespace('code') IS NOT NULL THEN
    EXECUTE 'GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA code TO systivra_worker, systivra_scheduler';
    EXECUTE 'ALTER DEFAULT PRIVILEGES IN SCHEMA code GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO systivra_worker, systivra_scheduler';
  END IF;
END
$$;

-- 008/009 で作る関数。マイグレーション時点ではロールが無いため、ここで付与する。
DO $$
BEGIN
  IF to_regprocedure('core.project_of(text, uuid)') IS NOT NULL THEN
    GRANT EXECUTE ON FUNCTION core.project_of(text, uuid)
      TO systivra_app, systivra_worker, systivra_scheduler;
  END IF;
END
$$;

-- 監査の追記とジョブイベント。
DO $$
BEGIN
  IF to_regclass('core.audit_log') IS NOT NULL THEN
    GRANT INSERT ON core.audit_log TO systivra_worker, systivra_scheduler;
    GRANT USAGE, SELECT ON SEQUENCE core.audit_log_id_seq TO systivra_worker, systivra_scheduler;
  END IF;

  IF to_regclass('core.job_event') IS NOT NULL THEN
    GRANT SELECT, INSERT ON core.job_event TO systivra_app;
    GRANT SELECT, INSERT ON core.job_event TO systivra_worker, systivra_scheduler;
    GRANT USAGE, SELECT ON SEQUENCE core.job_event_id_seq
      TO systivra_app, systivra_worker, systivra_scheduler;
  END IF;
END
$$;

-- worker は job を起動した人間のロールを引いてから能力を実行する
-- （Agent が人間の権限を超えないための確認）。**読み取りのみ**。
DO $$
BEGIN
  IF to_regclass('core.project_member') IS NOT NULL THEN
    GRANT SELECT ON core.project_member TO systivra_worker, systivra_scheduler;
  END IF;
  IF to_regclass('core.project') IS NOT NULL THEN
    GRANT SELECT ON core.project TO systivra_scheduler;
  END IF;
  -- 既に BYPASSRLS で作られた環境（旧 roles.sql）もここで正規化する。
  -- 属性は CREATE 時にしか扱わないと、作り直し不可なので残り続ける。
  IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'systivra_scheduler' AND rolbypassrls) THEN
    ALTER ROLE systivra_scheduler NOBYPASSRLS;
  END IF;
END
$$;

-- ─────────────────────────────────────────────────────────────
-- 4. scheduler の横断閲覧を RLS の方針として書く
-- ─────────────────────────────────────────────────────────────
--
-- BYPASSRLS の代替。**許可リスト**で「このロールだけ全行を見られる」表を並べる。
-- 追加のたびにここへ書く（権限の一覧が 1 か所に纏まるように）。
--
-- 対象:
--   core.job              claim / requeue / heartbeat（project_id 無しで取り出す）
--   core.project          drift 監視のプロジェクト一覧
--   ai.approval_request   期限切れ承認の一括 close
--   test.test_execution   artifact の保持期限掃き（project_id 無し）
--
-- 方針は権限を**付けない**。RLS はフィルタしかできず、GRANT は上の 2 / 3 が管轄。
-- つまり `FOR ALL USING (true)` を書いても、元々の GRANT 以上は触れない。
DO $$
DECLARE
  target text;
BEGIN
  FOREACH target IN ARRAY ARRAY[
    'core.job', 'core.project', 'ai.approval_request', 'test.test_execution'
  ] LOOP
    CONTINUE WHEN to_regclass(target) IS NULL;
    EXECUTE format('DROP POLICY IF EXISTS scheduler_scope ON %s', target);
    EXECUTE format(
      'CREATE POLICY scheduler_scope ON %s '
      'FOR ALL TO systivra_scheduler USING (true) WITH CHECK (true)', target);
  END LOOP;
END
$$;
