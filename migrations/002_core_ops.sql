-- 002_core_ops.sql
-- 正本: docs/design/ja/detail/02-database-ddl.md §1、順序は §7
-- 対象: job / job_checkpoint / outbox_event / outbox_dead_letter / audit_log / secret
-- forward-only。

-- ─────────────────────────────────────────────────────────────
-- ジョブキュー
--   取り出しは SKIP LOCKED。進捗は progress に持ち、正しさの根拠にしない。
-- ─────────────────────────────────────────────────────────────

CREATE TABLE core.job (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid NOT NULL,
  kind text NOT NULL,
  priority smallint NOT NULL DEFAULT 5,
  status text NOT NULL CHECK (status IN
    ('PENDING','QUEUED','RUNNING','WAITING_APPROVAL','SUCCEEDED','FAILED','CANCELLED','TIMEOUT')),
  idempotency_key text NOT NULL,
  input jsonb NOT NULL,
  progress jsonb NOT NULL DEFAULT '{"done":0,"total":0,"phase":null}',
  attempt smallint NOT NULL DEFAULT 0,
  max_attempts smallint NOT NULL DEFAULT 3,
  locked_by text,
  heartbeat_at timestamptz,
  error jsonb,
  created_at timestamptz NOT NULL DEFAULT now(),
  started_at timestamptz,
  finished_at timestamptz,
  UNIQUE (project_id, idempotency_key)
);

-- 取り出し用。待ち行列にあるものだけを部分索引にする（終了済みが増えても遅くならない）。
CREATE INDEX job_claim_idx ON core.job (status, priority DESC, created_at)
  WHERE status IN ('PENDING','QUEUED');

-- heartbeat 監視用。RUNNING のまま固まったジョブを回収する。
CREATE INDEX job_running_idx ON core.job (heartbeat_at)
  WHERE status = 'RUNNING';

CREATE TABLE core.job_checkpoint (
  job_id uuid NOT NULL REFERENCES core.job(id) ON DELETE CASCADE,
  seq integer NOT NULL,
  cursor jsonb NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (job_id, seq)
);

-- ─────────────────────────────────────────────────────────────
-- Outbox（PG → Neo4j 投影の唯一の経路。detail/07）
--   event_id が冪等キー。seq は進捗表示用で、正しさの根拠にはしない。
-- ─────────────────────────────────────────────────────────────

CREATE TABLE core.outbox_event (
  event_id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  aggregate text NOT NULL,
  aggregate_id uuid NOT NULL,
  seq bigint GENERATED ALWAYS AS IDENTITY NOT NULL,
  event_type text NOT NULL,
  payload jsonb NOT NULL,
  occurred_at timestamptz NOT NULL DEFAULT now(),
  published_at timestamptz,
  attempts smallint NOT NULL DEFAULT 0,
  last_error jsonb,
  dead_letter boolean NOT NULL DEFAULT false,
  UNIQUE (seq)
);

CREATE INDEX outbox_pending_idx ON core.outbox_event (seq)
  WHERE published_at IS NULL AND NOT dead_letter;

-- 部分失敗の退避。1 件の失敗でバッチを止めない（02-アーキテクチャ §4.3）。
CREATE TABLE core.outbox_dead_letter (
  event_id uuid PRIMARY KEY REFERENCES core.outbox_event(event_id),
  failed_at timestamptz NOT NULL DEFAULT now(),
  attempts smallint NOT NULL,
  error jsonb NOT NULL,
  resolved_at timestamptz
);

-- ─────────────────────────────────────────────────────────────
-- 監査ログ
--   追記専用。UPDATE / DELETE はアプリロールから REVOKE する。
--   月次 RANGE 分区。主キーに分区キーを含める必要があるため (id, occurred_at)。
-- ─────────────────────────────────────────────────────────────

CREATE TABLE core.audit_log (
  id bigserial,
  project_id uuid,
  actor_type text NOT NULL CHECK (actor_type IN ('USER','AGENT','SYSTEM')),
  actor_id text NOT NULL,
  action text NOT NULL,
  target_type text,
  target_id text,
  input_digest bytea,
  result text NOT NULL,
  detail jsonb NOT NULL DEFAULT '{}',
  trace_id text,
  occurred_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (id, occurred_at)
) PARTITION BY RANGE (occurred_at);

-- 親に張った索引は各分区へ自動で複製される。
CREATE INDEX audit_log_project_idx ON core.audit_log (project_id, occurred_at DESC);
CREATE INDEX audit_log_actor_idx   ON core.audit_log (actor_id, occurred_at DESC);
CREATE INDEX audit_log_target_idx  ON core.audit_log (target_type, target_id);

-- 24 か月先まで作る。以降は運用ジョブが毎月追加する（detail/02 §8）。
-- DEFAULT 分区は作らない：後から正規の分区を足すときに行が残っていると
-- ADD PARTITION が失敗するため。分区が無い月への INSERT は失敗するので、
-- 監査ログの取りこぼしは「静かに消える」ではなく「大きな音で落ちる」。
DO $$
DECLARE
  base date := date_trunc('month', now())::date;
  i    integer;
  lo   date;
  hi   date;
BEGIN
  FOR i IN 0..23 LOOP
    lo := (base + make_interval(months => i))::date;
    hi := (base + make_interval(months => i + 1))::date;
    EXECUTE format(
      'CREATE TABLE core.audit_log_%s PARTITION OF core.audit_log FOR VALUES FROM (%L) TO (%L)',
      to_char(lo, 'YYYYMM'), lo, hi
    );
  END LOOP;
END
$$;

-- 追記専用を権限で担保する。ロール名は環境で異なるため、存在するときだけ適用する。
-- シーケンスは USAGE / SELECT / UPDATE しか権限を持たず、追記を妨げないので触らない
-- （REVOKE すると警告が出るだけで意味がない）。
DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'systivra_app') THEN
    REVOKE UPDATE, DELETE, TRUNCATE ON core.audit_log FROM systivra_app;
  END IF;
END
$$;

-- ─────────────────────────────────────────────────────────────
-- 秘匿情報（Provider の API キー等）。平文を置かない。
-- ─────────────────────────────────────────────────────────────

CREATE TABLE core.secret (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  name text NOT NULL UNIQUE,
  ciphertext bytea NOT NULL,
  key_version integer NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);

-- ─────────────────────────────────────────────────────────────
-- RLS
-- ─────────────────────────────────────────────────────────────

ALTER TABLE core.job ENABLE ROW LEVEL SECURITY;
CREATE POLICY job_scope ON core.job
  USING (project_id = core.current_project());

-- job_checkpoint は project_id を持たない。job 経由で辿る。
ALTER TABLE core.job_checkpoint ENABLE ROW LEVEL SECURITY;
CREATE POLICY job_checkpoint_scope ON core.job_checkpoint
  USING (EXISTS (SELECT 1 FROM core.job j WHERE j.id = job_id));

-- audit_log / outbox_* / secret はサーバー側の運用経路だけが触る。
-- RLS ではなく権限で絞る（アプリロールに GRANT しない）。
