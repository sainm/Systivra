use std::time::Duration;

use app::{App, ParseTarget};
use contracts::{BulkDuplicate, BulkImportMode, DocType, JobErrorItem, JobStatus};
use domain::ProjectId;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

const ID_NAMESPACE: Uuid = Uuid::from_u128(0x6f9619ff_8b86_d011_b42d_00c04fc964ff);

pub async fn run(app: &App) -> Result<(), app::AppError> {
    let stale = std::env::var("WORKER_STALE_SEC")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(15);
    let (requeued, failed) = app.requeue_stalled_jobs(stale).await?;
    if requeued > 0 || failed > 0 {
        tracing::warn!(requeued, failed, "stalled jobs recovered");
    }
    // 承認待ちは 24 時間で期限切れ。古い依頼を後から承認させない。
    let approval_ttl = std::env::var("APPROVAL_TTL_SEC")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(86_400);
    let expired = app.expire_pending_approvals(approval_ttl).await?;
    if expired > 0 {
        tracing::info!(expired, "pending approvals expired");
    }
    while let Some(job) = app.claim_next_job().await? {
        tracing::info!(job_id = %job.id, kind = %job.kind, "job claimed");
        // 処理中も心拍を打つ。これが無いと `WORKER_STALE_SEC` を超える job は
        // 別 worker から「落ちた」と見なされ、同じ job が二重に走る。
        let beat_every = Duration::from_secs((stale as u64 / 3).max(1));
        let heartbeat = tokio::spawn({
            let app = app.clone();
            let project = ProjectId(job.project_id);
            let job_id = job.id;
            async move {
                let mut ticker = tokio::time::interval(beat_every);
                ticker.tick().await;
                loop {
                    ticker.tick().await;
                    if let Err(error) = app.heartbeat_job(project, job_id).await {
                        tracing::warn!(job_id = %job_id, %error, "heartbeat failed");
                        break;
                    }
                }
            }
        });
        let result = process(app, &job).await;
        heartbeat.abort();
        if let Err(error) = result {
            tracing::error!(job_id = %job.id, error = %error, "job failed");
            fail_job(app, &job, &error.to_string()).await;
        }
    }
    if let Err(error) = crate::project::drain(app).await {
        tracing::warn!(%error, "graph projector tick failed");
    }
    Ok(())
}

async fn process(app: &App, job: &app::JobRow) -> Result<(), app::AppError> {
    if job.kind == "BULK_IMPORT" {
        return process_bulk(app, job).await;
    }
    if job.kind == "INDEX_BUILD" {
        return process_index_build(app, job).await;
    }
    if job.kind == "CODE_SYNC" {
        return process_code_sync(app, job).await;
    }
    if job.kind == "CODE_ANALYZE" {
        return process_code_analyze(app, job).await;
    }
    if job.kind == "IMPACT_ANALYSIS" {
        return process_impact_analysis(app, job).await;
    }
    if job.kind == "TEST_RUN" {
        return process_test_run(app, job).await;
    }
    if job.kind == "AGENT_RUN" {
        return process_agent_run(app, job).await;
    }
    if job.kind == "WORKFLOW_RUN" {
        return process_workflow_run(app, job).await;
    }
    if job.kind != "DOCUMENT_PARSE" {
        return Err(app::AppError::Validation(format!(
            "unsupported job kind: {}",
            job.kind
        )));
    }
    let target = ParseTarget::from_job(job)?;
    let checkpoint = app
        .load_checkpoint(target.project_id, target.job_id.0)
        .await?;
    let start_page = checkpoint
        .as_ref()
        .and_then(|(_, cursor)| cursor.get("page").and_then(Value::as_i64))
        .unwrap_or(0) as i32;
    let start_chunks = checkpoint
        .as_ref()
        .and_then(|(_, cursor)| cursor.get("chunks_done").and_then(Value::as_i64))
        .unwrap_or(0);

    app.begin_parse(&target, checkpoint.is_some()).await?;
    app.mark_job_progress(
        target.project_id,
        target.job_id.0,
        "EXTRACT",
        0,
        0,
        start_chunks,
    )
    .await?;

    let bytes = app.read_object(&target.object_key).await?;
    if is_xlsx(&target) {
        return process_xlsx(app, &target, &bytes, start_page, start_chunks).await;
    }
    if is_docx(&target) {
        return process_docx(app, &target, &bytes, start_page, start_chunks).await;
    }
    if is_pptx(&target) {
        return process_pptx(app, &target, &bytes, start_page, start_chunks).await;
    }
    let temp = std::env::temp_dir().join(format!("systivra-{}.pdf", target.job_id));
    tokio::fs::write(&temp, &bytes).await?;
    let pages =
        crate::pdf::extract_pages(&temp).map_err(|e| app::AppError::Validation(e.to_string()));
    let _ = tokio::fs::remove_file(&temp).await;
    let pages = pages?;
    let page_count = pages.len() as i64;

    let mut chunks_done = start_chunks;
    for (index, text) in pages.iter().enumerate() {
        let page = (index + 1) as i32;
        if page <= start_page {
            continue;
        }
        let nodes = page_nodes(&target, page, text);
        let plans = engine_rag::chunk_page(target.revision_id.0, page, text, chunks_done as i32);
        let mut chunks = Vec::with_capacity(plans.len());
        let mut evidence = Vec::with_capacity(plans.len());
        for plan in &plans {
            if !text.contains(&plan.text) {
                return Err(app::AppError::Validation(
                    "evidence quote is not a substring of page text".into(),
                ));
            }
            let embedding = engine_rag::embed(&plan.text);
            let locator = json!({
                "kind": "pdf",
                "page": plan.page,
                "page_end": plan.page_end,
                "char_range": plan.char_range,
                "bbox": null,
            });
            let chunk_id = plan.id;
            chunks.push(app::ChunkDraft {
                id: chunk_id,
                project_id: target.project_id.0,
                document_id: target.document_id.0,
                revision_id: target.revision_id.0,
                chunk_type: "TEXT".into(),
                ordinal: plan.ordinal,
                text: plan.text.clone(),
                tokenized: plan.tokenized.clone(),
                token_count: plan.token_count,
                char_count: plan.text.chars().count() as i32,
                content_hash: engine_rag::sha256(engine_rag::normalize_nfc(&plan.text).as_bytes()),
                locator: locator.clone(),
                node_path: vec!["DOCUMENT".into(), format!("SECTION:{page}")],
                metadata: json!({"page": page, "parser": "in-process-pdf", "tokenizer": "lindera-ipadic", "tokenizer_version": engine_rag::TOKENIZER_VERSION}),
                embedding,
            });
            evidence.push(app::EvidenceDraft {
                id: Uuid::new_v5(&ID_NAMESPACE, format!("evidence:{chunk_id}").as_bytes()),
                project_id: target.project_id.0,
                source_type: "PDF_PAGE_TEXT".into(),
                document_id: Some(target.document_id.0),
                revision_id: Some(target.revision_id.0),
                chunk_id: Some(chunk_id),
                locator,
                quote: engine_rag::normalize_nfc(&plan.text),
                quote_hash: engine_rag::quote_hash(&plan.text),
                resolution: "RESOLVED".into(),
                confidence: 1.0,
            });
        }
        chunks_done += plans.len() as i64;
        app.save_parse_page(
            &target,
            &app::ParsePage {
                page,
                nodes: &nodes,
                chunks: &chunks,
                evidence: &evidence,
                files_total: page_count,
                chunks_done,
            },
        )
        .await?;
        app.save_checkpoint(
            target.project_id,
            target.job_id.0,
            page,
            &json!({"page": page, "chunks_done": chunks_done}),
        )
        .await?;
        tracing::info!(job_id = %target.job_id, page, page_count, chunks_done, "page indexed");
        if let Ok(ms) = std::env::var("WORKER_PAGE_DELAY_MS") {
            if let Ok(ms) = ms.parse::<u64>() {
                tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
            }
        }
    }

    let chunk_count = app::parse::chunk_count(app, &target).await?;
    app.finish_parse_success(&target, page_count, chunk_count)
        .await?;
    Ok(())
}

async fn process_impact_analysis(app: &App, job: &app::JobRow) -> Result<(), app::AppError> {
    let project = ProjectId(job.project_id);
    let impact_id = job
        .input
        .get("impact_id")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(|| app::AppError::Validation("impact_id がありません".into()))?;
    app.mark_job_progress(project, job.id, "IMPACT", 0, 0, 0)
        .await?;
    let report = match app
        .run_impact_assessment_from_job(project, job.id, &job.input)
        .await
    {
        Ok(report) => report,
        Err(error) => {
            let _ = app
                .fail_impact_assessment(project, impact_id, &error.to_string())
                .await;
            return Err(error);
        }
    };
    let errors = report
        .degraded
        .iter()
        .map(|reason| JobErrorItem {
            path: None,
            code: "IMPACT_PARTIAL".into(),
            message: reason.clone(),
        })
        .collect::<Vec<_>>();
    let progress = json!({
        "phase": "DONE",
        "files_done": 0,
        "files_total": 0,
        "cells_done": 0,
        "cells_total": 0,
        "chunks_done": report.stats.total,
        "impact_id": impact_id,
        "unverified": report.unverified.len(),
    });
    let status = if report.degraded.is_empty() && report.unverified.is_empty() {
        JobStatus::Succeeded
    } else {
        JobStatus::SucceededWithErrors
    };
    app.finish_job(project, job.id, status, errors, progress)
        .await?;
    tracing::info!(
        job_id = %job.id,
        impact_id = %impact_id,
        items = report.stats.total,
        "impact analysis finished"
    );
    Ok(())
}

/// TEST_RUN は「実行準備」だけを行う。
/// Playwright Runner は本縦切では未接続なので、PASSED / FAILED を生成しない。
/// READY（実行待ち）まで進め、結果は `executions:import` から取り込む。
async fn process_test_run(app: &App, job: &app::JobRow) -> Result<(), app::AppError> {
    let project = ProjectId(job.project_id);
    app.mark_job_progress(project, job.id, "TEST_PREPARE", 0, 0, 0)
        .await?;
    let case_count = match app
        .prepare_test_run_from_job(project, job.id, &job.input)
        .await
    {
        Ok(count) => count,
        Err(error) => return Err(error),
    };
    let progress = json!({
        "phase": "READY",
        "files_done": 0,
        "files_total": 0,
        "cells_done": 0,
        "cells_total": 0,
        "chunks_done": case_count,
        "case_count": case_count,
        "runner": "EXTERNAL_PROCESS",
    });
    let errors = vec![JobErrorItem {
        path: None,
        code: "RUNNER_EXTERNAL_PROCESS".into(),
        message: "Runner は別プロセス（workers/runner）。準備は READY で完了し、結果は executions:import で戻る".into(),
    }];
    app.finish_job(
        project,
        job.id,
        JobStatus::SucceededWithErrors,
        errors,
        progress,
    )
    .await?;
    tracing::info!(job_id = %job.id, case_count, "test run prepared (runner not connected)");
    Ok(())
}

/// AGENT_RUN は AI が未設定でも「失敗として記録」して正常終了する。
/// job を FAILED にすると worker が再試行し、同じ失敗を繰り返すため。
async fn process_agent_run(app: &App, job: &app::JobRow) -> Result<(), app::AppError> {
    let project = ProjectId(job.project_id);
    app.mark_job_progress(project, job.id, "AGENT", 0, 0, 0)
        .await?;
    let run = app.run_agent_from_job(project, job.id, &job.input).await?;
    let code = run
        .error
        .as_ref()
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
        .unwrap_or("AGENT_FAILED")
        .to_owned();
    let message = run
        .error
        .as_ref()
        .and_then(|error| error.get("detail"))
        .and_then(Value::as_str)
        .unwrap_or("agent run did not succeed")
        .to_owned();
    let errors = if run.status == "SUCCEEDED" {
        Vec::new()
    } else {
        vec![JobErrorItem {
            path: None,
            code,
            message,
        }]
    };
    let progress = json!({
        "phase": "DONE",
        "files_done": 0,
        "files_total": 0,
        "cells_done": 0,
        "cells_total": 0,
        "chunks_done": 0,
        "run_id": run.id,
        "agent": run.agent_code,
        "status": run.status,
    });
    app.finish_job(
        project,
        job.id,
        JobStatus::SucceededWithErrors,
        errors,
        progress,
    )
    .await?;
    tracing::info!(job_id = %job.id, status = %run.status, "agent run finished");
    Ok(())
}

/// WORKFLOW_RUN は承認ノードで WAITING_APPROVAL になっても job は成功で閉じる。
/// 再開は承認後に別 job として投入される。
async fn process_workflow_run(app: &App, job: &app::JobRow) -> Result<(), app::AppError> {
    let project = ProjectId(job.project_id);
    app.mark_job_progress(project, job.id, "WORKFLOW", 0, 0, 0)
        .await?;
    let run = app
        .run_workflow_from_job(project, job.id, &job.input)
        .await?;
    let progress = json!({
        "phase": "DONE",
        "files_done": 0,
        "files_total": 0,
        "cells_done": 0,
        "cells_total": 0,
        "chunks_done": 0,
        "run_id": run.id,
        "workflow": run.code,
        "status": run.status,
        "current_node": run.current_node,
    });
    app.finish_job(project, job.id, JobStatus::Succeeded, Vec::new(), progress)
        .await?;
    tracing::info!(job_id = %job.id, status = %run.status, "workflow run finished");
    Ok(())
}

async fn process_code_sync(app: &App, job: &app::JobRow) -> Result<(), app::AppError> {
    let project = ProjectId(job.project_id);
    app.mark_job_progress(project, job.id, "SCAN", 0, 0, 0)
        .await?;
    let revision = app
        .sync_code_revision_from_job(project, job.id, &job.input)
        .await?;
    let progress = json!({
        "phase": "DONE",
        "files_done": 0,
        "files_total": 0,
        "cells_done": 0,
        "cells_total": 0,
        "chunks_done": 0,
        "revision_id": revision.id,
        "revision": revision.revision,
    });
    app.finish_job(project, job.id, JobStatus::Succeeded, Vec::new(), progress)
        .await?;
    tracing::info!(
        job_id = %job.id,
        revision_id = %revision.id,
        "code revision synchronized"
    );
    Ok(())
}

async fn process_code_analyze(app: &App, job: &app::JobRow) -> Result<(), app::AppError> {
    let project = ProjectId(job.project_id);
    let run_id = job
        .input
        .get("run_id")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(|| app::AppError::Validation("run_id がありません".into()))?;
    let result = app
        .analyze_code_run_from_job(project, job.id, &job.input)
        .await;
    let report = match result {
        Ok(report) => report,
        Err(error) => {
            let _ = app
                .fail_code_analysis_run(project, run_id, &error.to_string())
                .await;
            return Err(error);
        }
    };
    let unresolved = report
        .facts
        .iter()
        .filter(|fact| fact.resolution != engine_code::FactResolution::Resolved)
        .count() as i64;
    let errors = report
        .errors
        .iter()
        .map(|error| JobErrorItem {
            path: Some(error.path.clone()),
            code: error.code.clone(),
            message: error.message.clone(),
        })
        .collect::<Vec<_>>();
    let total = report.files_analyzed as i64;
    let progress = json!({
        "phase": "DONE",
        "files_done": total,
        "files_total": total,
        "cells_done": 0,
        "cells_total": 0,
        "chunks_done": report.facts.len() as i64,
        "run_id": run_id,
        "unresolved": unresolved,
    });
    let status = if errors.is_empty() {
        JobStatus::Succeeded
    } else {
        JobStatus::SucceededWithErrors
    };
    app.finish_job(project, job.id, status, errors, progress)
        .await?;
    tracing::info!(
        job_id = %job.id,
        run_id = %run_id,
        files = total,
        symbols = report.symbols.len(),
        facts = report.facts.len(),
        "code analysis finished"
    );
    Ok(())
}

#[derive(Debug, Deserialize)]
struct ManifestEntry {
    path: String,
    doc_type: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    business_domain: Option<String>,
    #[serde(default)]
    acl_groups: Vec<String>,
    #[serde(default)]
    source_revision: Option<String>,
}

async fn process_index_build(app: &App, job: &app::JobRow) -> Result<(), app::AppError> {
    let project = ProjectId(job.project_id);
    let build_id = job
        .input
        .get("build_id")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(|| app::AppError::Validation("build_id がありません".into()))?;
    let model_id = job
        .input
        .get("model_id")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(|| app::AppError::Validation("model_id がありません".into()))?;
    let revision = job
        .input
        .get("revision")
        .and_then(Value::as_str)
        .ok_or_else(|| app::AppError::Validation("revision がありません".into()))?;

    loop {
        let (processed, count) = app
            .build_index_batch(project, build_id, model_id, revision, 200)
            .await?;
        app.mark_job_progress(project, job.id, "INDEX", count, count, count)
            .await?;
        app.save_checkpoint(
            project,
            job.id,
            count as i32,
            &json!({"chunks_done": count}),
        )
        .await?;
        if processed == 0 {
            break;
        }
    }
    let total = app.finish_index_build(project, build_id).await?;
    let progress = json!({
        "phase": "DONE",
        "files_done": total,
        "files_total": total,
        "cells_done": 0,
        "cells_total": 0,
        "chunks_done": total,
    });
    app.finish_job(project, job.id, JobStatus::Succeeded, Vec::new(), progress)
        .await?;
    tracing::info!(job_id = %job.id, build_id = %build_id, chunks = total, "index build ready");
    Ok(())
}

async fn process_bulk(app: &App, job: &app::JobRow) -> Result<(), app::AppError> {
    let project = ProjectId(job.project_id);
    let manifest_key = job
        .input
        .get("manifest_key")
        .and_then(Value::as_str)
        .ok_or_else(|| app::AppError::Validation("manifest_key がありません".into()))?;
    let mode: BulkImportMode = serde_json::from_value(
        job.input
            .get("mode")
            .cloned()
            .unwrap_or_else(|| json!("FULL")),
    )
    .map_err(|_| app::AppError::Validation("mode が不正です".into()))?;
    let on_duplicate: BulkDuplicate = serde_json::from_value(
        job.input
            .get("on_duplicate")
            .cloned()
            .unwrap_or_else(|| json!("SKIP")),
    )
    .map_err(|_| app::AppError::Validation("on_duplicate が不正です".into()))?;

    let bytes = app.read_object(manifest_key).await?;
    let text = String::from_utf8(bytes)
        .map_err(|_| app::AppError::Validation("manifest は UTF-8 で保存してください".into()))?;
    let lines: Vec<(usize, &str)> = text
        .lines()
        .enumerate()
        .map(|(index, line)| (index + 1, line))
        .filter(|(_, line)| !line.trim().is_empty())
        .collect();
    let total = lines.len() as i64;

    let checkpoint = app.load_checkpoint(project, job.id).await?;
    let (start_line, mut done, mut skipped, mut failed) = checkpoint
        .as_ref()
        .map(|(_, cursor)| {
            (
                cursor.get("line").and_then(Value::as_i64).unwrap_or(0),
                cursor
                    .get("files_done")
                    .and_then(Value::as_i64)
                    .unwrap_or(0),
                cursor
                    .get("skipped_duplicate")
                    .and_then(Value::as_i64)
                    .unwrap_or(0),
                cursor
                    .get("files_failed")
                    .and_then(Value::as_i64)
                    .unwrap_or(0),
            )
        })
        .unwrap_or((0, 0, 0, 0));
    let data_dir = app
        .data_dir()
        .canonicalize()
        .unwrap_or_else(|_| app.data_dir().to_path_buf());
    let manifest_dir = manifest_key
        .rsplit_once('/')
        .map(|(dir, _)| dir)
        .unwrap_or("");
    let mut errors: Vec<JobErrorItem> = Vec::new();

    app.mark_job_progress(project, job.id, "BULK", done, total, 0)
        .await?;

    for (line_number, line) in lines {
        if (line_number as i64) <= start_line {
            continue;
        }
        match serde_json::from_str::<ManifestEntry>(line) {
            Ok(entry) => {
                match import_manifest_entry(
                    app,
                    project,
                    job.id,
                    &data_dir,
                    manifest_dir,
                    &entry,
                    mode,
                    on_duplicate,
                    line_number,
                )
                .await
                {
                    Ok(app::document::BulkEntryOutcome::SkippedDuplicate) => skipped += 1,
                    Ok(_) => {}
                    Err(error) => {
                        failed += 1;
                        errors.push(JobErrorItem {
                            path: Some(entry.path.clone()),
                            code: error.code().as_str().to_string(),
                            message: error.to_string(),
                        });
                    }
                }
            }
            Err(error) => {
                failed += 1;
                errors.push(JobErrorItem {
                    path: None,
                    code: "VALIDATION_SCHEMA".into(),
                    message: format!("line {line_number}: {error}"),
                });
            }
        }
        done += 1;
        app.save_checkpoint(
            project,
            job.id,
            line_number as i32,
            &json!({
                "line": line_number,
                "files_done": done,
                "skipped_duplicate": skipped,
                "files_failed": failed,
            }),
        )
        .await?;
        if line_number % 20 == 0 {
            app.mark_job_progress(project, job.id, "BULK", done, total, 0)
                .await?;
        }
        if let Ok(ms) = std::env::var("BULK_LINE_DELAY_MS") {
            if let Ok(ms) = ms.parse::<u64>() {
                tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
            }
        }
    }

    let status = if failed > 0 {
        JobStatus::SucceededWithErrors
    } else {
        JobStatus::Succeeded
    };
    let progress = json!({
        "phase": "DONE",
        "files_done": done,
        "files_total": total,
        "cells_done": 0,
        "cells_total": 0,
        "chunks_done": 0,
        "done": done,
        "total": total,
        "skipped_duplicate": skipped,
        "files_failed": failed,
    });
    app.finish_job(project, job.id, status, errors, progress)
        .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn import_manifest_entry(
    app: &App,
    project: ProjectId,
    parent_job: Uuid,
    data_dir: &std::path::Path,
    manifest_dir: &str,
    entry: &ManifestEntry,
    mode: BulkImportMode,
    on_duplicate: BulkDuplicate,
    line: usize,
) -> Result<app::document::BulkEntryOutcome, app::AppError> {
    let candidate = data_dir.join(manifest_dir).join(&entry.path);
    let canonical = candidate.canonicalize().map_err(|_| {
        app::AppError::Validation(format!("ファイルが見つかりません: {}", entry.path))
    })?;
    if !canonical.starts_with(data_dir) {
        return Err(app::AppError::Validation(
            "manifest の path がデータディレクトリ外です".into(),
        ));
    }
    let bytes = tokio::fs::read(&canonical).await?;
    let filename = canonical
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&entry.path)
        .to_string();
    let title = entry
        .title
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            filename
                .rsplit_once('.')
                .map(|(stem, _)| stem.to_string())
                .unwrap_or_else(|| filename.clone())
        });
    let doc_type = DocType::parse(&entry.doc_type).ok_or_else(|| {
        app::AppError::Validation(format!("doc_type が不正です: {}", entry.doc_type))
    })?;
    app.bulk_import_entry(
        project,
        app::document::BulkEntry {
            doc_type,
            title,
            business_domain: entry.business_domain.clone(),
            acl_groups: entry.acl_groups.clone(),
            source_revision: entry.source_revision.clone(),
            filename,
            bytes,
        },
        mode,
        on_duplicate,
        parent_job,
        line,
    )
    .await
}

fn is_xlsx(target: &ParseTarget) -> bool {
    let mime = target.mime_type.to_ascii_lowercase();
    let key = target.object_key.to_ascii_lowercase();
    mime.contains("spreadsheetml")
        || mime.contains("ms-excel.sheet")
        || key.ends_with(".xlsx")
        || key.ends_with(".xlsm")
}

fn is_docx(target: &ParseTarget) -> bool {
    let mime = target.mime_type.to_ascii_lowercase();
    let key = target.object_key.to_ascii_lowercase();
    mime.contains("wordprocessingml") || key.ends_with(".docx")
}

fn is_pptx(target: &ParseTarget) -> bool {
    let mime = target.mime_type.to_ascii_lowercase();
    let key = target.object_key.to_ascii_lowercase();
    mime.contains("presentationml") || key.ends_with(".pptx")
}

struct OfficePageView<'a> {
    index: i32,
    label: &'a str,
    nodes: &'a [app::NodeDraft],
    chunks: &'a [app::ChunkDraft],
    evidence: &'a [app::EvidenceDraft],
}

async fn process_docx(
    app: &App,
    target: &ParseTarget,
    bytes: &[u8],
    start_page: i32,
    start_chunks: i64,
) -> Result<(), app::AppError> {
    let ingest = match crate::docx::read(
        target.project_id.0,
        target.document_id.0,
        target.revision_id.0,
        &target.object_key,
        bytes,
    ) {
        Ok(ingest) => ingest,
        Err(error) => {
            app.finish_parse_failure(target, error.code(), &error.message())
                .await?;
            return Ok(());
        }
    };
    let pages: Vec<OfficePageView<'_>> = ingest
        .pages
        .iter()
        .map(|page| OfficePageView {
            index: page.index,
            label: &page.label,
            nodes: &page.nodes,
            chunks: &page.chunks,
            evidence: &page.evidence,
        })
        .collect();
    save_office_pages(app, target, &pages, start_page, start_chunks).await
}

async fn process_pptx(
    app: &App,
    target: &ParseTarget,
    bytes: &[u8],
    start_page: i32,
    start_chunks: i64,
) -> Result<(), app::AppError> {
    let ingest = match crate::pptx::read(
        target.project_id.0,
        target.document_id.0,
        target.revision_id.0,
        &target.object_key,
        bytes,
    ) {
        Ok(ingest) => ingest,
        Err(error) => {
            app.finish_parse_failure(target, error.code(), &error.message())
                .await?;
            return Ok(());
        }
    };
    let pages: Vec<OfficePageView<'_>> = ingest
        .pages
        .iter()
        .map(|page| OfficePageView {
            index: page.index,
            label: &page.label,
            nodes: &page.nodes,
            chunks: &page.chunks,
            evidence: &page.evidence,
        })
        .collect();
    save_office_pages(app, target, &pages, start_page, start_chunks).await
}

async fn save_office_pages(
    app: &App,
    target: &ParseTarget,
    pages: &[OfficePageView<'_>],
    start_page: i32,
    start_chunks: i64,
) -> Result<(), app::AppError> {
    let total = pages.len() as i64;
    let mut chunks_done = start_chunks;
    for page in pages {
        if page.index <= start_page {
            continue;
        }
        chunks_done += page.chunks.len() as i64;
        app.save_parse_page(
            target,
            &app::ParsePage {
                page: page.index,
                nodes: page.nodes,
                chunks: page.chunks,
                evidence: page.evidence,
                files_total: total,
                chunks_done,
            },
        )
        .await?;
        app.save_checkpoint(
            target.project_id,
            target.job_id.0,
            page.index,
            &json!({"page": page.index, "chunks_done": chunks_done}),
        )
        .await?;
        tracing::info!(
            job_id = %target.job_id.0,
            section = page.label,
            page_index = page.index,
            page_count = total,
            chunks_done,
            "document section indexed"
        );
    }
    let chunk_count = app::parse::chunk_count(app, target).await?;
    app.finish_parse_success(target, total, chunk_count).await?;
    Ok(())
}

async fn process_xlsx(
    app: &App,
    target: &ParseTarget,
    bytes: &[u8],
    start_page: i32,
    start_chunks: i64,
) -> Result<(), app::AppError> {
    let include_hidden = std::env::var("XLSX_INCLUDE_HIDDEN")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let ingest = match crate::xlsx::read_xlsx(
        target.project_id.0,
        target.document_id.0,
        target.revision_id.0,
        &target.object_key,
        u64::try_from(target.file_size).unwrap_or(u64::MAX),
        bytes,
        include_hidden,
    ) {
        Ok(ingest) => ingest,
        Err(error) => {
            app.finish_parse_failure(target, error.code(), &error.message())
                .await?;
            return Ok(());
        }
    };
    for warning in &ingest.warnings {
        tracing::warn!(job_id = %target.job_id.0, warning, "xlsx parse warning");
    }

    let total = ingest.pages.len() as i64;
    let mut chunks_done = start_chunks;
    for page in &ingest.pages {
        if page.index <= start_page {
            continue;
        }
        chunks_done += page.chunks.len() as i64;
        app.save_parse_page(
            target,
            &app::ParsePage {
                page: page.index,
                nodes: &page.nodes,
                chunks: &page.chunks,
                evidence: &page.evidence,
                files_total: total,
                chunks_done,
            },
        )
        .await?;
        app.save_checkpoint(
            target.project_id,
            target.job_id.0,
            page.index,
            &json!({"page": page.index, "chunks_done": chunks_done}),
        )
        .await?;
        tracing::info!(
            job_id = %target.job_id.0,
            sheet = %page.sheet,
            sheet_index = page.index,
            sheet_count = total,
            chunks_done,
            "sheet indexed"
        );
    }

    let chunk_count = app::parse::chunk_count(app, target).await?;
    app.finish_parse_success(target, total, chunk_count).await?;
    Ok(())
}

fn page_nodes(target: &ParseTarget, page: i32, text: &str) -> Vec<app::NodeDraft> {
    let document_node = Uuid::new_v5(
        &ID_NAMESPACE,
        format!("document:{}", target.revision_id).as_bytes(),
    );
    let section_node = Uuid::new_v5(
        &ID_NAMESPACE,
        format!("section:{}:{page}", target.revision_id).as_bytes(),
    );
    let mut nodes = Vec::new();
    if page == 1 {
        nodes.push(app::NodeDraft {
            id: document_node,
            revision_id: target.revision_id.0,
            parent_id: None,
            node_type: "DOCUMENT".into(),
            locator: json!({"kind": "pdf"}),
            ordinal: 0,
            text: None,
            meta: json!({"filename": target.object_key}),
        });
    }
    nodes.push(app::NodeDraft {
        id: section_node,
        revision_id: target.revision_id.0,
        parent_id: Some(document_node),
        node_type: "SECTION".into(),
        locator: json!({"kind": "pdf", "page": page}),
        ordinal: page,
        text: Some(text.to_string()),
        meta: json!({"page": page}),
    });

    let mut paragraph_ordinal = 0;
    let mut offset = 0usize;
    for paragraph in text.split("\n\n") {
        let trimmed = paragraph.trim();
        if !trimmed.is_empty() {
            let start = text[offset..]
                .find(trimmed)
                .map(|v| v + offset)
                .unwrap_or(offset);
            nodes.push(app::NodeDraft {
                id: Uuid::new_v5(
                    &ID_NAMESPACE,
                    format!(
                        "paragraph:{}:{page}:{paragraph_ordinal}",
                        target.revision_id
                    )
                    .as_bytes(),
                ),
                revision_id: target.revision_id.0,
                parent_id: Some(section_node),
                node_type: "PARAGRAPH".into(),
                locator: json!({
                    "kind": "pdf",
                    "page": page,
                    "char_range": [start, start + trimmed.len()],
                }),
                ordinal: paragraph_ordinal,
                text: Some(engine_rag::normalize_nfc(trimmed)),
                meta: json!({"page": page}),
            });
            paragraph_ordinal += 1;
        }
        offset += paragraph.len() + 2;
    }
    nodes
}

async fn fail_job(app: &App, job: &app::JobRow, message: &str) {
    if job.kind == "INDEX_BUILD" {
        if let Some(build_id) = job
            .input
            .get("build_id")
            .and_then(Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
        {
            let _ = app
                .fail_index_build(ProjectId(job.project_id), build_id, message)
                .await;
        }
        return;
    }
    if matches!(
        job.kind.as_str(),
        "CODE_SYNC"
            | "CODE_ANALYZE"
            | "IMPACT_ANALYSIS"
            | "TEST_RUN"
            | "AGENT_RUN"
            | "WORKFLOW_RUN"
    ) {
        let code = match job.kind.as_str() {
            "CODE_SYNC" => "CODE_SYNC_FAILED",
            "CODE_ANALYZE" => "CODE_ANALYSIS_FAILED",
            _ => "JOB_FAILED",
        };
        let _ = app
            .finish_job(
                ProjectId(job.project_id),
                job.id,
                JobStatus::Failed,
                vec![JobErrorItem {
                    path: None,
                    code: code.into(),
                    message: message.into(),
                }],
                json!({
                    "phase": "FAILED",
                    "done": 0,
                    "total": 0,
                    "error_kind": job.kind,
                }),
            )
            .await;
        return;
    }
    if let Ok(target) = ParseTarget::from_job(job) {
        let _ = app
            .finish_parse_failure(&target, "E-PARSE-001", message)
            .await;
        return;
    }
    let _ = app
        .finish_job(
            ProjectId(job.project_id),
            job.id,
            JobStatus::Failed,
            vec![JobErrorItem {
                path: None,
                code: "JOB_FAILED".into(),
                message: message.into(),
            }],
            json!({"phase": "FAILED", "done": 0, "total": 0}),
        )
        .await;
}
