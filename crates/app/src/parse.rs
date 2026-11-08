use contracts::{ErrorCode, JobStatus};
use domain::{DocumentId, JobId, ParseRunId, ProjectId, RevisionId};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::AppError;
use crate::App;

#[derive(Debug, Clone)]
pub struct ParseTarget {
    pub project_id: ProjectId,
    pub job_id: JobId,
    pub document_id: DocumentId,
    pub revision_id: RevisionId,
    pub parse_run_id: ParseRunId,
    pub object_key: String,
    pub mime_type: String,
    pub file_size: i64,
}

pub struct ParsePage<'a> {
    pub page: i32,
    pub nodes: &'a [store::NodeDraft],
    pub chunks: &'a [store::ChunkDraft],
    pub evidence: &'a [store::EvidenceDraft],
    pub files_total: i64,
    pub chunks_done: i64,
}

impl ParseTarget {
    pub fn from_job(job: &store::JobRow) -> Result<Self, AppError> {
        let input = &job.input;
        let get_uuid = |key: &str| -> Result<Uuid, AppError> {
            input
                .get(key)
                .and_then(Value::as_str)
                .and_then(|v| v.parse::<Uuid>().ok())
                .ok_or_else(|| AppError::Validation(format!("job input に {key} がありません")))
        };
        Ok(Self {
            project_id: ProjectId(get_uuid("project_id")?),
            job_id: JobId(job.id),
            document_id: DocumentId(get_uuid("document_id")?),
            revision_id: RevisionId(get_uuid("revision_id")?),
            parse_run_id: ParseRunId(get_uuid("parse_run_id")?),
            object_key: input
                .get("object_key")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            mime_type: input
                .get("mime_type")
                .and_then(Value::as_str)
                .unwrap_or("application/pdf")
                .to_string(),
            file_size: input.get("file_size").and_then(Value::as_i64).unwrap_or(0),
        })
    }
}

impl App {
    pub async fn begin_parse(&self, target: &ParseTarget, resume: bool) -> Result<(), AppError> {
        let mut tx = self.store().begin_scoped(target.project_id).await?;
        if !resume {
            self.store()
                .purge_revision(&mut tx, target.revision_id.0)
                .await?;
        }
        self.store()
            .update_parse_run(
                &mut tx,
                target.parse_run_id.0,
                "RUNNING",
                &json!({"page_count": 0, "chunk_count": 0}),
                None,
            )
            .await?;
        store::commit(tx).await?;
        Ok(())
    }

    pub async fn save_parse_page(
        &self,
        target: &ParseTarget,
        page: &ParsePage<'_>,
    ) -> Result<(), AppError> {
        let mut tx = self.store().begin_scoped(target.project_id).await?;
        let (model_id, _model_name, revision) = self
            .store()
            .active_embedding_model(&mut tx, target.project_id.0)
            .await?;
        let mut chunks: Vec<store::ChunkDraft> = page.chunks.to_vec();
        for chunk in &mut chunks {
            chunk.embedding = engine_rag::embed_for_revision(&chunk.text, &revision);
        }
        self.store().insert_nodes(&mut tx, page.nodes).await?;
        self.store()
            .sync_document_nodes(
                &mut tx,
                target.project_id.0,
                target.document_id.0,
                target.revision_id.0,
                page.nodes,
            )
            .await?;
        self.store()
            .insert_chunks(&mut tx, model_id, &chunks)
            .await?;
        for item in page.evidence {
            self.store().insert_evidence(&mut tx, item).await?;
        }
        let progress = json!({
            "phase": "PARSE",
            "files_done": page.page,
            "files_total": page.files_total,
            "cells_done": 0,
            "cells_total": 0,
            "chunks_done": page.chunks_done,
            "done": page.page,
            "total": page.files_total,
        });
        self.store()
            .update_progress(&mut tx, target.job_id.0, &progress, "progress")
            .await
            .ok();
        store::commit(tx).await?;
        Ok(())
    }

    pub async fn finish_parse_success(
        &self,
        target: &ParseTarget,
        page_count: i64,
        chunk_count: i64,
    ) -> Result<(), AppError> {
        let stats = json!({"page_count": page_count, "chunk_count": chunk_count});
        let mut tx = self.store().begin_scoped(target.project_id).await?;
        self.store()
            .update_parse_run(&mut tx, target.parse_run_id.0, "SUCCEEDED", &stats, None)
            .await?;
        self.store()
            .mark_revision_parsed(&mut tx, target.revision_id.0)
            .await?;
        self.store()
            .append_audit(
                &mut tx,
                &store::AuditDraft {
                    project_id: Some(target.project_id.0),
                    actor_type: "SYSTEM".into(),
                    actor_id: "systivra-worker".into(),
                    action: "document.parse.completed".into(),
                    target_type: Some("document".into()),
                    target_id: Some(target.document_id.to_string()),
                    input_digest: None,
                    result: "SUCCESS".into(),
                    detail: json!({
                        "revision_id": target.revision_id,
                        "parse_run_id": target.parse_run_id,
                        "page_count": page_count,
                        "chunk_count": chunk_count,
                    }),
                    trace_id: None,
                },
            )
            .await?;
        store::commit(tx).await?;

        let progress = json!({
            "phase": "DONE",
            "files_done": page_count,
            "files_total": page_count,
            "cells_done": 0,
            "cells_total": 0,
            "chunks_done": chunk_count,
            "done": page_count,
            "total": page_count,
        });
        self.finish_job(
            target.project_id,
            target.job_id.0,
            JobStatus::Succeeded,
            Vec::new(),
            progress,
        )
        .await?;
        Ok(())
    }

    pub async fn finish_parse_failure(
        &self,
        target: &ParseTarget,
        code: &str,
        message: &str,
    ) -> Result<(), AppError> {
        let error = json!([{"path": target.object_key, "code": code, "message": message}]);
        let stats = json!({"page_count": 0, "chunk_count": 0});
        let mut tx = self.store().begin_scoped(target.project_id).await?;
        self.store()
            .update_parse_run(
                &mut tx,
                target.parse_run_id.0,
                "FAILED",
                &stats,
                Some(&error),
            )
            .await?;
        self.store()
            .append_audit(
                &mut tx,
                &store::AuditDraft {
                    project_id: Some(target.project_id.0),
                    actor_type: "SYSTEM".into(),
                    actor_id: "systivra-worker".into(),
                    action: "document.parse.failed".into(),
                    target_type: Some("document".into()),
                    target_id: Some(target.document_id.to_string()),
                    input_digest: None,
                    result: "FAILED".into(),
                    detail: json!({
                        "revision_id": target.revision_id,
                        "code": code,
                        "message": message,
                    }),
                    trace_id: None,
                },
            )
            .await?;
        store::commit(tx).await?;

        let errors = vec![contracts::JobErrorItem {
            path: Some(target.object_key.clone()),
            code: code.to_string(),
            message: message.to_string(),
        }];
        let progress = json!({
            "phase": "FAILED",
            "files_done": 0,
            "files_total": 0,
            "cells_done": 0,
            "cells_total": 0,
            "chunks_done": 0,
            "done": 0,
            "total": 0,
        });
        self.finish_job(
            target.project_id,
            target.job_id.0,
            JobStatus::Failed,
            errors,
            progress,
        )
        .await?;
        Ok(())
    }

    pub async fn audit_parse_rejected(&self, job: &store::JobRow, message: &str) {
        if let Ok(project) = Self::job_project(job) {
            let _ = self
                .append_system_audit(
                    project,
                    "document.parse.rejected",
                    "job",
                    &job.id.to_string(),
                    json!({"message": message}),
                )
                .await;
        }
    }

    pub fn job_project(job: &store::JobRow) -> Result<ProjectId, AppError> {
        Ok(ProjectId(job.project_id))
    }

    pub fn not_found_document() -> AppError {
        AppError::NotFound(ErrorCode::NotFoundDocument)
    }
}

pub async fn chunk_count(app: &App, target: &ParseTarget) -> Result<i64, AppError> {
    let mut tx = app.store().begin_scoped(target.project_id).await?;
    let count = app
        .store()
        .inserted_chunk_count(&mut tx, target.revision_id.0)
        .await?;
    store::commit(tx).await?;
    Ok(count)
}
