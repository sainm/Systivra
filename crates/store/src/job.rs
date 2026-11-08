use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

use crate::pool::{Store, StoreError, Tx};

#[derive(Debug, Clone)]
pub struct JobRow {
    pub id: Uuid,
    pub project_id: Uuid,
    pub kind: String,
    pub status: String,
    pub input: Value,
    pub progress: Value,
    pub attempt: i16,
    pub max_attempts: i16,
    pub error: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewJob {
    pub project_id: Uuid,
    pub kind: String,
    pub idempotency_key: String,
    pub input: Value,
}

#[derive(Debug, Clone)]
pub struct EnqueuedJob {
    pub job: JobRow,
    pub created: bool,
}

#[derive(Debug, Clone)]
pub struct JobEvent {
    pub id: i64,
    pub event: String,
    pub data: Value,
}

pub struct JobCompletion<'a> {
    pub status: &'a str,
    pub error: Option<&'a Value>,
    pub progress: &'a Value,
    pub event: &'a str,
    pub event_data: &'a Value,
}

impl Store {
    pub async fn enqueue_job(
        &self,
        tx: &mut Tx<'_>,
        job: &NewJob,
    ) -> Result<EnqueuedJob, StoreError> {
        let inserted = sqlx::query(
            "INSERT INTO core.job (project_id, kind, idempotency_key, input, status)
             VALUES ($1, $2, $3, $4, 'QUEUED')
             ON CONFLICT (project_id, idempotency_key) DO NOTHING
             RETURNING id, project_id, kind, status, input, progress, attempt, max_attempts,
                       error, created_at, started_at, finished_at",
        )
        .bind(job.project_id)
        .bind(&job.kind)
        .bind(&job.idempotency_key)
        .bind(&job.input)
        .fetch_optional(&mut **tx)
        .await?;

        if let Some(row) = inserted {
            let created = job_row(&row);
            sqlx::query(
                "INSERT INTO core.job_event (job_id, event, data)
                 VALUES ($1, 'queued', jsonb_build_object('status', 'QUEUED'))",
            )
            .bind(created.id)
            .execute(&mut **tx)
            .await?;
            return Ok(EnqueuedJob {
                job: created,
                created: true,
            });
        }

        let row = sqlx::query(
            "SELECT id, project_id, kind, status, input, progress, attempt, max_attempts,
                    error, created_at, started_at, finished_at
               FROM core.job
              WHERE project_id = $1 AND idempotency_key = $2",
        )
        .bind(job.project_id)
        .bind(&job.idempotency_key)
        .fetch_one(&mut **tx)
        .await?;
        Ok(EnqueuedJob {
            job: job_row(&row),
            created: false,
        })
    }

    pub async fn find_job_by_idempotency(
        &self,
        tx: &mut Tx<'_>,
        project_id: Uuid,
        idempotency_key: &str,
    ) -> Result<Option<JobRow>, StoreError> {
        let row = sqlx::query(
            "SELECT id, project_id, kind, status, input, progress, attempt, max_attempts,
                    error, created_at, started_at, finished_at
               FROM core.job
              WHERE project_id = $1 AND idempotency_key = $2",
        )
        .bind(project_id)
        .bind(idempotency_key)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.as_ref().map(job_row))
    }

    pub async fn find_job(
        &self,
        tx: &mut Tx<'_>,
        job_id: Uuid,
    ) -> Result<Option<JobRow>, StoreError> {
        let row = sqlx::query(
            "SELECT id, project_id, kind, status, input, progress, attempt, max_attempts,
                    error, created_at, started_at, finished_at
               FROM core.job
              WHERE id = $1",
        )
        .bind(job_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.as_ref().map(job_row))
    }

    /// 心拍が途切れた `RUNNING` を回収する。
    ///
    /// 試行回数が残っていれば `QUEUED` に戻して再実行し、使い切っていれば
    /// **`FAILED` で止める**。止めないと、プロセスごと落とす job が
    /// 永久に再キューされ続ける（poison pill）。
    pub async fn requeue_stalled(
        &self,
        tx: &mut Tx<'_>,
        stale_seconds: i64,
    ) -> Result<(u64, u64), StoreError> {
        // 回収は**静かに状態を書き換えない**。`core.job_event` に残す。
        // これが無いと /jobs/{id}/events は「RUNNING で始まって終わる」だけで、
        // なぜ FAILED になったのか・なぜ再キューされたのかが後から追えない。
        let failed: i64 = sqlx::query(
            "WITH failed AS (
                UPDATE core.job
                   SET status = 'FAILED',
                       finished_at = now(),
                       locked_by = NULL,
                       heartbeat_at = NULL,
                       error = jsonb_build_object(
                           'code', 'WORKER_HEARTBEAT_LOST',
                           'detail', 'worker stopped heartbeating and attempts are exhausted'
                       )
                 WHERE status = 'RUNNING'
                   AND attempt >= max_attempts
                   AND COALESCE(heartbeat_at, started_at, created_at)
                       < now() - make_interval(secs => $1)
              RETURNING id, attempt, max_attempts
             ), events AS (
                INSERT INTO core.job_event (job_id, event, data)
                SELECT id, 'error', jsonb_build_object(
                    'status', 'FAILED',
                    'code', 'WORKER_HEARTBEAT_LOST',
                    'reason', 'stalled_heartbeat',
                    'attempt', attempt,
                    'max_attempts', max_attempts
                ) FROM failed
              RETURNING job_id
             )
             SELECT count(*) AS n FROM events",
        )
        .bind(stale_seconds as f64)
        .fetch_one(&mut **tx)
        .await?
        .get("n");

        let requeued: i64 = sqlx::query(
            "WITH requeued AS (
                UPDATE core.job
                   SET status = 'QUEUED', locked_by = NULL, heartbeat_at = NULL
                 WHERE status = 'RUNNING'
                   AND attempt < max_attempts
                   AND COALESCE(heartbeat_at, started_at, created_at)
                       < now() - make_interval(secs => $1)
              RETURNING id, attempt, kind
             ), events AS (
                INSERT INTO core.job_event (job_id, event, data)
                SELECT id, 'progress', jsonb_build_object(
                    'status', 'QUEUED',
                    'phase', 'RECOVERED',
                    'reason', 'stalled_heartbeat',
                    'attempt', attempt,
                    'kind', kind
                ) FROM requeued
              RETURNING job_id
             )
             SELECT count(*) AS n FROM events",
        )
        .bind(stale_seconds as f64)
        .fetch_one(&mut **tx)
        .await?
        .get("n");

        Ok((requeued as u64, failed as u64))
    }

    pub async fn claim_job(
        &self,
        tx: &mut Tx<'_>,
        worker_id: &str,
    ) -> Result<Option<JobRow>, StoreError> {
        let row = sqlx::query(
            "UPDATE core.job
                SET status = 'RUNNING',
                    started_at = COALESCE(started_at, now()),
                    heartbeat_at = now(),
                    locked_by = $1,
                    attempt = attempt + 1
              WHERE id = (
                SELECT id FROM core.job
                 WHERE status IN ('PENDING', 'QUEUED')
                 ORDER BY priority DESC, created_at
                 LIMIT 1
                 FOR UPDATE SKIP LOCKED
              )
              RETURNING id, project_id, kind, status, input, progress, attempt, max_attempts,
                        error, created_at, started_at, finished_at",
        )
        .bind(worker_id)
        .fetch_optional(&mut **tx)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };
        let job = job_row(&row);
        sqlx::query(
            "INSERT INTO core.job_event (job_id, event, data)
             VALUES ($1, 'progress', jsonb_build_object('status', 'RUNNING', 'phase', 'START'))",
        )
        .bind(job.id)
        .execute(&mut **tx)
        .await?;
        Ok(Some(job))
    }

    pub async fn heartbeat(&self, tx: &mut Tx<'_>, job_id: Uuid) -> Result<(), StoreError> {
        sqlx::query("UPDATE core.job SET heartbeat_at = now() WHERE id = $1")
            .bind(job_id)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    pub async fn update_progress(
        &self,
        tx: &mut Tx<'_>,
        job_id: Uuid,
        progress: &Value,
        event: &str,
    ) -> Result<i64, StoreError> {
        sqlx::query("UPDATE core.job SET progress = $2, heartbeat_at = now() WHERE id = $1")
            .bind(job_id)
            .bind(progress)
            .execute(&mut **tx)
            .await?;
        let row = sqlx::query(
            "INSERT INTO core.job_event (job_id, event, data)
             VALUES ($1, $2, $3)
             RETURNING id",
        )
        .bind(job_id)
        .bind(event)
        .bind(progress)
        .fetch_one(&mut **tx)
        .await?;
        Ok(row.get("id"))
    }

    pub async fn append_event(
        &self,
        tx: &mut Tx<'_>,
        job_id: Uuid,
        event: &str,
        data: &Value,
    ) -> Result<i64, StoreError> {
        let row = sqlx::query(
            "INSERT INTO core.job_event (job_id, event, data)
             VALUES ($1, $2, $3)
             RETURNING id",
        )
        .bind(job_id)
        .bind(event)
        .bind(data)
        .fetch_one(&mut **tx)
        .await?;
        Ok(row.get("id"))
    }

    pub async fn finish_job(
        &self,
        tx: &mut Tx<'_>,
        job_id: Uuid,
        completion: &JobCompletion<'_>,
    ) -> Result<i64, StoreError> {
        sqlx::query(
            "UPDATE core.job
                SET status = $2, finished_at = now(), error = $3, progress = $4
              WHERE id = $1",
        )
        .bind(job_id)
        .bind(completion.status)
        .bind(completion.error)
        .bind(completion.progress)
        .execute(&mut **tx)
        .await?;
        self.append_event(tx, job_id, completion.event, completion.event_data)
            .await
    }

    pub async fn events_since(
        &self,
        tx: &mut Tx<'_>,
        job_id: Uuid,
        after_id: i64,
        limit: i64,
    ) -> Result<Vec<JobEvent>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, event, data
               FROM core.job_event
              WHERE job_id = $1 AND id > $2
              ORDER BY id
              LIMIT $3",
        )
        .bind(job_id)
        .bind(after_id)
        .bind(limit)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| JobEvent {
                id: r.get("id"),
                event: r.get("event"),
                data: r.get("data"),
            })
            .collect())
    }

    pub async fn last_event_id(&self, tx: &mut Tx<'_>, job_id: Uuid) -> Result<i64, StoreError> {
        let row =
            sqlx::query("SELECT COALESCE(max(id), 0) AS id FROM core.job_event WHERE job_id = $1")
                .bind(job_id)
                .fetch_one(&mut **tx)
                .await?;
        Ok(row.get("id"))
    }

    pub async fn put_checkpoint(
        &self,
        tx: &mut Tx<'_>,
        job_id: Uuid,
        seq: i32,
        cursor: &Value,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO core.job_checkpoint (job_id, seq, cursor)
             VALUES ($1, $2, $3)
             ON CONFLICT (job_id, seq) DO UPDATE SET cursor = EXCLUDED.cursor",
        )
        .bind(job_id)
        .bind(seq)
        .bind(cursor)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn last_checkpoint(
        &self,
        tx: &mut Tx<'_>,
        job_id: Uuid,
    ) -> Result<Option<(i32, Value)>, StoreError> {
        let row = sqlx::query(
            "SELECT seq, cursor FROM core.job_checkpoint
              WHERE job_id = $1
              ORDER BY seq DESC
              LIMIT 1",
        )
        .bind(job_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.map(|r| (r.get("seq"), r.get("cursor"))))
    }

    pub async fn job_project(
        &self,
        tx: &mut Tx<'_>,
        job_id: Uuid,
    ) -> Result<Option<Uuid>, StoreError> {
        let row = sqlx::query("SELECT project_id FROM core.job WHERE id = $1")
            .bind(job_id)
            .fetch_optional(&mut **tx)
            .await?;
        Ok(row.map(|r| r.get("project_id")))
    }
}

fn job_row(row: &sqlx::postgres::PgRow) -> JobRow {
    JobRow {
        id: row.get("id"),
        project_id: row.get("project_id"),
        kind: row.get("kind"),
        status: row.get("status"),
        input: row.get("input"),
        progress: row.get("progress"),
        attempt: row.get("attempt"),
        max_attempts: row.get("max_attempts"),
        error: row.get("error"),
        created_at: row.get("created_at"),
        started_at: row.get("started_at"),
        finished_at: row.get("finished_at"),
    }
}
