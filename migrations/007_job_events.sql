ALTER TABLE core.job DROP CONSTRAINT IF EXISTS job_status_check;
ALTER TABLE core.job ADD CONSTRAINT job_status_check CHECK (status IN
  ('PENDING','QUEUED','RUNNING','WAITING_APPROVAL','SUCCEEDED','SUCCEEDED_WITH_ERRORS','FAILED','CANCELLED','TIMEOUT'));

CREATE TABLE core.job_event (
  id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  job_id uuid NOT NULL REFERENCES core.job(id) ON DELETE CASCADE,
  event text NOT NULL,
  data jsonb NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX job_event_job_idx ON core.job_event (job_id, id);

ALTER TABLE core.job_event ENABLE ROW LEVEL SECURITY;
CREATE POLICY job_event_scope ON core.job_event
  FOR ALL
  USING (EXISTS (SELECT 1 FROM core.job j WHERE j.id = job_id))
  WITH CHECK (EXISTS (SELECT 1 FROM core.job j WHERE j.id = job_id));
