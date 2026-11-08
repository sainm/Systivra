use std::path::PathBuf;

use contracts::{ArtifactCleanupResponse, ErrorCode};
use domain::ProjectId;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{App, AppError, Authenticated};

fn artifact_root() -> PathBuf {
    std::env::var("ARTIFACT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn expire_entries(
    artifacts: &Value,
    retention_days: i64,
    trace_days: i64,
    force: bool,
) -> (Value, i64, i64, i64) {
    let root = artifact_root();
    let mut entries = artifacts.as_array().cloned().unwrap_or_default();
    let mut expired = 0i64;
    let mut deleted_files = 0i64;
    let mut bytes_reclaimed = 0i64;
    for entry in &mut entries {
        let kind = entry.get("kind").and_then(Value::as_str).unwrap_or("");
        if kind == "UPLOAD_FAILED" {
            continue;
        }
        if entry.get("status").and_then(Value::as_str) == Some("EXPIRED") {
            continue;
        }
        let uri = entry.get("uri").and_then(Value::as_str).unwrap_or("");
        if uri.is_empty() {
            continue;
        }
        let path = root.join(uri);
        if !path.starts_with(&root) {
            continue;
        }
        let meta = std::fs::metadata(&path);
        let age_days = match &meta {
            Ok(m) => m
                .modified()
                .ok()
                .and_then(|t| t.elapsed().ok())
                .map(|d| d.as_secs() as f64 / 86_400.0)
                .unwrap_or(f64::MAX),
            Err(_) => f64::MAX,
        };
        let limit = if kind == "TRACE" {
            trace_days
        } else {
            retention_days
        } as f64;
        if force || age_days > limit {
            let bytes = entry.get("bytes").and_then(Value::as_i64).unwrap_or(0);
            if meta.is_ok() && std::fs::remove_file(&path).is_ok() {
                deleted_files += 1;
                bytes_reclaimed += bytes;
            }
            entry["status"] = json!("EXPIRED");
            entry["bytes"] = json!(0);
            expired += 1;
        }
    }
    (
        Value::Array(entries),
        expired,
        deleted_files,
        bytes_reclaimed,
    )
}

impl App {
    pub async fn expire_test_artifacts(
        &self,
        project: Option<ProjectId>,
        retention_days: i64,
        trace_days: i64,
    ) -> Result<ArtifactCleanupResponse, AppError> {
        let mut tx = match project {
            Some(p) => self.store().begin_scoped(p).await?,
            None => self.store().begin_unscoped().await?,
        };
        let rows = self
            .store()
            .executions_with_artifacts(&mut tx, project.map(|p| p.0))
            .await?;
        let mut response = ArtifactCleanupResponse {
            scanned: rows.len() as i64,
            expired: 0,
            deleted_files: 0,
            bytes_reclaimed: 0,
        };
        for (id, artifacts, _started) in rows {
            let (updated, expired, deleted, reclaimed) =
                expire_entries(&artifacts, retention_days, trace_days, false);
            if expired > 0 {
                self.store()
                    .update_execution_artifacts(&mut tx, id, &updated)
                    .await?;
            }
            response.expired += expired;
            response.deleted_files += deleted;
            response.bytes_reclaimed += reclaimed;
        }
        store::commit(tx).await?;
        Ok(response)
    }

    pub async fn cleanup_run_artifacts(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        run_id: Uuid,
        trace: Option<String>,
    ) -> Result<ArtifactCleanupResponse, AppError> {
        self.require_test_run(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        self.store()
            .test_run(&mut tx, project.0, run_id)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFound))?;
        let executions = self
            .store()
            .list_test_executions(&mut tx, project.0)
            .await?;
        let mut response = ArtifactCleanupResponse {
            scanned: 0,
            expired: 0,
            deleted_files: 0,
            bytes_reclaimed: 0,
        };
        for execution in executions.iter().filter(|e| e.test_run_id == run_id) {
            if execution
                .artifacts
                .as_array()
                .map(Vec::is_empty)
                .unwrap_or(true)
            {
                continue;
            }
            response.scanned += 1;
            let (updated, expired, deleted, reclaimed) =
                expire_entries(&execution.artifacts, 0, 0, true);
            if expired > 0 {
                self.store()
                    .update_execution_artifacts(&mut tx, execution.id, &updated)
                    .await?;
            }
            response.expired += expired;
            response.deleted_files += deleted;
            response.bytes_reclaimed += reclaimed;
        }
        self.store()
            .append_audit(
                &mut tx,
                &store::AuditDraft {
                    project_id: Some(project.0),
                    actor_type: "USER".into(),
                    actor_id: auth.user_id.to_string(),
                    action: "test.artifacts.cleanup".into(),
                    target_type: Some("test.test_run".into()),
                    target_id: Some(run_id.to_string()),
                    input_digest: None,
                    result: "OK".into(),
                    detail: json!({
                        "expired": response.expired,
                        "deleted_files": response.deleted_files,
                        "bytes_reclaimed": response.bytes_reclaimed,
                    }),
                    trace_id: trace,
                },
            )
            .await?;
        store::commit(tx).await?;
        Ok(response)
    }
}
