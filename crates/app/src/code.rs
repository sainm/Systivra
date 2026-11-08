use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use tokio::process::Command;

use chrono::{DateTime, Utc};
use contracts::{
    CodeAnalysisAcceptedResponse, CodeAnalysisRunListResponse, CodeAnalysisRunResponse,
    CodeEvidence, CodeFactListResponse, CodeFactSummary, CodeRepositoryListResponse,
    CodeRepositorySummary, CodeRevisionListResponse, CodeRevisionSummary,
    CodeSourceFileListResponse, CodeSourceFileSummary, CodeSymbolListResponse, CodeSymbolSummary,
    CreateCodeAnalysisRequest, CreateCodeRepositoryRequest, ErrorCode, JobLinks, JobStatus,
    SyncCodeRevisionRequest, SyncCodeRevisionResponse,
};
use domain::{Permission, ProjectId, Role};
use engine_code::{AnalysisReport, SourceInput};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::auth::Authenticated;
use crate::error::AppError;
use crate::App;

struct HeartbeatGuard(tokio::task::JoinHandle<()>);

impl Drop for HeartbeatGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl App {
    pub async fn code_repositories(
        &self,
        auth: &Authenticated,
        project: ProjectId,
    ) -> Result<CodeRepositoryListResponse, AppError> {
        self.require_code_read(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let rows = self
            .store()
            .list_code_repositories(&mut tx, project.0)
            .await?;
        store::commit(tx).await?;
        Ok(CodeRepositoryListResponse {
            items: rows.into_iter().map(repository_summary).collect(),
        })
    }

    pub async fn create_code_repository(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        request: CreateCodeRepositoryRequest,
        trace: Option<String>,
    ) -> Result<CodeRepositorySummary, AppError> {
        let role = self.require_code_write(auth, project).await?;
        let kind = request.kind.trim().to_ascii_uppercase();
        if kind == "SVN" {
            return Err(AppError::CapabilityUnsupported(
                "SVN は Phase 4 では未導入です。DIR / SNAPSHOT / GIT を使用してください".into(),
            ));
        }
        if !matches!(kind.as_str(), "DIR" | "SNAPSHOT" | "GIT") {
            return Err(AppError::CapabilityUnsupported(
                "repository kind は DIR / SNAPSHOT / GIT のいずれかです".into(),
            ));
        }
        let (repository_key, settings) = if kind == "GIT" {
            let url = self.git_repository_url(&request.path)?;
            if let Some(branch) = request.default_branch.as_deref() {
                validate_git_revision(branch)?;
            }
            let remote = is_git_remote(&url);
            (url, json!({"git": true, "remote": remote}))
        } else {
            (
                self.code_repository_key(&request.path)?,
                json!({"local": true}),
            )
        };
        let id = Uuid::new_v5(
            &project.0,
            format!("code-repository:{kind}:{repository_key}").as_bytes(),
        );
        let mut tx = self.store().begin_scoped(project).await?;
        let row = self
            .store()
            .create_code_repository(
                &mut tx,
                &store::NewCodeRepository {
                    id,
                    project_id: project.0,
                    kind,
                    url: repository_key.clone(),
                    default_branch: request.default_branch,
                    settings,
                },
            )
            .await?;
        self.store()
            .append_audit(
                &mut tx,
                &store::AuditDraft {
                    project_id: Some(project.0),
                    actor_type: "USER".into(),
                    actor_id: auth.user_id.to_string(),
                    action: "code.repository.create".into(),
                    target_type: Some("code.repository".into()),
                    target_id: Some(row.id.to_string()),
                    input_digest: None,
                    result: "SUCCESS".into(),
                    detail: json!({
                        "kind": row.kind,
                        "path": repository_key,
                        "role": role,
                    }),
                    trace_id: trace,
                },
            )
            .await?;
        store::commit(tx).await?;
        Ok(repository_summary(row))
    }

    pub async fn enqueue_code_sync(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        request: SyncCodeRevisionRequest,
        trace: Option<String>,
    ) -> Result<SyncCodeRevisionResponse, AppError> {
        self.require_code_write(auth, project).await?;
        let revision = request.revision.trim();
        if revision.is_empty() || revision.chars().count() > 200 {
            return Err(AppError::Validation("revision は 1〜200 文字です".into()));
        }
        let mut tx = self.store().begin_scoped(project).await?;
        let repository = self
            .store()
            .code_repository(&mut tx, project.0, request.repository_id)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFound))?;
        if repository.kind == "GIT" {
            validate_git_revision(revision)?;
        }
        let immutable =
            matches!(revision.len(), 40 | 64) && revision.chars().all(|c| c.is_ascii_hexdigit());
        let revision_key = if repository.kind == "GIT" && !immutable {
            format!("{revision}@{}", Utc::now().timestamp_millis())
        } else {
            revision.to_owned()
        };
        let idempotency_key = format!("code-sync:{}:{revision_key}", request.repository_id);
        let enqueued = self
            .store()
            .enqueue_job(
                &mut tx,
                &store::NewJob {
                    project_id: project.0,
                    kind: "CODE_SYNC".into(),
                    idempotency_key,
                    input: json!({
                        "project_id": project,
                        "repository_id": request.repository_id,
                        "revision": revision,
                        "committed_at": request.committed_at,
                    }),
                },
            )
            .await?;
        self.store()
            .append_audit(
                &mut tx,
                &store::AuditDraft {
                    project_id: Some(project.0),
                    actor_type: "USER".into(),
                    actor_id: auth.user_id.to_string(),
                    action: "code.revision.sync".into(),
                    target_type: Some("job".into()),
                    target_id: Some(enqueued.job.id.to_string()),
                    input_digest: None,
                    result: "QUEUED".into(),
                    detail: json!({
                        "repository_id": request.repository_id,
                        "revision": revision,
                    }),
                    trace_id: trace,
                },
            )
            .await?;
        store::commit(tx).await?;
        Ok(SyncCodeRevisionResponse {
            job_id: enqueued.job.id,
            status: JobStatus::parse(&enqueued.job.status).unwrap_or(JobStatus::Queued),
            repository_id: request.repository_id,
            revision: revision.into(),
            links: JobLinks {
                events: format!("/api/v1/jobs/{}/events", enqueued.job.id),
            },
        })
    }

    pub async fn code_revisions(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        repository_id: Option<Uuid>,
        limit: i64,
    ) -> Result<CodeRevisionListResponse, AppError> {
        self.require_code_read(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let rows = self
            .store()
            .list_code_revisions(&mut tx, project.0, repository_id, limit.clamp(1, 200))
            .await?;
        let mut items = Vec::with_capacity(rows.len());
        for row in rows {
            let file_count = self
                .store()
                .code_source_file_count(&mut tx, project.0, row.id)
                .await?;
            items.push(revision_summary(row, file_count));
        }
        store::commit(tx).await?;
        Ok(CodeRevisionListResponse { items })
    }

    pub async fn enqueue_code_analysis(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        request: CreateCodeAnalysisRequest,
        trace: Option<String>,
    ) -> Result<CodeAnalysisAcceptedResponse, AppError> {
        self.require_code_write(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        if self
            .store()
            .code_revision(&mut tx, project.0, request.revision_id)
            .await?
            .is_none()
        {
            return Err(AppError::NotFound(ErrorCode::NotFound));
        }
        let run_id = Uuid::new_v5(
            &project.0,
            format!("code-analysis:{}", request.revision_id).as_bytes(),
        );
        let run = self
            .store()
            .create_code_analysis_run(
                &mut tx,
                &store::NewCodeAnalysisRun {
                    id: run_id,
                    project_id: project.0,
                    revision_id: request.revision_id,
                    engine_versions: json!({
                        "scanner": env!("CARGO_PKG_VERSION"),
                        "syntax": "tree-sitter-java-0.23/tree-sitter-html-0.23",
                        "semantic": "source-only",
                    }),
                },
            )
            .await?;
        let idempotency_key = format!("code-analysis:{}", request.revision_id);
        let enqueued = self
            .store()
            .enqueue_job(
                &mut tx,
                &store::NewJob {
                    project_id: project.0,
                    kind: "CODE_ANALYZE".into(),
                    idempotency_key,
                    input: json!({
                        "project_id": project,
                        "run_id": run.id,
                        "revision_id": request.revision_id,
                    }),
                },
            )
            .await?;
        self.store()
            .append_audit(
                &mut tx,
                &store::AuditDraft {
                    project_id: Some(project.0),
                    actor_type: "USER".into(),
                    actor_id: auth.user_id.to_string(),
                    action: "code.analysis.start".into(),
                    target_type: Some("code.analysis_run".into()),
                    target_id: Some(run.id.to_string()),
                    input_digest: None,
                    result: "QUEUED".into(),
                    detail: json!({"revision_id": request.revision_id, "job_id": enqueued.job.id}),
                    trace_id: trace,
                },
            )
            .await?;
        store::commit(tx).await?;
        Ok(CodeAnalysisAcceptedResponse {
            job_id: enqueued.job.id,
            run_id: run.id,
            status: JobStatus::parse(&enqueued.job.status).unwrap_or(JobStatus::Queued),
            links: JobLinks {
                events: format!("/api/v1/jobs/{}/events", enqueued.job.id),
            },
        })
    }

    pub async fn code_analysis_run(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        run_id: Uuid,
    ) -> Result<CodeAnalysisRunResponse, AppError> {
        self.require_code_read(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let row = self
            .store()
            .code_analysis_run(&mut tx, project.0, run_id)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFound))?;
        store::commit(tx).await?;
        Ok(analysis_run_response(row))
    }

    pub async fn code_analysis_runs(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        limit: i64,
    ) -> Result<CodeAnalysisRunListResponse, AppError> {
        self.require_code_read(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let rows = self
            .store()
            .list_code_analysis_runs(&mut tx, project.0, limit.clamp(1, 100))
            .await?;
        store::commit(tx).await?;
        Ok(CodeAnalysisRunListResponse {
            items: rows.into_iter().map(analysis_run_response).collect(),
        })
    }

    pub async fn code_symbols(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        revision_id: Option<Uuid>,
        fqn: &str,
        limit: i64,
    ) -> Result<CodeSymbolListResponse, AppError> {
        self.require_code_read(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let rows = self
            .store()
            .list_code_symbols(&mut tx, project.0, revision_id, fqn, limit.clamp(1, 200))
            .await?;
        store::commit(tx).await?;
        Ok(CodeSymbolListResponse {
            items: rows
                .into_iter()
                .map(|row| CodeSymbolSummary {
                    id: row.id,
                    file_id: row.file_id,
                    kind: row.kind,
                    fqn: row.fqn,
                    simple_name: row.simple_name,
                    signature: row.signature,
                    line_start: row.line_start,
                    line_end: row.line_end,
                    byte_start: row.byte_start,
                    byte_end: row.byte_end,
                })
                .collect(),
        })
    }

    pub async fn code_facts(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        revision_id: Option<Uuid>,
        subject: &str,
        relation: &str,
        limit: i64,
    ) -> Result<CodeFactListResponse, AppError> {
        self.require_code_read(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let rows = self
            .store()
            .list_code_facts(
                &mut tx,
                project.0,
                revision_id,
                subject,
                relation,
                limit.clamp(1, 200),
            )
            .await?;
        store::commit(tx).await?;
        Ok(CodeFactListResponse {
            items: rows.into_iter().map(fact_summary).collect(),
        })
    }

    pub async fn code_source_files(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        revision_id: Uuid,
    ) -> Result<CodeSourceFileListResponse, AppError> {
        self.require_code_read(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let rows = self
            .store()
            .code_source_files(&mut tx, project.0, revision_id)
            .await?;
        store::commit(tx).await?;
        Ok(CodeSourceFileListResponse {
            items: rows
                .into_iter()
                .map(|row| CodeSourceFileSummary {
                    id: row.id,
                    path: row.path,
                    language: row.language,
                    size_bytes: row.size_bytes,
                    loc: row.loc,
                    encoding: row.encoding,
                    exact_offsets: row.exact_offsets,
                    content_hash: hex(&row.content_hash),
                })
                .collect(),
        })
    }

    pub async fn code_source_file_content(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        file_id: Uuid,
    ) -> Result<Vec<u8>, AppError> {
        self.require_code_read(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let file = self
            .store()
            .code_source_file(&mut tx, project.0, file_id)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFound))?;
        let revision = self
            .store()
            .code_revision(&mut tx, project.0, file.revision_id)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFound))?;
        store::commit(tx).await?;
        let key = format!("{}/{}", revision.snapshot_key, file.path);
        Ok(tokio::fs::read(self.object_path(&key)?).await?)
    }

    pub async fn sync_code_revision_from_job(
        &self,
        project: ProjectId,
        job_id: Uuid,
        input: &Value,
    ) -> Result<store::CodeRevisionRow, AppError> {
        let repository_id = required_uuid(input, "repository_id")?;
        let mut revision = required_str(input, "revision")?.to_owned();
        let mut committed_at = input
            .get("committed_at")
            .and_then(Value::as_str)
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc));
        let mut tx = self.store().begin_scoped(project).await?;
        let repository = self
            .store()
            .code_repository(&mut tx, project.0, repository_id)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFound))?;
        let parent = self
            .store()
            .latest_code_revision(&mut tx, project.0, repository_id)
            .await?;
        store::commit(tx).await?;

        let (root, workdir) =
            if repository.kind == "GIT" {
                let (sha, git_committed_at, workdir) = self
                    .materialize_git_snapshot(project, &repository, &revision)
                    .await?;
                revision = sha;
                if committed_at.is_none() {
                    committed_at = git_committed_at;
                }
                (workdir.clone(), Some(workdir))
            } else {
                let root = self.object_path(repository.url.as_deref().ok_or_else(|| {
                    AppError::Validation("repository path がありません".into())
                })?)?;
                (root, None)
            };
        let revision_id = Uuid::new_v5(
            &repository_id,
            format!("code-revision:{revision}").as_bytes(),
        );
        let snapshot_key = format!("code/{}/{}/{}", project.0, repository_id, revision_id);
        let scan = self.scan_code_snapshot(&root, &snapshot_key).await;
        if let Some(workdir) = workdir {
            let _ = tokio::fs::remove_dir_all(workdir).await;
        }
        let files = scan?;
        let tree_digest = code_tree_digest(&files);
        let mut tx = self.store().begin_scoped(project).await?;
        let row = self
            .store()
            .create_code_revision(
                &mut tx,
                &store::NewCodeRevision {
                    id: revision_id,
                    project_id: project.0,
                    repository_id,
                    revision: revision.clone(),
                    parent_id: parent.map(|value| value.id),
                    tree_digest,
                    snapshot_key,
                    committed_at,
                },
                &files
                    .iter()
                    .map(|file| file.draft.clone())
                    .collect::<Vec<_>>(),
            )
            .await?;
        self.store()
            .append_audit(
                &mut tx,
                &store::AuditDraft {
                    project_id: Some(project.0),
                    actor_type: "SYSTEM".into(),
                    actor_id: "systivra-worker".into(),
                    action: "code.revision.sync".into(),
                    target_type: Some("code.revision".into()),
                    target_id: Some(row.id.to_string()),
                    input_digest: Some(row.tree_digest.clone()),
                    result: "SUCCESS".into(),
                    detail: json!({
                        "repository_id": repository_id,
                        "revision": revision,
                        "files": files.len(),
                        "job_id": job_id,
                    }),
                    trace_id: None,
                },
            )
            .await?;
        store::commit(tx).await?;
        Ok(row)
    }

    pub async fn analyze_code_run_from_job(
        &self,
        project: ProjectId,
        job_id: Uuid,
        input: &Value,
    ) -> Result<AnalysisReport, AppError> {
        let run_id = required_uuid(input, "run_id")?;
        let revision_id = required_uuid(input, "revision_id")?;
        let mut tx = self.store().begin_scoped(project).await?;
        let _run = self
            .store()
            .code_analysis_run(&mut tx, project.0, run_id)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFound))?;
        let revision = self
            .store()
            .code_revision(&mut tx, project.0, revision_id)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFound))?;
        self.store().start_code_analysis(&mut tx, run_id).await?;
        let source_files = self
            .store()
            .code_source_files(&mut tx, project.0, revision_id)
            .await?;
        let changed = if let Some(parent_id) = revision.parent_id {
            self.store()
                .changed_code_paths(&mut tx, project.0, parent_id, revision_id)
                .await?
        } else {
            source_files.iter().map(|file| file.path.clone()).collect()
        };
        let mut copied_symbols = 0usize;
        let mut copied_facts = 0usize;
        if let Some(parent_id) = revision.parent_id {
            let copied = self
                .store()
                .copy_unchanged_code_analysis(&mut tx, project.0, parent_id, revision_id)
                .await?;
            copied_symbols = copied.0;
            copied_facts = copied.1;
        }
        store::commit(tx).await?;

        self.mark_job_progress(
            project,
            job_id,
            "SCAN",
            0,
            changed.len() as i64,
            copied_facts as i64,
        )
        .await?;
        let heartbeat_app = self.clone();
        let _heartbeat = HeartbeatGuard(tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(15));
            loop {
                ticker.tick().await;
                if let Err(e) = heartbeat_app.heartbeat_job(project, job_id).await {
                    tracing::warn!(error = %e, "code analysis heartbeat failed");
                }
            }
        }));
        let changed_set: BTreeSet<String> = changed.into_iter().collect();
        let mut inputs = Vec::new();
        for file in &source_files {
            let key = format!("{}/{}", revision.snapshot_key, file.path);
            let bytes = tokio::fs::read(self.object_path(&key)?).await?;
            inputs.push(SourceInput {
                path: file.path.clone(),
                bytes,
            });
        }
        let options = engine_code::AnalyzeOptions {
            classpath_complete: false,
            selected_paths: Some(changed_set.clone()),
        };
        let batch = tokio::task::block_in_place(|| engine_code::analyze_sources(&inputs, &options));
        let mut file_ids: BTreeMap<String, Uuid> = source_files
            .iter()
            .map(|file| (file.path.clone(), file.id))
            .collect();
        let mut tx = self.store().begin_scoped(project).await?;
        let mut symbols = Vec::new();
        let mut facts = Vec::new();
        for analyzed in &batch.files {
            let file_id = *file_ids
                .entry(analyzed.path.clone())
                .or_insert_with(|| Uuid::new_v5(&revision_id, analyzed.path.as_bytes()));
            for symbol in &analyzed.symbols {
                symbols.push(to_new_symbol(
                    project.0,
                    revision_id,
                    file_id,
                    &analyzed.path,
                    symbol,
                ));
            }
            for (index, fact) in analyzed.facts.iter().enumerate() {
                facts.push(to_new_fact(
                    project.0,
                    revision_id,
                    analyzer_evidence_id(revision_id, &analyzed.path, index),
                    fact,
                ));
            }
        }
        self.store().insert_code_symbols(&mut tx, &symbols).await?;
        self.store()
            .insert_code_facts(&mut tx, &revision.revision, &facts)
            .await?;

        let symbol_total = self
            .store()
            .code_symbol_count(&mut tx, project.0, revision_id)
            .await?;
        let (fact_total, unresolved) = self
            .store()
            .code_fact_counts(&mut tx, project.0, revision_id)
            .await?;
        let files_failed = batch.report.errors.len() as i64;
        let mut report = batch.report.clone();
        report.files_analyzed = changed_set.len() as u64;
        let stats = json!({
            "files_total": source_files.len(),
            "files_changed": changed_set.len(),
            "files_analyzed": report.files_analyzed,
            "files_skipped": report.files_skipped,
            "files_failed": files_failed,
            "copied_symbols": copied_symbols,
            "copied_facts": copied_facts,
            "new_symbols": symbols.len(),
            "new_facts": facts.len(),
            "symbols": symbol_total,
            "facts": fact_total,
            "unresolved": unresolved,
            "classpath_complete": false,
            "errors": report.errors,
            "parser": report.parser,
            "engine_versions": {
                "scanner": env!("CARGO_PKG_VERSION"),
                "syntax": "tree-sitter-java-0.23/tree-sitter-html-0.23",
                "semantic": "source-only"
            },
            "degraded_reasons": [
                "classpath 未指定: 外部型・外部シグネチャは解決しない",
                "構文レベルのみ: 型解決は ADR-017 により行わない"
            ]
        });
        let status = if files_failed > 0 {
            "PARTIAL"
        } else {
            "SUCCEEDED"
        };
        self.store()
            .finish_code_analysis(&mut tx, run_id, status, false, &stats, None)
            .await?;
        store::commit(tx).await?;
        Ok(report)
    }

    pub async fn fail_code_analysis_run(
        &self,
        project: ProjectId,
        run_id: Uuid,
        error: &str,
    ) -> Result<(), AppError> {
        let mut tx = self.store().begin_scoped(project).await?;
        self.store()
            .finish_code_analysis(
                &mut tx,
                run_id,
                "FAILED",
                false,
                &json!({}),
                Some(&json!({"message": error})),
            )
            .await?;
        store::commit(tx).await?;
        Ok(())
    }

    async fn git_capture(&self, args: &[&str]) -> Result<String, AppError> {
        let output = Command::new("git")
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GCM_INTERACTIVE", "never")
            .output()
            .await
            .map_err(|e| AppError::Validation(format!("git を実行できません: {e}")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AppError::Validation(format!(
                "git に失敗しました: {}",
                stderr.trim()
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }

    fn git_repository_url(&self, raw: &str) -> Result<String, AppError> {
        let raw = raw.trim();
        if is_git_remote(raw) {
            if std::env::var("GIT_ALLOW_REMOTE")
                .map(|v| v != "1")
                .unwrap_or(true)
            {
                return Err(AppError::Validation(
                    "remote Git は GIT_ALLOW_REMOTE=1 のときだけ許可されます".into(),
                ));
            }
            let authority = raw
                .split("://")
                .nth(1)
                .and_then(|rest| rest.split('/').next())
                .unwrap_or("");
            if (raw.starts_with("http://") || raw.starts_with("https://"))
                && authority.contains('@')
            {
                return Err(AppError::Validation(
                    "URL に資格情報を含めないでください（credential helper を使用）".into(),
                ));
            }
            return Ok(raw.to_owned());
        }
        self.code_repository_key(raw)
    }

    fn git_source(&self, repository_url: &str) -> Result<String, AppError> {
        if is_git_remote(repository_url) {
            if std::env::var("GIT_ALLOW_REMOTE")
                .map(|v| v != "1")
                .unwrap_or(true)
            {
                return Err(AppError::Validation(
                    "remote Git は GIT_ALLOW_REMOTE=1 のときだけ許可されます".into(),
                ));
            }
            return Ok(repository_url.to_owned());
        }
        let data_dir = self
            .data_dir()
            .canonicalize()
            .unwrap_or_else(|_| self.data_dir().to_path_buf());
        let source = self
            .data_dir()
            .join(repository_url)
            .canonicalize()
            .map_err(|_| AppError::Validation("Git のローカルパスが見つかりません".into()))?;
        if !source.starts_with(&data_dir) {
            return Err(AppError::Validation(
                "Git のローカルパスはデータディレクトリ内に置いてください".into(),
            ));
        }
        Ok(source.to_string_lossy().into_owned())
    }

    async fn materialize_git_snapshot(
        &self,
        project: ProjectId,
        repository: &store::CodeRepositoryRow,
        requested: &str,
    ) -> Result<(String, Option<DateTime<Utc>>, PathBuf), AppError> {
        let repository_url = repository
            .url
            .as_deref()
            .ok_or_else(|| AppError::Validation("repository の URL がありません".into()))?;
        let source = self.git_source(repository_url)?;
        let cache = self
            .data_dir()
            .join("code-git")
            .join(project.0.to_string())
            .join(repository.id.to_string());
        if let Some(parent) = cache.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let cache_arg = cache.to_string_lossy().into_owned();
        if !cache.join("HEAD").exists() {
            let mut args = vec!["clone", "--bare"];
            if let Some(branch) = repository.default_branch.as_deref() {
                if !branch.trim().is_empty() {
                    args.push("--branch");
                    args.push(branch);
                }
            }
            args.push(&source);
            args.push(&cache_arg);
            if let Err(error) = self.git_capture(&args).await {
                let _ = tokio::fs::remove_dir_all(&cache).await;
                return Err(error);
            }
        } else {
            let git_dir = format!("--git-dir={cache_arg}");
            self.git_capture(&[
                &git_dir,
                "fetch",
                "--atomic",
                "--prune",
                "origin",
                "+refs/heads/*:refs/heads/*",
                "+refs/tags/*:refs/tags/*",
            ])
            .await?;
        }

        let candidate = if requested.trim().is_empty() || requested == "HEAD" {
            repository
                .default_branch
                .clone()
                .unwrap_or_else(|| "HEAD".to_owned())
        } else {
            requested.to_owned()
        };
        let git_dir = format!("--git-dir={cache_arg}");
        let sha = match self
            .git_capture(&[
                &git_dir,
                "rev-parse",
                "--verify",
                "--end-of-options",
                &format!("{candidate}^{{commit}}"),
            ])
            .await
        {
            Ok(sha) => sha,
            Err(_) => {
                self.git_capture(&[
                    &git_dir,
                    "rev-parse",
                    "--verify",
                    "--end-of-options",
                    &format!("origin/{candidate}^{{commit}}"),
                ])
                .await?
            }
        };
        if sha.len() < 40 || !sha.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Err(AppError::Validation(
                "Git revision が commit SHA に解決されませんでした".into(),
            ));
        }
        let committed_at = self
            .git_capture(&[&git_dir, "show", "-s", "--format=%cI", &sha])
            .await
            .ok()
            .and_then(|value| DateTime::parse_from_rfc3339(value.trim()).ok())
            .map(|value| value.with_timezone(&Utc));

        let workdir = self
            .data_dir()
            .join("code-git-work")
            .join(project.0.to_string())
            .join(repository.id.to_string())
            .join(&sha);
        if workdir.exists() {
            tokio::fs::remove_dir_all(&workdir).await?;
        }
        tokio::fs::create_dir_all(&workdir).await?;
        let checkout = self
            .git_capture(&[
                &git_dir,
                &format!("--work-tree={}", workdir.display()),
                "checkout",
                "-f",
                &sha,
            ])
            .await;
        if let Err(error) = checkout {
            let _ = tokio::fs::remove_dir_all(&workdir).await;
            return Err(error);
        }
        Ok((sha, committed_at, workdir))
    }

    async fn scan_code_snapshot(
        &self,
        source_root: &Path,
        snapshot_key: &str,
    ) -> Result<Vec<ScannedCodeFile>, AppError> {
        let canonical_root = source_root
            .canonicalize()
            .map_err(|_| AppError::Validation("repository path が見つかりません".into()))?;
        let data_dir = self
            .data_dir()
            .canonicalize()
            .unwrap_or_else(|_| self.data_dir().to_path_buf());
        if !canonical_root.starts_with(&data_dir) || !canonical_root.is_dir() {
            return Err(AppError::Validation(
                "repository はデータディレクトリ内のディレクトリを指定してください".into(),
            ));
        }
        let snapshot_root = self.object_path(snapshot_key)?;
        let mut paths = Vec::new();
        collect_code_paths(&canonical_root, &canonical_root, &mut paths)?;
        paths.sort();
        let mut files = Vec::new();
        for absolute in paths {
            let relative = absolute
                .strip_prefix(&canonical_root)
                .map_err(|_| AppError::Validation("repository path の解決に失敗".into()))?
                .to_string_lossy()
                .replace('\\', "/");
            let Some(language) = engine_code::Language::detect(&absolute) else {
                continue;
            };
            let bytes = tokio::fs::read(&absolute).await?;
            if bytes.len() > 20 * 1024 * 1024 {
                continue;
            }
            let decoded = engine_code::text::decide(&bytes);
            let content_hash = Sha256::digest(&bytes).to_vec();
            let (encoding, exact_offsets, line_offsets, loc) = decoded
                .as_ref()
                .map(|value| {
                    (
                        value.encoding.name().to_string(),
                        value.is_exact(),
                        value
                            .line_index()
                            .starts()
                            .iter()
                            .flat_map(|offset| offset.to_le_bytes())
                            .collect::<Vec<_>>(),
                        value.line_index().starts().len() as i32,
                    )
                })
                .unwrap_or_else(|| {
                    (
                        "UNKNOWN".to_string(),
                        false,
                        Vec::new(),
                        bytes.iter().filter(|byte| **byte == b'\n').count() as i32 + 1,
                    )
                });
            let destination = snapshot_root.join(&relative);
            if let Some(parent) = destination.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            let revision_id = Uuid::parse_str(
                snapshot_key
                    .rsplit('/')
                    .next()
                    .ok_or_else(|| AppError::Validation("snapshot key が不正".into()))?,
            )
            .map_err(|_| AppError::Validation("snapshot key が不正".into()))?;
            files.push(ScannedCodeFile {
                draft: store::NewSourceFile {
                    id: Uuid::new_v5(&revision_id, relative.as_bytes()),
                    path: relative,
                    language: language.as_str().into(),
                    content_hash,
                    size_bytes: bytes.len() as i64,
                    encoding,
                    loc,
                    line_offsets,
                    exact_offsets,
                },
            });
            tokio::fs::write(destination, bytes).await?;
        }
        Ok(files)
    }

    fn code_repository_key(&self, raw: &str) -> Result<String, AppError> {
        let raw = raw.trim().strip_prefix("file://").unwrap_or(raw.trim());
        if raw.is_empty() || raw.contains("..") {
            return Err(AppError::Validation("repository path が不正です".into()));
        }
        let input = PathBuf::from(raw);
        let data_dir = self
            .data_dir()
            .canonicalize()
            .unwrap_or_else(|_| self.data_dir().to_path_buf());
        let candidate = if input.is_absolute() {
            input
        } else {
            data_dir.join(input)
        };
        let canonical = candidate
            .canonicalize()
            .map_err(|_| AppError::Validation("repository path が見つかりません".into()))?;
        let relative = canonical
            .strip_prefix(&data_dir)
            .map_err(|_| {
                AppError::Validation("repository はデータディレクトリ内に置いてください".into())
            })?
            .to_string_lossy()
            .replace('\\', "/");
        if relative.is_empty() {
            return Err(AppError::Validation(
                "データディレクトリ直下を repository にはできません".into(),
            ));
        }
        Ok(relative)
    }

    async fn require_code_read(
        &self,
        auth: &Authenticated,
        project: ProjectId,
    ) -> Result<Role, AppError> {
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFound))?;
        if !Permission::CodeAnalysis.access_for(role).can_read() {
            return Err(AppError::Forbidden("code:read がありません".into()));
        }
        Ok(role)
    }

    async fn require_code_write(
        &self,
        auth: &Authenticated,
        project: ProjectId,
    ) -> Result<Role, AppError> {
        let role = self.require_code_read(auth, project).await?;
        if !Permission::CodeAnalysis.access_for(role).can_write() {
            return Err(AppError::Forbidden("code:write がありません".into()));
        }
        Ok(role)
    }
}

#[derive(Debug)]
struct ScannedCodeFile {
    draft: store::NewSourceFile,
}

fn collect_code_paths(
    root: &Path,
    current: &Path,
    output: &mut Vec<PathBuf>,
) -> Result<(), AppError> {
    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.')
            || matches!(
                name.as_str(),
                "target" | "node_modules" | "build" | "dist" | ".git" | ".svn"
            )
        {
            continue;
        }
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_code_paths(root, &path, output)?;
        } else if file_type.is_file()
            && path.starts_with(root)
            && engine_code::Language::detect(&path).is_some()
        {
            output.push(path);
        }
    }
    Ok(())
}

fn code_tree_digest(files: &[ScannedCodeFile]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    for file in files {
        hasher.update(file.draft.path.as_bytes());
        hasher.update([0]);
        hasher.update(&file.draft.content_hash);
        hasher.update([0]);
    }
    hasher.finalize().to_vec()
}

fn to_new_symbol(
    project_id: Uuid,
    revision_id: Uuid,
    file_id: Uuid,
    path: &str,
    symbol: &engine_code::Symbol,
) -> store::NewCodeSymbol {
    store::NewCodeSymbol {
        id: Uuid::new_v5(
            &revision_id,
            format!("symbol:{path}:{}:{}", symbol.kind.as_str(), symbol.fqn).as_bytes(),
        ),
        project_id,
        revision_id,
        file_id,
        kind: symbol.kind.as_str().into(),
        fqn: symbol.fqn.clone(),
        simple_name: symbol.simple_name.clone(),
        signature: symbol.signature.clone(),
        line_start: symbol.line_start,
        line_end: symbol.line_end,
        byte_start: symbol.byte_start as i64,
        byte_end: symbol.byte_end as i64,
        modifiers: symbol.modifiers.clone(),
        metadata: symbol.metadata.clone(),
    }
}

fn to_new_fact(
    project_id: Uuid,
    revision_id: Uuid,
    _evidence_seed: Uuid,
    fact: &engine_code::Fact,
) -> store::NewCodeFact {
    let fact_id = Uuid::new_v5(
        &revision_id,
        format!(
            "fact:{}:{}:{}:{}",
            fact.subject.key, fact.relation, fact.object.key, fact.analysis_engine
        )
        .as_bytes(),
    );
    let evidence = fact
        .evidence
        .iter()
        .enumerate()
        .map(|(index, evidence)| store::NewCodeEvidence {
            id: Uuid::new_v5(
                &fact_id,
                format!("evidence:{index}:{}:{}", evidence.path, evidence.byte_start).as_bytes(),
            ),
            quote: engine_code::analyze::normalize_nfc(&evidence.quote),
            quote_hash: Sha256::digest(
                engine_code::analyze::normalize_nfc(&evidence.quote).as_bytes(),
            )
            .to_vec(),
            locator: json!({
                "kind": "code",
                "path": evidence.path,
                "revision_id": revision_id,
                "byte_start": evidence.byte_start,
                "byte_end": evidence.byte_end,
                "line": evidence.line,
                "column": evidence.column,
            }),
            resolution: fact.resolution.as_str().into(),
            confidence: fact.confidence,
            role: "PRIMARY".into(),
        })
        .collect::<Vec<_>>();
    let canonical = json!({
        "subject_kind": fact.subject.kind,
        "subject_key": fact.subject.key,
        "relation": fact.relation,
        "object_kind": fact.object.kind,
        "object_key": fact.object.key,
        "resolution": fact.resolution.as_str(),
        "analysis_engine": fact.analysis_engine,
        "engine_version": fact.engine_version,
        "candidates": fact.candidates,
    });
    store::NewCodeFact {
        id: fact_id,
        project_id,
        revision_id,
        subject_kind: fact.subject.kind.clone(),
        subject_key: fact.subject.key.clone(),
        relation: fact.relation.clone(),
        object_kind: fact.object.kind.clone(),
        object_key: fact.object.key.clone(),
        resolution: fact.resolution.as_str().into(),
        confidence: fact.confidence,
        analysis_engine: fact.analysis_engine.clone(),
        engine_version: format!("{}:{ENGINE_CODE_VERSION}", fact.engine_version),
        conflict_group: None,
        unresolved_reason: fact.unresolved_reason.clone(),
        candidates: serde_json::to_value(&fact.candidates).unwrap_or_else(|_| json!([])),
        fact_hash: Sha256::digest(canonical.to_string().as_bytes()).to_vec(),
        evidence,
    }
}

const ENGINE_CODE_VERSION: &str = env!("CARGO_PKG_VERSION");

fn analyzer_evidence_id(revision_id: Uuid, path: &str, index: usize) -> Uuid {
    Uuid::new_v5(
        &revision_id,
        format!("analysis-evidence:{path}:{index}").as_bytes(),
    )
}

fn required_uuid(input: &Value, name: &str) -> Result<Uuid, AppError> {
    input
        .get(name)
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(|| AppError::Validation(format!("{name} がありません")))
}

fn required_str<'a>(input: &'a Value, name: &str) -> Result<&'a str, AppError> {
    input
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::Validation(format!("{name} がありません")))
}

fn is_git_remote(value: &str) -> bool {
    ["http://", "https://", "ssh://", "git://"]
        .iter()
        .any(|prefix| value.starts_with(prefix))
}

fn validate_git_revision(value: &str) -> Result<(), AppError> {
    let value = value.trim();
    if value.is_empty()
        || value.chars().count() > 200
        || value.starts_with('-')
        || value.chars().any(char::is_control)
        || value.chars().any(char::is_whitespace)
    {
        return Err(AppError::Validation(
            "Git revision は branch / tag / commit SHA を 1〜200 文字で指定してください".into(),
        ));
    }
    Ok(())
}

fn repository_summary(row: store::CodeRepositoryRow) -> CodeRepositorySummary {
    CodeRepositorySummary {
        id: row.id,
        kind: row.kind,
        path: row.url.unwrap_or_default(),
        default_branch: row.default_branch,
        created_at: row.created_at,
    }
}

fn revision_summary(row: store::CodeRevisionRow, file_count: i64) -> CodeRevisionSummary {
    CodeRevisionSummary {
        id: row.id,
        repository_id: row.repository_id,
        revision: row.revision,
        parent_id: row.parent_id,
        tree_digest: hex(&row.tree_digest),
        is_baseline: row.is_baseline,
        committed_at: row.committed_at,
        created_at: row.created_at,
        file_count,
    }
}

fn analysis_run_response(row: store::CodeAnalysisRunRow) -> CodeAnalysisRunResponse {
    let files_analyzed = row
        .stats
        .get("files_analyzed")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let files_skipped = row
        .stats
        .get("files_skipped")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let files_failed = row
        .stats
        .get("files_failed")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let symbols = row
        .stats
        .get("symbols")
        .and_then(Value::as_i64)
        .or_else(|| row.stats.get("new_symbols").and_then(Value::as_i64))
        .unwrap_or(0);
    let facts = row
        .stats
        .get("facts")
        .and_then(Value::as_i64)
        .or_else(|| row.stats.get("new_facts").and_then(Value::as_i64))
        .unwrap_or(0);
    let unresolved = row
        .stats
        .get("unresolved")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let unresolved_rate = if facts == 0 {
        0.0
    } else {
        unresolved as f64 / facts as f64
    };
    let degraded_reasons = row
        .stats
        .get("degraded_reasons")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    CodeAnalysisRunResponse {
        id: row.id,
        revision_id: row.revision_id,
        status: row.status,
        classpath_complete: row.classpath_complete,
        files_analyzed,
        files_skipped,
        files_failed,
        symbols,
        facts,
        unresolved,
        unresolved_rate,
        degraded_reasons,
        engine_versions: row.engine_versions,
        stats: row.stats,
        error: row.error,
        started_at: row.started_at,
        finished_at: row.finished_at,
        created_at: row.created_at,
    }
}

fn fact_summary(row: store::CodeFactRow) -> CodeFactSummary {
    CodeFactSummary {
        id: row.id,
        revision_id: row.revision_id,
        subject_kind: row.subject_kind,
        subject_key: row.subject_key,
        relation: row.relation,
        object_kind: row.object_kind,
        object_key: row.object_key,
        resolution: row.resolution,
        confidence: row.confidence,
        analysis_engine: row.analysis_engine,
        engine_version: row.engine_version,
        unresolved_reason: row.unresolved_reason,
        candidates: row.candidates,
        evidence: row
            .evidence
            .into_iter()
            .map(|evidence| CodeEvidence {
                id: evidence.id,
                role: evidence.role,
                quote: evidence.quote,
                locator: evidence.locator,
                resolution: evidence.resolution,
                confidence: evidence.confidence,
            })
            .collect(),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_closed_network_git_urls() {
        assert!(is_git_remote("https://git.internal/example.git"));
        assert!(is_git_remote("ssh://git@git.internal/example.git"));
        assert!(is_git_remote("git://git.internal/example.git"));
        assert!(!is_git_remote("file:///srv/git/example.git"));
        assert!(!is_git_remote("repositories/example.git"));
    }

    #[test]
    fn rejects_git_revision_option_injection() {
        assert!(validate_git_revision("main").is_ok());
        assert!(validate_git_revision("release/2026.11").is_ok());
        assert!(validate_git_revision("-h").is_err());
        assert!(validate_git_revision("main --help").is_err());
        assert!(validate_git_revision("").is_err());
    }
}
