ALTER TABLE core.job ADD COLUMN cancel_requested_at timestamptz;

ALTER TABLE corpus.parse_run DROP CONSTRAINT parse_run_status_check;
ALTER TABLE corpus.parse_run ADD CONSTRAINT parse_run_status_check
  CHECK (status IN ('PENDING','RUNNING','SUCCEEDED','FAILED','PARTIAL','CANCELLED'));

CREATE FUNCTION core.request_job_cancel(p_job_id uuid) RETURNS text
  LANGUAGE plpgsql
  SECURITY DEFINER
  SET search_path = core, pg_temp
AS $$
DECLARE
  v_status text;
  v_flag timestamptz;
  v_project uuid;
BEGIN
  SELECT status, cancel_requested_at, project_id
    INTO v_status, v_flag, v_project
    FROM core.job WHERE id = p_job_id FOR UPDATE;
  IF NOT FOUND THEN
    RETURN 'NOT_FOUND';
  END IF;
  IF v_project IS DISTINCT FROM core.current_project() THEN
    RETURN 'NOT_FOUND';
  END IF;
  IF v_status = 'CANCELLED' THEN
    RETURN 'ALREADY_CANCELLED';
  END IF;
  IF v_status IN ('PENDING', 'QUEUED') THEN
    UPDATE core.job
       SET status = 'CANCELLED',
           cancel_requested_at = COALESCE(cancel_requested_at, now()),
           finished_at = now(), locked_by = NULL, heartbeat_at = NULL,
           error = jsonb_build_array(jsonb_build_object(
             'code', 'JOB_CANCELLED', 'message', 'cancelled by user'))
     WHERE id = p_job_id;
    RETURN 'IMMEDIATE';
  END IF;
  IF v_status IN ('RUNNING', 'WAITING_APPROVAL') THEN
    IF v_flag IS NOT NULL THEN
      RETURN 'ALREADY_CANCELLED';
    END IF;
    UPDATE core.job SET cancel_requested_at = now() WHERE id = p_job_id;
    RETURN 'FLAGGED';
  END IF;
  RETURN 'TERMINAL';
END;
$$;

REVOKE EXECUTE ON FUNCTION core.request_job_cancel(uuid) FROM PUBLIC;