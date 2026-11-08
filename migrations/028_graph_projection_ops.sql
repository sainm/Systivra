-- 028: 投影運用 — 影ラベル再構築・日次漂流・複数 projector の claim リース。
-- 対象: core.outbox_event の lease 列、graph_meta.projection_drift。

ALTER TABLE core.outbox_event
  ADD COLUMN locked_by text,
  ADD COLUMN locked_until timestamptz;

CREATE INDEX outbox_claim_idx ON core.outbox_event (seq)
  WHERE published_at IS NULL AND NOT dead_letter;

CREATE TABLE graph_meta.projection_drift (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  project_id uuid NOT NULL,
  checked_at timestamptz NOT NULL DEFAULT now(),
  pg_nodes bigint NOT NULL,
  pg_relationships bigint NOT NULL,
  neo4j_nodes bigint NOT NULL,
  neo4j_relationships bigint NOT NULL,
  drift_rate numeric(6,5) NOT NULL,
  degraded boolean NOT NULL
);

CREATE INDEX projection_drift_scope_idx
  ON graph_meta.projection_drift (project_id, checked_at DESC);

ALTER TABLE graph_meta.projection_drift ENABLE ROW LEVEL SECURITY;
CREATE POLICY projection_drift_scope ON graph_meta.projection_drift
  FOR ALL
  USING (project_id = core.current_project())
  WITH CHECK (project_id = core.current_project());

DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'systivra_worker') THEN
    EXECUTE 'GRANT SELECT, INSERT ON graph_meta.projection_drift TO systivra_worker, systivra_scheduler';
  END IF;
  IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'systivra_scheduler') THEN
    EXECUTE 'GRANT SELECT ON core.project TO systivra_scheduler';
  END IF;
  IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'systivra_app') THEN
    EXECUTE 'GRANT SELECT ON graph_meta.projection_drift TO systivra_app';
  END IF;
END $$;