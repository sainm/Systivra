use contracts::{
    CreateIndexBuildRequest, CreateIndexBuildResponse, IndexBuildListResponse, IndexBuildSummary,
    JobLinks,
};
use domain::{JobId, Permission, ProjectId};
use serde_json::json;
use uuid::Uuid;

use crate::auth::Authenticated;
use crate::error::AppError;
use crate::App;

impl App {
    pub async fn create_index_build(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        req: CreateIndexBuildRequest,
        trace: Option<String>,
    ) -> Result<CreateIndexBuildResponse, AppError> {
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
        if !Permission::ProviderManage.access_for(role).can_write() {
            return Err(AppError::Forbidden("provider:manage がありません".into()));
        }

        let mut tx = self.store().begin_scoped(project).await?;
        let model = self
            .store()
            .embedding_model_by_name(&mut tx, &req.model)
            .await?
            .ok_or_else(|| AppError::Validation("embedding model がありません".into()))?;
        let build_id = self
            .store()
            .create_index_build(&mut tx, project.0, model.0)
            .await?;
        let input = json!({
            "project_id": project,
            "build_id": build_id,
            "model_id": model.0,
            "model_name": model.1,
            "revision": model.2,
        });
        let enqueued = self
            .store()
            .enqueue_job(
                &mut tx,
                &store::NewJob {
                    project_id: project.0,
                    kind: "INDEX_BUILD".into(),
                    idempotency_key: format!("index-build:{build_id}"),
                    input,
                },
            )
            .await?;
        let row = self
            .store()
            .index_build(&mut tx, build_id)
            .await?
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
        self.store()
            .append_audit(
                &mut tx,
                &store::AuditDraft {
                    project_id: Some(project.0),
                    actor_type: "USER".into(),
                    actor_id: auth.user_id.to_string(),
                    action: "index_build.create".into(),
                    target_type: Some("index_build".into()),
                    target_id: Some(build_id.to_string()),
                    input_digest: None,
                    result: "QUEUED".into(),
                    detail: json!({"model": model.1, "revision": model.2, "job_id": enqueued.job.id}),
                    trace_id: trace,
                },
            )
            .await?;
        store::commit(tx).await?;

        Ok(CreateIndexBuildResponse {
            build: summary(row),
            job_id: JobId(enqueued.job.id),
            links: JobLinks {
                events: format!("/api/v1/jobs/{}/events", enqueued.job.id),
            },
        })
    }

    pub async fn list_index_builds(
        &self,
        auth: &Authenticated,
        project: ProjectId,
    ) -> Result<IndexBuildListResponse, AppError> {
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
        if !Permission::Search.access_for(role).can_read() {
            return Err(AppError::Forbidden("knowledge:read がありません".into()));
        }
        let mut tx = self.store().begin_scoped(project).await?;
        let rows = self.store().index_builds(&mut tx, project.0).await?;
        store::commit(tx).await?;
        Ok(IndexBuildListResponse {
            items: rows.into_iter().map(summary).collect(),
        })
    }

    pub async fn activate_index_build(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        build_id: Uuid,
        trace: Option<String>,
    ) -> Result<IndexBuildSummary, AppError> {
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
        if !Permission::ProviderManage.access_for(role).can_write() {
            return Err(AppError::Forbidden("provider:manage がありません".into()));
        }
        let mut tx = self.store().begin_scoped(project).await?;
        let build = self
            .store()
            .index_build(&mut tx, build_id)
            .await?
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
        if build.status != "READY" {
            return Err(AppError::Validation(
                "READY の索引だけを有効化できます".into(),
            ));
        }
        let activated = self
            .store()
            .activate_index_build(&mut tx, project.0, build_id)
            .await?;
        if !activated {
            return Err(AppError::Validation("索引を有効化できません".into()));
        }
        let row = self
            .store()
            .index_build(&mut tx, build_id)
            .await?
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
        self.store()
            .append_audit(
                &mut tx,
                &store::AuditDraft {
                    project_id: Some(project.0),
                    actor_type: "USER".into(),
                    actor_id: auth.user_id.to_string(),
                    action: "index_build.activate".into(),
                    target_type: Some("index_build".into()),
                    target_id: Some(build_id.to_string()),
                    input_digest: None,
                    result: "SUCCESS".into(),
                    detail: json!({"model": row.model_name, "revision": row.revision}),
                    trace_id: trace,
                },
            )
            .await?;
        store::commit(tx).await?;
        Ok(summary(row))
    }

    pub async fn fail_index_build(
        &self,
        project: ProjectId,
        build_id: Uuid,
        message: &str,
    ) -> Result<(), AppError> {
        let mut tx = self.store().begin_scoped(project).await?;
        let count = self
            .store()
            .index_build(&mut tx, build_id)
            .await?
            .map(|row| row.chunk_count)
            .unwrap_or(0);
        self.store()
            .update_index_build(
                &mut tx,
                build_id,
                "FAILED",
                count,
                Some(&json!({"message": message})),
            )
            .await?;
        store::commit(tx).await?;
        Ok(())
    }

    pub async fn build_index_batch(
        &self,
        project: ProjectId,
        build_id: Uuid,
        model_id: Uuid,
        revision: &str,
        limit: i64,
    ) -> Result<(usize, i64), AppError> {
        let mut tx = self.store().begin_scoped(project).await?;
        let batch = self
            .store()
            .chunks_pending_embedding(&mut tx, project.0, model_id, limit)
            .await?;
        let processed = batch.len();
        for (chunk_id, text) in batch {
            let embedding = engine_rag::embed_for_revision(&text, revision);
            self.store()
                .upsert_embedding(&mut tx, chunk_id, project.0, model_id, &embedding)
                .await?;
        }
        let total = self
            .store()
            .embedding_count(&mut tx, project.0, model_id)
            .await?;
        self.store()
            .update_index_build(&mut tx, build_id, "BUILDING", total, None)
            .await?;
        store::commit(tx).await?;
        Ok((processed, total))
    }

    pub async fn finish_index_build(
        &self,
        project: ProjectId,
        build_id: Uuid,
    ) -> Result<i64, AppError> {
        let mut tx = self.store().begin_scoped(project).await?;
        let build = self
            .store()
            .index_build(&mut tx, build_id)
            .await?
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
        let total = self
            .store()
            .embedding_count(&mut tx, project.0, build.model_id)
            .await?;
        self.store()
            .update_index_build(&mut tx, build_id, "READY", total, None)
            .await?;
        store::commit(tx).await?;
        Ok(total)
    }
}

fn summary(row: store::IndexBuildRow) -> IndexBuildSummary {
    IndexBuildSummary {
        id: row.id,
        model_id: row.model_id,
        model_name: row.model_name,
        revision: row.revision,
        status: row.status,
        chunk_count: row.chunk_count,
        started_at: row.started_at,
        finished_at: row.finished_at,
        is_active: row.is_active,
    }
}
