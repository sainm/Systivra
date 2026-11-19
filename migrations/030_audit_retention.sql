-- 030_audit_retention.sql
-- 監査ログの保持期間と、月次パーティションの維持。
--
-- なぜ DELETE ではないか:
--   002_core_ops.sql は audit_log を**追記専用**にするため、アプリ系ロールから
--   UPDATE / DELETE / TRUNCATE を剥がしている。保持期間のためにその約束を崩すと、
--   「消せない」はずの監査ログを常時消せるロールが生まれる。月次パーティションを
--   **DROP** すれば、消せるのは「丸ごと期限切れの月」だけで、当月の行には触れない。
--
-- あわせて、002 のコメントが「以降は運用ジョブが毎月追加する」と書いていた
-- 分区追加を実装する（24 か月後に INSERT が失敗する穴を塞ぐ）。

CREATE OR REPLACE FUNCTION core.maintain_audit_partitions(
  retention_months integer DEFAULT 0,
  ahead_months integer DEFAULT 12
) RETURNS jsonb
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, core
AS $$
DECLARE
  dropped text[] := ARRAY[]::text[];
  created text[] := ARRAY[]::text[];
  cutoff  integer;
  base    date := date_trunc('month', now())::date;
  part    record;
  lo      date;
  hi      date;
  name    text;
  i       integer;
BEGIN
  -- 期限切れの月を落とす。retention_months <= 0 は「消さない」。
  IF retention_months > 0 THEN
    cutoff := to_char(
      (date_trunc('month', now()) - make_interval(months => retention_months))::date,
      'YYYYMM'
    )::integer;

    FOR part IN
      SELECT c.relname
        FROM pg_class c
        JOIN pg_namespace n ON n.oid = c.relnamespace
       WHERE n.nspname = 'core'
         AND c.relkind = 'r'
         -- **名前で限定する。** 索引や制約（audit_log_202611_pkey 等）を
         -- DROP TABLE の対象にしないための防波堤。
         AND c.relname ~ '^audit_log_[0-9]{6}$'
       ORDER BY c.relname
    LOOP
      IF substring(part.relname from 11)::integer < cutoff THEN
        EXECUTE format('DROP TABLE core.%I', part.relname);
        dropped := dropped || part.relname;
      END IF;
    END LOOP;
  END IF;

  -- 先の月を足す。既にあれば何もしない。
  IF ahead_months > 0 THEN
    FOR i IN 0..(ahead_months - 1) LOOP
      lo := (base + make_interval(months => i))::date;
      hi := (base + make_interval(months => i + 1))::date;
      name := 'audit_log_' || to_char(lo, 'YYYYMM');
      IF to_regclass(format('core.%I', name)) IS NULL THEN
        EXECUTE format(
          'CREATE TABLE core.%I PARTITION OF core.audit_log FOR VALUES FROM (%L) TO (%L)',
          name, lo, hi
        );
        created := created || name;
      END IF;
    END LOOP;
  END IF;

  RETURN jsonb_build_object(
    'dropped', to_jsonb(dropped),
    'created', to_jsonb(created),
    'cutoff', cutoff
  );
END
$$;

-- 破壊的な関数なので PUBLIC からは剥がす。呼べるのは worker（scheduler）だけ。
REVOKE ALL ON FUNCTION core.maintain_audit_partitions(integer, integer) FROM PUBLIC;

DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'systivra_scheduler') THEN
    GRANT EXECUTE ON FUNCTION core.maintain_audit_partitions(integer, integer)
      TO systivra_scheduler;
  END IF;
END
$$;
