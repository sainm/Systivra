use contracts::{
    ErrorCode, JobErrorItem, JobEventPayload, JobLinks, JobProgress, JobResponse, JobStatus,
};
use domain::{JobId, Permission, ProjectId};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::Authenticated;
use crate::error::AppError;
use crate::App;

impl App {
    pub async fn job_for_user(
        &self,
        auth: &Authenticated,
        job_id: JobId,
    ) -> Result<JobResponse, AppError> {
        let (project, role) = self
            .scope_for_resource(auth.user_id, "job", job_id.0)
            .await?;
        if !Permission::DocumentIngest.access_for(role).can_read() {
            return Err(AppError::NotFound(ErrorCode::NotFoundJob));
        }
        let mut tx = self.store().begin_scoped(project).await?;
        let job = self
            .store()
            .find_job(&mut tx, job_id.0)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFoundJob))?;
        store::commit(tx).await?;
        Ok(job_response(&job))
    }

    pub async fn job_events_since(
        &self,
        auth: &Authenticated,
        job_id: JobId,
        after_id: i64,
        limit: i64,
    ) -> Result<Vec<JobEventPayload>, AppError> {
        let (project, role) = self
            .scope_for_resource(auth.user_id, "job", job_id.0)
            .await?;
        if !Permission::DocumentIngest.access_for(role).can_read() {
            return Err(AppError::NotFound(ErrorCode::NotFoundJob));
        }
        let mut tx = self.store().begin_scoped(project).await?;
        let events = self
            .store()
            .events_since(&mut tx, job_id.0, after_id, limit.clamp(1, 500))
            .await?;
        store::commit(tx).await?;
        Ok(events
            .into_iter()
            .map(|e| JobEventPayload {
                id: e.id,
                event: e.event,
                data: e.data,
            })
            .collect())
    }

    pub async fn job_last_event_id(
        &self,
        auth: &Authenticated,
        job_id: JobId,
    ) -> Result<i64, AppError> {
        let (project, role) = self
            .scope_for_resource(auth.user_id, "job", job_id.0)
            .await?;
        if !Permission::DocumentIngest.access_for(role).can_read() {
            return Err(AppError::NotFound(ErrorCode::NotFoundJob));
        }
        let mut tx = self.store().begin_scoped(project).await?;
        let id = self.store().last_event_id(&mut tx, job_id.0).await?;
        store::commit(tx).await?;
        Ok(id)
    }

    pub async fn claim_next_job(&self) -> Result<Option<store::JobRow>, AppError> {
        let mut tx = self.store().begin_unscoped().await?;
        let job = self.store().claim_job(&mut tx, "systivra-worker").await?;
        store::commit(tx).await?;
        Ok(job)
    }

    pub async fn requeue_stalled_jobs(&self, stale_seconds: i64) -> Result<(u64, u64), AppError> {
        let mut tx = self.store().begin_unscoped().await?;
        let counts = self.store().requeue_stalled(&mut tx, stale_seconds).await?;
        store::commit(tx).await?;
        Ok(counts)
    }

    pub async fn mark_job_progress(
        &self,
        project: ProjectId,
        job_id: Uuid,
        phase: &str,
        files_done: i64,
        files_total: i64,
        chunks_done: i64,
    ) -> Result<i64, AppError> {
        let progress = json!({
            "phase": phase,
            "files_done": files_done,
            "files_total": files_total,
            "cells_done": 0,
            "cells_total": 0,
            "chunks_done": chunks_done,
            "done": files_done,
            "total": files_total,
        });
        let mut tx = self.store().begin_scoped(project).await?;
        let event_id = self
            .store()
            .update_progress(&mut tx, job_id, &progress, "progress")
            .await?;
        store::commit(tx).await?;
        Ok(event_id)
    }

    pub async fn heartbeat_job(&self, project: ProjectId, job_id: Uuid) -> Result<(), AppError> {
        let mut tx = self.store().begin_scoped(project).await?;
        self.store().heartbeat(&mut tx, job_id).await?;
        store::commit(tx).await?;
        Ok(())
    }

    pub async fn append_job_event(
        &self,
        project: ProjectId,
        job_id: Uuid,
        event: &str,
        data: Value,
    ) -> Result<i64, AppError> {
        let mut tx = self.store().begin_scoped(project).await?;
        let event_id = self
            .store()
            .append_event(&mut tx, job_id, event, &data)
            .await?;
        store::commit(tx).await?;
        Ok(event_id)
    }

    pub async fn finish_job(
        &self,
        project: ProjectId,
        job_id: Uuid,
        status: JobStatus,
        errors: Vec<JobErrorItem>,
        progress: Value,
    ) -> Result<(), AppError> {
        let error_value = if errors.is_empty() {
            None
        } else {
            Some(serde_json::to_value(&errors).unwrap_or(Value::Null))
        };
        let event = if status.is_terminal() && status != JobStatus::Failed {
            "done"
        } else {
            "error"
        };
        let event_data = json!({
            "status": status.as_str(),
            "progress": progress,
            "errors": errors,
        });
        let mut tx = self.store().begin_scoped(project).await?;
        self.store()
            .finish_job(
                &mut tx,
                job_id,
                &store::JobCompletion {
                    status: status.as_str(),
                    error: error_value.as_ref(),
                    progress: &progress,
                    event,
                    event_data: &event_data,
                },
            )
            .await?;
        store::commit(tx).await?;
        Ok(())
    }

    pub async fn save_checkpoint(
        &self,
        project: ProjectId,
        job_id: Uuid,
        seq: i32,
        cursor: &Value,
    ) -> Result<(), AppError> {
        let mut tx = self.store().begin_scoped(project).await?;
        self.store()
            .put_checkpoint(&mut tx, job_id, seq, cursor)
            .await?;
        store::commit(tx).await?;
        Ok(())
    }

    pub async fn load_checkpoint(
        &self,
        project: ProjectId,
        job_id: Uuid,
    ) -> Result<Option<(i32, Value)>, AppError> {
        let mut tx = self.store().begin_scoped(project).await?;
        let checkpoint = self.store().last_checkpoint(&mut tx, job_id).await?;
        store::commit(tx).await?;
        Ok(checkpoint)
    }

    pub async fn append_system_audit(
        &self,
        project: ProjectId,
        action: &str,
        target_type: &str,
        target_id: &str,
        detail: Value,
    ) -> Result<(), AppError> {
        let mut tx = self.store().begin_scoped(project).await?;
        self.store()
            .append_audit(
                &mut tx,
                &store::AuditDraft {
                    project_id: Some(project.0),
                    actor_type: "SYSTEM".into(),
                    actor_id: "systivra-worker".into(),
                    action: action.into(),
                    target_type: Some(target_type.into()),
                    target_id: Some(target_id.into()),
                    input_digest: None,
                    result: "SUCCESS".into(),
                    detail,
                    trace_id: None,
                },
            )
            .await?;
        store::commit(tx).await?;
        Ok(())
    }
}

pub fn job_response(job: &store::JobRow) -> JobResponse {
    JobResponse {
        job_id: JobId(job.id),
        kind: job.kind.clone(),
        status: JobStatus::parse(&job.status).unwrap_or(JobStatus::Queued),
        progress: progress_from_value(&job.progress),
        started_at: job.started_at,
        eta_seconds: None,
        errors: errors_from_value(job.error.as_ref()),
        links: JobLinks {
            events: format!("/api/v1/jobs/{}/events", job.id),
        },
    }
}

pub fn progress_from_value(value: &Value) -> JobProgress {
    let phase = value
        .get("phase")
        .and_then(Value::as_str)
        .unwrap_or("QUEUED")
        .to_string();
    let files_done = value
        .get("files_done")
        .and_then(Value::as_i64)
        .or_else(|| value.get("done").and_then(Value::as_i64))
        .unwrap_or(0);
    let files_total = value
        .get("files_total")
        .and_then(Value::as_i64)
        .or_else(|| value.get("total").and_then(Value::as_i64))
        .unwrap_or(0);
    JobProgress {
        phase,
        files_done,
        files_total,
        cells_done: value.get("cells_done").and_then(Value::as_i64).unwrap_or(0),
        cells_total: value
            .get("cells_total")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        chunks_done: value
            .get("chunks_done")
            .and_then(Value::as_i64)
            .unwrap_or(0),
    }
}

fn errors_from_value(value: Option<&Value>) -> Vec<JobErrorItem> {
    let Some(value) = value else {
        return Vec::new();
    };
    serde_json::from_value::<Vec<JobErrorItem>>(value.clone()).unwrap_or_default()
}
