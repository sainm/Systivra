use std::collections::{BTreeMap, BTreeSet, HashMap};

use contracts::{
    CreateImpactAssessmentRequest, ErrorCode, ImpactAssessmentAcceptedResponse,
    ImpactAssessmentDraft, ImpactAssessmentListResponse, ImpactAssessmentResponse,
    ImpactAssessmentSummary, ImpactEvidence, ImpactItem, ImpactLineHunk, ImpactOptionsRequest,
    ImpactPathEdge, ImpactRisk, ImpactRiskFactor, ImpactSeedSummary, ImpactStats, ImpactTruncation,
    ImpactUnverifiedItem, JobLinks, JobStatus, NarrateImpactResponse,
};
use domain::{Permission, ProjectId};
use engine_impact::{ImpactEdge, ImpactNode, ImpactOptions, ImpactTraversal, PathEdge, RiskFactor};
use providers::{AiError, AiMessage, AiRequest, AiStatus};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::auth::Authenticated;
use crate::error::AppError;
use crate::App;

impl App {
    /// AI で叙述を足す。AI 未設定なら narrative は None（空文字にしない）。
    pub async fn narrate_impact(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        impact_id: Uuid,
    ) -> Result<NarrateImpactResponse, AppError> {
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFound))?;
        if !Permission::ImpactRun.access_for(role).can_write() {
            return Err(AppError::Forbidden("impact:write がありません".into()));
        }
        let response = self.impact_assessment(auth, project, impact_id).await?;
        let Some(report) = response.report.clone() else {
            return Err(AppError::Validation("report がありません".into()));
        };
        if matches!(self.ai().status(), AiStatus::Unavailable) {
            return Ok(NarrateImpactResponse {
                narrative: None,
                model: None,
                degraded: vec!["ai_unavailable".to_owned()],
            });
        }
        let first = report
            .items
            .first()
            .map(|item| item.label.as_str())
            .unwrap_or("-");
        let user = format!(
            "次の影響分析を 2 文で要約してください。数値を変えない。推測は書かない。\n\n件数: total {} / code {} / screens {} / documents {} / unverified {}\nリスク: {:.3} ({})\n最初の項目: {}",
            report.stats.total,
            report.stats.code,
            report.stats.screens,
            report.stats.documents,
            report.unverified.len(),
            report.risk.score,
            report.risk.level,
            first
        );
        let completion = self
            .ai()
            .complete(&AiRequest {
                model_profile: "fast".to_owned(),
                messages: vec![
                    AiMessage::system(
                        "You summarize impact reports faithfully. Do not invent facts.",
                    ),
                    AiMessage::user(user),
                ],
                tools: Vec::new(),
            })
            .await;
        match completion {
            Ok(response) => {
                let narrative = extract_narrative(&response.text);
                let mut tx = self.store().begin_scoped(project).await?;
                if let Some(row) = self
                    .store()
                    .impact_assessment(&mut tx, project.0, impact_id)
                    .await?
                {
                    if let Some(mut value) = row.report.clone() {
                        value["narrative"] = json!(narrative);
                        value["narrative_source"] = json!("AI");
                        self.store()
                            .finish_impact_assessment(
                                &mut tx,
                                impact_id,
                                &store::ImpactCompletion {
                                    status: &row.status,
                                    report: Some(&value),
                                    engine_version: row.engine_version.as_deref(),
                                    ontology_version: row.ontology_version.as_deref(),
                                    error: None,
                                },
                            )
                            .await?;
                    }
                }
                store::commit(tx).await?;
                Ok(NarrateImpactResponse {
                    narrative: Some(narrative),
                    model: Some(response.model),
                    degraded: vec!["ai_inferred".to_owned()],
                })
            }
            Err(AiError::Unavailable(reason)) => Ok(NarrateImpactResponse {
                narrative: None,
                model: None,
                degraded: vec![format!("ai_unavailable: {reason}")],
            }),
            Err(error) => Err(AppError::Validation(format!("AI 呼び出しに失敗: {error}"))),
        }
    }

    pub async fn enqueue_impact_assessment(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        request: CreateImpactAssessmentRequest,
        trace: Option<String>,
    ) -> Result<ImpactAssessmentAcceptedResponse, AppError> {
        let mut request = request;
        self.require_impact_write(auth, project).await?;
        normalize_impact_request(&mut request)?;
        let input = serde_json::to_value(&request)
            .map_err(|error| AppError::Validation(error.to_string()))?;
        let digest = Sha256::digest(
            serde_json::to_vec(&request)
                .map_err(|error| AppError::Validation(error.to_string()))?,
        );
        let impact_id = Uuid::new_v5(&project.0, digest.as_slice());
        let idempotency_key = format!("impact:{}", hex_lower(digest.as_slice()));

        let mut tx = self.store().begin_scoped(project).await?;
        if request.seed.kind == "ENTITY" {
            for entity_id in &request.seed.entity_ids {
                if self
                    .store()
                    .graph_entity(&mut tx, project.0, *entity_id)
                    .await?
                    .is_none()
                {
                    return Err(AppError::Validation(format!(
                        "entity_id がこのプロジェクトに存在しません: {entity_id}"
                    )));
                }
            }
        } else {
            let head = request
                .seed
                .head_revision_id
                .ok_or_else(|| AppError::Validation("head_revision_id がありません".into()))?;
            let head_revision = self
                .store()
                .code_revision(&mut tx, project.0, head)
                .await?
                .ok_or(AppError::NotFound(ErrorCode::NotFound))?;
            if let Some(base) = request.seed.base_revision_id {
                let base_revision = self
                    .store()
                    .code_revision(&mut tx, project.0, base)
                    .await?
                    .ok_or(AppError::NotFound(ErrorCode::NotFound))?;
                if base_revision.repository_id != head_revision.repository_id {
                    return Err(AppError::Validation(
                        "base/head revision は同じ repository を指定してください".into(),
                    ));
                }
            }
        }

        let draft = self
            .store()
            .create_impact_assessment(
                &mut tx,
                &store::NewImpactAssessment {
                    id: impact_id,
                    project_id: project.0,
                    input: input.clone(),
                },
            )
            .await?;
        let job_input = json!({
            "project_id": project,
            "impact_id": impact_id,
        });
        let mut enqueued = self
            .store()
            .enqueue_job(
                &mut tx,
                &store::NewJob {
                    project_id: project.0,
                    kind: "IMPACT_ANALYSIS".into(),
                    idempotency_key: idempotency_key.clone(),
                    input: job_input.clone(),
                },
            )
            .await?;
        // 冪等キーは「同じ依頼」を指すが、草稿行は作り直されうる（バックアップ復元、
        // 検証用のリセットなど）。その場合、返ってくる job は**前の草稿**を処理した
        // 完了済みの job で、新しい草稿は PENDING のまま取り残される。
        // 呼び出し側には「QUEUED」と答えているのに、実際には何も走らない。
        let terminal = matches!(
            enqueued.job.status.as_str(),
            "SUCCEEDED" | "SUCCEEDED_WITH_ERRORS" | "FAILED" | "CANCELLED" | "TIMEOUT"
        );
        if terminal && draft.status != "SUCCEEDED" {
            let retry_key = format!(
                "{idempotency_key}:{}",
                chrono::Utc::now().timestamp_millis()
            );
            enqueued = self
                .store()
                .enqueue_job(
                    &mut tx,
                    &store::NewJob {
                        project_id: project.0,
                        kind: "IMPACT_ANALYSIS".into(),
                        idempotency_key: retry_key,
                        input: job_input,
                    },
                )
                .await?;
        }
        self.store()
            .append_audit(
                &mut tx,
                &store::AuditDraft {
                    project_id: Some(project.0),
                    actor_type: "USER".into(),
                    actor_id: auth.user_id.to_string(),
                    action: "impact.assessment.enqueue".into(),
                    target_type: Some("impact.assessment_draft".into()),
                    target_id: Some(impact_id.to_string()),
                    input_digest: Some(digest.to_vec()),
                    result: "QUEUED".into(),
                    detail: json!({
                        "job_id": enqueued.job.id,
                        "seed_kind": request.seed.kind,
                    }),
                    trace_id: trace,
                },
            )
            .await?;
        store::commit(tx).await?;
        Ok(ImpactAssessmentAcceptedResponse {
            impact_id: draft.id,
            job_id: enqueued.job.id,
            status: JobStatus::parse(&enqueued.job.status).unwrap_or(JobStatus::Queued),
            links: JobLinks {
                events: format!("/api/v1/jobs/{}/events", enqueued.job.id),
            },
        })
    }

    pub async fn impact_assessment(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        impact_id: Uuid,
    ) -> Result<ImpactAssessmentResponse, AppError> {
        self.require_impact_read(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let row = self
            .store()
            .impact_assessment(&mut tx, project.0, impact_id)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFound))?;
        store::commit(tx).await?;
        impact_response(row)
    }

    pub async fn impact_assessments(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        limit: i64,
    ) -> Result<ImpactAssessmentListResponse, AppError> {
        self.require_impact_read(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let rows = self
            .store()
            .list_impact_assessments(&mut tx, project.0, limit.clamp(1, 100))
            .await?;
        store::commit(tx).await?;
        let items = rows
            .into_iter()
            .map(|row| {
                let report = row.report.as_ref().and_then(|value| {
                    serde_json::from_value::<ImpactAssessmentDraft>(value.clone()).ok()
                });
                ImpactAssessmentSummary {
                    impact_id: row.id,
                    status: row.status,
                    created_at: row.created_at,
                    finished_at: row.finished_at,
                    total: report.as_ref().map(|value| value.stats.total).unwrap_or(0),
                    degraded: report.map(|value| value.degraded).unwrap_or_default(),
                }
            })
            .collect();
        Ok(ImpactAssessmentListResponse { items })
    }

    pub async fn run_impact_assessment_from_job(
        &self,
        project: ProjectId,
        job_id: Uuid,
        input: &Value,
    ) -> Result<ImpactAssessmentDraft, AppError> {
        let impact_id = required_uuid(input, "impact_id")?;
        let mut tx = self.store().begin_scoped(project).await?;
        let draft = self
            .store()
            .impact_assessment(&mut tx, project.0, impact_id)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFound))?;
        self.store()
            .start_impact_assessment(&mut tx, impact_id)
            .await?;
        let request: CreateImpactAssessmentRequest = serde_json::from_value(draft.input.clone())
            .map_err(|error| AppError::Validation(error.to_string()))?;
        let seed_resolution = self
            .resolve_impact_seeds(project, &request, &mut tx)
            .await?;
        if seed_resolution.seed_ids.is_empty() {
            let error = json!({
                "code": "NOT_FOUND_SEED",
                "message": "変更影響のシードを解決できませんでした",
                "degraded": seed_resolution.degraded,
            });
            self.store()
                .finish_impact_assessment(
                    &mut tx,
                    impact_id,
                    &store::ImpactCompletion {
                        status: "FAILED",
                        report: None,
                        engine_version: Some(engine_impact::ENGINE_VERSION),
                        ontology_version: None,
                        error: Some(&error),
                    },
                )
                .await?;
            store::commit(tx).await?;
            return Err(AppError::Validation(
                "NOT_FOUND_SEED: 変更影響のシードを解決できませんでした".into(),
            ));
        }
        let ontology_version = self.store().published_ontology_version(&mut tx).await?;
        let loaded = self
            .load_impact_graph(
                project,
                &seed_resolution.seed_ids,
                &request.options,
                &mut tx,
            )
            .await?;
        let node_index: HashMap<Uuid, store::GraphEntityRow> = loaded
            .nodes
            .iter()
            .map(|node| (node.id, node.clone()))
            .collect();
        let engine_nodes = loaded.nodes.iter().map(impact_node).collect::<Vec<_>>();
        let engine_edges = loaded.edges.iter().map(impact_edge).collect::<Vec<_>>();
        let mut traversal = engine_impact::analyze(
            &engine_nodes,
            &engine_edges,
            &seed_resolution.seed_ids.iter().copied().collect::<Vec<_>>(),
            &impact_options(&request.options),
        );
        traversal.truncation.hubs += loaded.hubs;
        traversal.truncation.budget |= loaded.budget;
        let draft_report = self
            .materialize_impact_draft(
                project,
                impact_id,
                &request,
                &seed_resolution,
                &traversal,
                &node_index,
                &ontology_version,
                &mut tx,
            )
            .await?;
        let report_value = serde_json::to_value(&draft_report)
            .map_err(|error| AppError::Validation(error.to_string()))?;
        let status = if draft_report.degraded.is_empty() && draft_report.unverified.is_empty() {
            "SUCCEEDED"
        } else {
            "PARTIAL"
        };
        self.store()
            .finish_impact_assessment(
                &mut tx,
                impact_id,
                &store::ImpactCompletion {
                    status,
                    report: Some(&report_value),
                    engine_version: Some(&draft_report.engine_version),
                    ontology_version: Some(&draft_report.ontology_version),
                    error: None,
                },
            )
            .await?;
        store::commit(tx).await?;
        tracing::info!(
            job_id = %job_id,
            impact_id = %impact_id,
            items = draft_report.stats.total,
            status,
            "impact assessment draft ready"
        );
        Ok(draft_report)
    }

    pub async fn fail_impact_assessment(
        &self,
        project: ProjectId,
        impact_id: Uuid,
        error: &str,
    ) -> Result<(), AppError> {
        let mut tx = self.store().begin_scoped(project).await?;
        let error_value = json!({"message": error});
        self.store()
            .finish_impact_assessment(
                &mut tx,
                impact_id,
                &store::ImpactCompletion {
                    status: "FAILED",
                    report: None,
                    engine_version: Some(engine_impact::ENGINE_VERSION),
                    ontology_version: None,
                    error: Some(&error_value),
                },
            )
            .await?;
        store::commit(tx).await?;
        Ok(())
    }

    async fn resolve_impact_seeds(
        &self,
        project: ProjectId,
        request: &CreateImpactAssessmentRequest,
        tx: &mut store::Tx<'_>,
    ) -> Result<SeedResolution, AppError> {
        let mut resolved = SeedResolution::default();
        match request.seed.kind.as_str() {
            "ENTITY" => {
                let nodes = self
                    .store()
                    .graph_entities_by_ids(tx, project.0, &request.seed.entity_ids)
                    .await?;
                let node_index = nodes
                    .iter()
                    .map(|node| (node.id, node))
                    .collect::<HashMap<_, _>>();
                for entity_id in &request.seed.entity_ids {
                    let Some(node) = node_index.get(entity_id) else {
                        resolved
                            .degraded
                            .push(format!("entity_not_found:{entity_id}"));
                        continue;
                    };
                    resolved.seed_ids.insert(*entity_id);
                    resolved.seeds.push(ImpactSeedSummary {
                        source: "ENTITY".into(),
                        natural_key: node.natural_key.clone(),
                        kind: node.entity_type.clone(),
                        label: node.name.clone(),
                        entity_id: Some(*entity_id),
                        path: None,
                        line_start: None,
                        line_end: None,
                    });
                }
            }
            "DIFF" => {
                let head = request
                    .seed
                    .head_revision_id
                    .ok_or_else(|| AppError::Validation("head_revision_id がありません".into()))?;
                resolved.base_revision_id = request.seed.base_revision_id;
                resolved.head_revision_id = Some(head);
                let (mut hunks, mut changed, mut diff_degraded) = self
                    .resolve_changed_line_hunks(project, request, tx)
                    .await?;
                resolved.degraded.append(&mut diff_degraded);
                if !request.seed.paths.is_empty() {
                    let requested = request.seed.paths.iter().cloned().collect::<BTreeSet<_>>();
                    let before = changed.len();
                    changed.retain(|path| requested.contains(path));
                    hunks.retain(|hunk| requested.contains(&hunk.path));
                    let ignored = requested
                        .iter()
                        .filter(|path| !changed.contains(*path))
                        .count();
                    if ignored > 0 {
                        resolved
                            .degraded
                            .push(format!("provided_path_not_changed:{ignored}"));
                    }
                    if before == 0 {
                        resolved.degraded.push("no_changed_paths".into());
                    }
                }
                if changed.is_empty() {
                    resolved.degraded.push("no_logical_change".into());
                }
                if hunks.is_empty() && !changed.is_empty() {
                    resolved.degraded.push("line_hunks_unavailable".into());
                }
                resolved.changed_hunks = hunks.clone();
                let changed_paths = changed.iter().cloned().collect::<Vec<_>>();
                let symbols = self
                    .store()
                    .changed_code_symbols(tx, project.0, head, &changed_paths)
                    .await?;
                for path in &changed {
                    if let Some(node) = self
                        .store()
                        .graph_entities_by_natural_keys(
                            tx,
                            project.0,
                            &["Module".into()],
                            std::slice::from_ref(path),
                        )
                        .await?
                        .into_iter()
                        .next()
                    {
                        resolved.seed_ids.insert(node.id);
                        resolved.seeds.push(ImpactSeedSummary {
                            source: "DIFF".into(),
                            natural_key: path.clone(),
                            kind: "Module".into(),
                            label: path.clone(),
                            entity_id: Some(node.id),
                            path: Some(path.clone()),
                            line_start: None,
                            line_end: None,
                        });
                    }
                }
                let mut selected_symbols = BTreeSet::new();
                for hunk in &hunks {
                    let candidates = symbols
                        .iter()
                        .filter(|symbol| {
                            symbol.path == hunk.path
                                && symbol.line_start <= hunk.new_end
                                && symbol.line_end >= hunk.new_start
                        })
                        .collect::<Vec<_>>();
                    let Some(symbol) = candidates.into_iter().min_by_key(|symbol| {
                        (
                            symbol.line_end.saturating_sub(symbol.line_start),
                            symbol.line_start,
                            symbol.kind.as_str(),
                            symbol.fqn.as_str(),
                        )
                    }) else {
                        resolved.degraded.push(format!(
                            "line_not_mapped_to_symbol:{}:{}",
                            hunk.path, hunk.new_start
                        ));
                        continue;
                    };
                    selected_symbols.insert((
                        symbol.path.clone(),
                        symbol.kind.clone(),
                        symbol.fqn.clone(),
                        symbol.simple_name.clone(),
                        symbol.line_start,
                        symbol.line_end,
                        hunk.new_start,
                        hunk.new_end,
                    ));
                }
                if hunks.is_empty() {
                    for symbol in symbols {
                        selected_symbols.insert((
                            symbol.path,
                            symbol.kind,
                            symbol.fqn,
                            symbol.simple_name,
                            symbol.line_start,
                            symbol.line_end,
                            symbol.line_start,
                            symbol.line_end,
                        ));
                    }
                }
                for (
                    path,
                    kind,
                    fqn,
                    simple_name,
                    line_start,
                    line_end,
                    changed_start,
                    changed_end,
                ) in selected_symbols
                {
                    let entity_type = code_kind_to_entity(&kind);
                    let node = self
                        .store()
                        .graph_entities_by_natural_keys(
                            tx,
                            project.0,
                            &[entity_type.into()],
                            std::slice::from_ref(&fqn),
                        )
                        .await?
                        .into_iter()
                        .next();
                    if let Some(node) = node {
                        resolved.seed_ids.insert(node.id);
                        resolved.seeds.push(ImpactSeedSummary {
                            source: "DIFF".into(),
                            natural_key: fqn,
                            kind: entity_type.into(),
                            label: simple_name,
                            entity_id: Some(node.id),
                            path: Some(path),
                            line_start: Some(changed_start.max(line_start)),
                            line_end: Some(changed_end.min(line_end)),
                        });
                    }
                }
                if resolved.seed_ids.is_empty() && !changed.is_empty() {
                    resolved
                        .degraded
                        .push("code_graph_not_available_for_changed_paths".into());
                }
            }
            _ => {
                return Err(AppError::Validation(
                    "seed.kind は ENTITY または DIFF です".into(),
                ));
            }
        }
        resolved.seeds.sort_by(|left, right| {
            (
                left.natural_key.as_str(),
                left.kind.as_str(),
                left.entity_id,
            )
                .cmp(&(
                    right.natural_key.as_str(),
                    right.kind.as_str(),
                    right.entity_id,
                ))
        });
        resolved.seeds.dedup_by_key(|seed| {
            (
                seed.natural_key.clone(),
                seed.kind.clone(),
                seed.entity_id,
                seed.path.clone(),
            )
        });
        Ok(resolved)
    }

    async fn resolve_changed_line_hunks(
        &self,
        project: ProjectId,
        request: &CreateImpactAssessmentRequest,
        tx: &mut store::Tx<'_>,
    ) -> Result<(Vec<ImpactLineHunk>, BTreeSet<String>, Vec<String>), AppError> {
        if let Some(diff) = request.seed.diff_unified.as_deref() {
            let parsed = engine_impact::parse_unified_diff(diff)
                .map_err(|error| AppError::Validation(error.to_string()))?;
            let mut hunks = parsed
                .into_iter()
                .map(|hunk| ImpactLineHunk {
                    path: hunk.path,
                    old_start: hunk.old_start,
                    old_end: hunk.old_end,
                    new_start: hunk.new_start,
                    new_end: hunk.new_end,
                })
                .collect::<Vec<_>>();
            hunks.sort_by(|left, right| {
                (
                    left.path.as_str(),
                    left.new_start,
                    left.old_start,
                    left.new_end,
                    left.old_end,
                )
                    .cmp(&(
                        right.path.as_str(),
                        right.new_start,
                        right.old_start,
                        right.new_end,
                        right.old_end,
                    ))
            });
            hunks.dedup();
            let changed = hunks.iter().map(|hunk| hunk.path.clone()).collect();
            return Ok((hunks, changed, Vec::new()));
        }

        let base = request
            .seed
            .base_revision_id
            .ok_or_else(|| AppError::Validation("base_revision_id がありません".into()))?;
        let head = request
            .seed
            .head_revision_id
            .ok_or_else(|| AppError::Validation("head_revision_id がありません".into()))?;
        let changed = self
            .store()
            .changed_code_paths(tx, project.0, base, head)
            .await?
            .into_iter()
            .collect::<BTreeSet<_>>();
        let base_revision = self
            .store()
            .code_revision(tx, project.0, base)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFound))?;
        let head_revision = self
            .store()
            .code_revision(tx, project.0, head)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFound))?;
        let old_files = self
            .store()
            .code_source_files(tx, project.0, base)
            .await?
            .into_iter()
            .map(|file| (file.path.clone(), file))
            .collect::<HashMap<_, _>>();
        let new_files = self
            .store()
            .code_source_files(tx, project.0, head)
            .await?
            .into_iter()
            .map(|file| (file.path.clone(), file))
            .collect::<HashMap<_, _>>();
        let mut hunks = Vec::new();
        let mut degraded = Vec::new();
        for path in &changed {
            let old_file = old_files.get(path);
            let new_file = new_files.get(path);
            let Some(new_file) = new_file else {
                degraded.push(format!("deleted_path_not_seedable:{path}"));
                continue;
            };
            let whole_new = ImpactLineHunk {
                path: path.clone(),
                old_start: old_file.map(|file| file.loc).unwrap_or(1).max(1),
                old_end: old_file.map(|file| file.loc).unwrap_or(1).max(1),
                new_start: 1,
                new_end: new_file.loc.max(1),
            };
            let Some(old_file) = old_file else {
                hunks.push(whole_new);
                continue;
            };
            let old_text = match self.read_code_snapshot_text(&base_revision, path).await {
                Ok(value) => value,
                Err(error) => {
                    degraded.push(format!("old_text_unavailable:{path}:{error}"));
                    hunks.push(whole_new);
                    continue;
                }
            };
            let new_text = match self.read_code_snapshot_text(&head_revision, path).await {
                Ok(value) => value,
                Err(error) => {
                    degraded.push(format!("new_text_unavailable:{path}:{error}"));
                    hunks.push(whole_new);
                    continue;
                }
            };
            let path_hunks = engine_impact::line_hunks(&old_text, &new_text);
            if path_hunks.is_empty() {
                if old_file.content_hash != new_file.content_hash {
                    degraded.push(format!("line_diff_fallback:{path}"));
                    hunks.push(whole_new);
                }
                continue;
            }
            hunks.extend(path_hunks.into_iter().map(|hunk| ImpactLineHunk {
                path: path.clone(),
                old_start: hunk.old_start,
                old_end: hunk.old_end,
                new_start: hunk.new_start,
                new_end: hunk.new_end,
            }));
        }
        hunks.sort_by(|left, right| {
            (
                left.path.as_str(),
                left.new_start,
                left.old_start,
                left.new_end,
                left.old_end,
            )
                .cmp(&(
                    right.path.as_str(),
                    right.new_start,
                    right.old_start,
                    right.new_end,
                    right.old_end,
                ))
        });
        hunks.dedup();
        Ok((hunks, changed, degraded))
    }

    async fn read_code_snapshot_text(
        &self,
        revision: &store::CodeRevisionRow,
        path: &str,
    ) -> Result<String, AppError> {
        let key = format!("{}/{}", revision.snapshot_key, path);
        let bytes = self.read_object(&key).await?;
        engine_code::text::decide(&bytes)
            .map(|decoded| decoded.text)
            .ok_or_else(|| AppError::Validation("文字コードを確定できません".into()))
    }

    async fn load_impact_graph(
        &self,
        project: ProjectId,
        seeds: &BTreeSet<Uuid>,
        options: &ImpactOptionsRequest,
        tx: &mut store::Tx<'_>,
    ) -> Result<LoadedImpactGraph, AppError> {
        let max_edges = (options.max_items as usize)
            .saturating_mul(40)
            .clamp(1_000, 200_000) as i64;
        let max_nodes = (options.max_items as usize)
            .saturating_mul(20)
            .clamp(1_000, 100_000);
        let seed_ids = seeds.iter().copied().collect::<Vec<_>>();
        let seed_nodes = self
            .store()
            .graph_entities_by_ids(tx, project.0, &seed_ids)
            .await?;
        let mut nodes = BTreeMap::<Uuid, store::GraphEntityRow>::new();
        let mut seen_depth = BTreeMap::<Uuid, i32>::new();
        for node in seed_nodes {
            seen_depth.insert(node.id, 0);
            nodes.insert(node.id, node);
        }
        let mut frontier = seeds
            .iter()
            .filter(|id| seen_depth.contains_key(id))
            .copied()
            .collect::<Vec<_>>();
        let mut edges = BTreeMap::<Uuid, store::GraphEdgeRow>::new();
        let mut hubs = 0i64;
        let mut budget = false;

        for depth in 0..options.max_depth {
            if frontier.is_empty() {
                break;
            }
            let expandable = if depth == 0 {
                frontier.clone()
            } else {
                let degrees = self.store().graph_degrees(tx, project.0, &frontier).await?;
                frontier
                    .iter()
                    .filter_map(|node_id| {
                        let degree = degrees.get(node_id).copied().unwrap_or(0);
                        if degree > options.hub_threshold as i64 {
                            hubs += 1;
                            None
                        } else {
                            Some(*node_id)
                        }
                    })
                    .collect()
            };
            if expandable.is_empty() {
                break;
            }
            let remaining = max_edges as usize - edges.len();
            if remaining == 0 {
                budget = true;
                break;
            }
            let mut incident = self
                .store()
                .graph_incident_edges(tx, project.0, &expandable, remaining as i64 + 1)
                .await?;
            if incident.len() > remaining {
                incident.truncate(remaining);
                budget = true;
            }
            for edge in incident {
                edges.entry(edge.id).or_insert(edge);
            }

            let mut next_frontier = BTreeSet::new();
            let candidate_ids = edges
                .values()
                .flat_map(|edge| [edge.src_id, edge.dst_id])
                .filter(|id| !nodes.contains_key(id))
                .collect::<BTreeSet<_>>();
            let mut candidate_ids = candidate_ids.into_iter().collect::<Vec<_>>();
            if nodes.len() + candidate_ids.len() > max_nodes {
                candidate_ids.truncate(max_nodes.saturating_sub(nodes.len()));
                budget = true;
            }
            for node in self
                .store()
                .graph_entities_by_ids(tx, project.0, &candidate_ids)
                .await?
            {
                seen_depth.entry(node.id).or_insert(depth + 1);
                nodes.entry(node.id).or_insert(node);
            }
            for edge in edges.values() {
                for neighbor in [edge.src_id, edge.dst_id] {
                    if seen_depth.get(&neighbor) == Some(&(depth + 1)) {
                        next_frontier.insert(neighbor);
                    }
                }
            }
            frontier = next_frontier.into_iter().collect();
            if budget {
                break;
            }
        }

        let edges = edges
            .into_values()
            .filter(|edge| nodes.contains_key(&edge.src_id) && nodes.contains_key(&edge.dst_id))
            .collect();
        Ok(LoadedImpactGraph {
            nodes: nodes.into_values().collect(),
            edges,
            hubs,
            budget,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn materialize_impact_draft(
        &self,
        project: ProjectId,
        impact_id: Uuid,
        request: &CreateImpactAssessmentRequest,
        seeds: &SeedResolution,
        traversal: &ImpactTraversal,
        node_index: &HashMap<Uuid, store::GraphEntityRow>,
        ontology_version: &str,
        tx: &mut store::Tx<'_>,
    ) -> Result<ImpactAssessmentDraft, AppError> {
        let mut degraded = seeds.degraded.clone();
        let mut evidence_cache = HashMap::<String, Option<store::ImpactEvidenceRow>>::new();
        let mut items = Vec::new();
        let mut unverified = Vec::new();
        for item in &traversal.items {
            let Some(node) = node_index.get(&item.entity_id) else {
                continue;
            };
            let evidence = self
                .node_evidence(
                    project,
                    node,
                    seeds.head_revision_id,
                    &mut evidence_cache,
                    tx,
                )
                .await?;
            if evidence.is_empty() {
                unverified.push(ImpactUnverifiedItem {
                    entity_id: Some(item.entity_id),
                    natural_key: item.natural_key.clone(),
                    label: item.label.clone(),
                    category: item.category.clone(),
                    why: item.why.clone(),
                    reason: "evidence_missing".into(),
                });
                continue;
            }
            let mut path_edges = Vec::new();
            for edge in &item.path_edges {
                let edge_evidence = if let Some(target) = node_index.get(&edge.dst) {
                    self.node_evidence(
                        project,
                        target,
                        seeds.head_revision_id,
                        &mut evidence_cache,
                        tx,
                    )
                    .await?
                } else {
                    Vec::new()
                };
                path_edges.push(contract_path_edge(edge, edge_evidence.into_iter().next()));
            }
            items.push(ImpactItem {
                item_id: Uuid::new_v5(
                    &impact_id,
                    format!("graph:{}:{}", item.entity_id, item.depth).as_bytes(),
                ),
                level: item.level.clone(),
                category: item.category.clone(),
                direction: item.direction.clone(),
                entity_id: item.entity_id,
                natural_key: item.natural_key.clone(),
                label: item.label.clone(),
                entity_kind: item.entity_kind.clone(),
                depth: item.depth,
                resolution: item.resolution.clone(),
                confidence: item.confidence,
                why: item.why.clone(),
                path: item.path.clone(),
                path_edges,
                evidence,
            });
        }

        if request.options.include_documents {
            let terms = items
                .iter()
                .filter(|item| matches!(item.category.as_str(), "CODE" | "CHANGED"))
                .map(|item| {
                    let base = item.natural_key.rsplit('.').next().unwrap_or(&item.label);
                    base.split('(').next().unwrap_or(base).to_string()
                })
                .filter(|term| term.chars().count() >= 3 && term.chars().count() <= 120)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .take(30)
                .collect::<Vec<_>>();
            let mentions = self
                .store()
                .impact_document_mentions(tx, project.0, &terms, 50)
                .await?;
            if mentions.is_empty() {
                degraded.push("documents_not_indexed_or_no_symbol_mention".into());
            }
            for mention in mentions {
                let item_id = Uuid::new_v5(
                    &impact_id,
                    format!("document:{}:{}", mention.document_id, mention.chunk_id).as_bytes(),
                );
                if items.iter().any(|item| item.item_id == item_id) {
                    continue;
                }
                let source_item = items
                    .iter()
                    .find(|item| {
                        let simple = item
                            .natural_key
                            .rsplit('.')
                            .next()
                            .unwrap_or(&item.label)
                            .split('(')
                            .next()
                            .unwrap_or(&item.label);
                        terms.iter().any(|term| term == simple)
                            && mention
                                .quote
                                .to_lowercase()
                                .contains(&simple.to_lowercase())
                    })
                    .map(|item| item.entity_id);
                items.push(ImpactItem {
                    item_id,
                    level: "L3".into(),
                    category: "DOCUMENT".into(),
                    direction: "shared_resource".into(),
                    entity_id: mention.document_id,
                    natural_key: mention.document_id.to_string(),
                    label: mention.document_title,
                    entity_kind: "Document".into(),
                    depth: 3,
                    resolution: "INFERRED".into(),
                    confidence: 0.6,
                    why: "影響コードのシンボル名が設計書チャンクに一致した（名前一致）".into(),
                    path: source_item.into_iter().collect(),
                    path_edges: Vec::new(),
                    evidence: vec![ImpactEvidence {
                        evidence_id: mention.evidence_id,
                        source_type: format!("DOCUMENT:{}", mention.doc_type),
                        quote: mention.quote,
                        locator: mention.locator,
                        resolution: "INFERRED".into(),
                        confidence: 0.6,
                    }],
                });
            }
        } else {
            degraded.push("documents_excluded_by_request".into());
        }
        degraded.push("tests_not_available".into());
        if !unverified.is_empty() {
            degraded.push(format!("unverified_items:{}", unverified.len()));
        }

        let mut stats = impact_stats(&items);
        if items.is_empty() {
            stats.total = 0;
        }
        let truncated = ImpactTruncation {
            hubs: traversal.truncation.hubs,
            depth: traversal.truncation.depth,
            budget: traversal.truncation.budget,
            no_logical_change: traversal.truncation.no_logical_change,
        };
        let risk = adjusted_risk(&traversal.risk.factors, &stats, &truncated);
        degraded.sort();
        degraded.dedup();
        items.sort_by(|left, right| {
            (
                left.depth,
                left.category.as_str(),
                left.natural_key.as_str(),
                left.item_id,
            )
                .cmp(&(
                    right.depth,
                    right.category.as_str(),
                    right.natural_key.as_str(),
                    right.item_id,
                ))
        });
        unverified.sort_by(|left, right| {
            (
                left.category.as_str(),
                left.natural_key.as_str(),
                left.entity_id,
            )
                .cmp(&(
                    right.category.as_str(),
                    right.natural_key.as_str(),
                    right.entity_id,
                ))
        });

        Ok(ImpactAssessmentDraft {
            seeds: seeds.seeds.clone(),
            changed_hunks: seeds.changed_hunks.clone(),
            items,
            unverified,
            stats,
            risk,
            truncated,
            degraded,
            base_revision_id: seeds.base_revision_id,
            head_revision_id: seeds.head_revision_id,
            engine_version: engine_impact::ENGINE_VERSION.into(),
            ontology_version: ontology_version.into(),
            approval_state: "DRAFT".into(),
            code_modified: false,
            tests_executed: false,
            narrative: None,
            narrative_source: None,
        })
    }

    async fn node_evidence(
        &self,
        project: ProjectId,
        node: &store::GraphEntityRow,
        revision_id: Option<Uuid>,
        cache: &mut HashMap<String, Option<store::ImpactEvidenceRow>>,
        tx: &mut store::Tx<'_>,
    ) -> Result<Vec<ImpactEvidence>, AppError> {
        let key = format!(
            "{}:{}:{}",
            node.natural_key,
            revision_id
                .map(|value| value.to_string())
                .unwrap_or_default(),
            node.properties
                .get("document_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
        );
        let evidence = if let Some(value) = cache.get(&key) {
            value.clone()
        } else {
            let mut value = self
                .store()
                .impact_evidence_for_entity(tx, project.0, &node.natural_key, revision_id)
                .await?;
            if value.is_none() {
                if let Some(document_id) = node
                    .properties
                    .get("document_id")
                    .and_then(Value::as_str)
                    .and_then(|value| Uuid::parse_str(value).ok())
                {
                    value = self
                        .store()
                        .impact_evidence_for_document_entity(tx, project.0, document_id, &node.name)
                        .await?;
                }
            }
            cache.insert(key, value.clone());
            value
        };
        Ok(evidence
            .map(|evidence| {
                vec![ImpactEvidence {
                    evidence_id: evidence.evidence_id,
                    source_type: evidence
                        .relation
                        .map(|relation| format!("{}:{relation}", evidence.source_type))
                        .unwrap_or(evidence.source_type),
                    quote: evidence.quote,
                    locator: evidence.locator,
                    resolution: evidence.resolution,
                    confidence: evidence.confidence,
                }]
            })
            .unwrap_or_default())
    }

    async fn require_impact_read(
        &self,
        auth: &Authenticated,
        project: ProjectId,
    ) -> Result<(), AppError> {
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFound))?;
        if !Permission::ImpactRun.access_for(role).can_read() {
            return Err(AppError::Forbidden("impact:read がありません".into()));
        }
        Ok(())
    }

    async fn require_impact_write(
        &self,
        auth: &Authenticated,
        project: ProjectId,
    ) -> Result<(), AppError> {
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFound))?;
        if !Permission::ImpactRun.access_for(role).can_write() {
            return Err(AppError::Forbidden("impact:write がありません".into()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
struct SeedResolution {
    seed_ids: BTreeSet<Uuid>,
    seeds: Vec<ImpactSeedSummary>,
    changed_hunks: Vec<ImpactLineHunk>,
    degraded: Vec<String>,
    base_revision_id: Option<Uuid>,
    head_revision_id: Option<Uuid>,
}

#[derive(Debug)]
struct LoadedImpactGraph {
    nodes: Vec<store::GraphEntityRow>,
    edges: Vec<store::GraphEdgeRow>,
    hubs: i64,
    budget: bool,
}

fn normalize_impact_request(request: &mut CreateImpactAssessmentRequest) -> Result<(), AppError> {
    request.seed.kind = request.seed.kind.trim().to_ascii_uppercase();
    if !matches!(request.seed.kind.as_str(), "ENTITY" | "DIFF") {
        return Err(AppError::Validation(
            "seed.kind は ENTITY または DIFF です".into(),
        ));
    }
    request.options.max_depth = request.options.max_depth.clamp(1, 10);
    request.options.hub_threshold = request.options.hub_threshold.clamp(1, 100_000);
    request.options.max_items = request.options.max_items.clamp(1, 10_000);
    if request.seed.kind == "ENTITY" {
        if request.seed.entity_ids.is_empty() {
            return Err(AppError::Validation(
                "entity_ids を 1 件以上指定してください".into(),
            ));
        }
        if request.seed.entity_ids.len() > 100 {
            return Err(AppError::Validation("entity_ids は 100 件以下です".into()));
        }
        request.seed.entity_ids.sort();
        request.seed.entity_ids.dedup();
        request.seed.paths.clear();
    } else {
        if request.seed.head_revision_id.is_none() {
            return Err(AppError::Validation(
                "DIFF には head_revision_id が必要です".into(),
            ));
        }
        request.seed.diff_unified = request
            .seed
            .diff_unified
            .take()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        if request.seed.diff_unified.is_none() && request.seed.base_revision_id.is_none() {
            return Err(AppError::Validation(
                "DIFF には base_revision_id または diff_unified が必要です".into(),
            ));
        }
        if request
            .seed
            .diff_unified
            .as_ref()
            .is_some_and(|value| value.len() > 20 * 1024 * 1024)
        {
            return Err(AppError::Validation(
                "diff_unified は 20 MiB 以下です".into(),
            ));
        }
        request.seed.entity_ids.clear();
        request.seed.paths.sort();
        request.seed.paths.dedup();
    }
    Ok(())
}

fn impact_options(request: &ImpactOptionsRequest) -> ImpactOptions {
    ImpactOptions {
        max_depth: request.max_depth,
        hub_threshold: request.hub_threshold,
        max_items: request.max_items,
    }
}

fn impact_node(node: &store::GraphEntityRow) -> ImpactNode {
    ImpactNode {
        id: node.id,
        kind: node.entity_type.clone(),
        natural_key: node.natural_key.clone(),
        name: node.name.clone(),
        resolution: node.resolution.clone(),
        confidence: 0.9,
        properties: node.properties.clone(),
    }
}

fn impact_edge(edge: &store::GraphEdgeRow) -> ImpactEdge {
    ImpactEdge {
        id: edge.id,
        src: edge.src_id,
        dst: edge.dst_id,
        relation: edge.rel_type.clone(),
        resolution: edge.resolution.clone(),
        confidence: edge.confidence,
    }
}

fn contract_path_edge(edge: &PathEdge, evidence: Option<ImpactEvidence>) -> ImpactPathEdge {
    ImpactPathEdge {
        edge_id: edge.edge_id,
        src: edge.src,
        dst: edge.dst,
        relation: edge.relation.clone(),
        resolution: edge.resolution.clone(),
        confidence: edge.confidence,
        evidence,
    }
}

fn impact_stats(items: &[ImpactItem]) -> ImpactStats {
    let mut stats = ImpactStats {
        total: items.len() as i64,
        ..ImpactStats::default()
    };
    for item in items {
        match item.category.as_str() {
            "CODE" | "CHANGED" => stats.code += 1,
            "SCREEN" => stats.screens += 1,
            "DOCUMENT" => stats.documents += 1,
            "DATA" => stats.data += 1,
            _ => {}
        }
        match item.resolution.as_str() {
            "RESOLVED" => stats.resolved += 1,
            "PARTIAL" => stats.partial += 1,
            "INFERRED" => stats.inferred += 1,
            "CONFLICT" => stats.conflict += 1,
            _ => stats.unresolved += 1,
        }
    }
    stats
}

fn adjusted_risk(
    factors: &[RiskFactor],
    stats: &ImpactStats,
    truncated: &ImpactTruncation,
) -> ImpactRisk {
    let breadth = (stats.total as f32 / 50.0).min(1.0);
    let uncertainty = if stats.total == 0 {
        1.0
    } else {
        (stats.inferred + stats.unresolved + stats.conflict) as f32 / stats.total as f32
    };
    let truncation = if truncated.depth || truncated.budget || truncated.hubs > 0 {
        1.0
    } else {
        0.0
    };
    let mut result = factors
        .iter()
        .map(|factor| ImpactRiskFactor {
            name: factor.name.clone(),
            weight: factor.weight,
            value: match factor.name.as_str() {
                "impact_breadth" => breadth,
                "uncertainty_ratio" => uncertainty,
                "truncation" => truncation,
                _ => factor.value,
            },
        })
        .collect::<Vec<_>>();
    if result.is_empty() {
        result = vec![
            ImpactRiskFactor {
                name: "impact_breadth".into(),
                weight: 0.35,
                value: breadth,
            },
            ImpactRiskFactor {
                name: "uncertainty_ratio".into(),
                weight: 0.30,
                value: uncertainty,
            },
            ImpactRiskFactor {
                name: "truncation".into(),
                weight: 0.15,
                value: truncation,
            },
        ];
    }
    let score = result
        .iter()
        .map(|factor| factor.weight * factor.value)
        .sum::<f32>()
        .clamp(0.0, 1.0);
    let level = if score >= 0.70 {
        "HIGH"
    } else if score >= 0.40 {
        "MEDIUM"
    } else {
        "LOW"
    };
    ImpactRisk {
        score: (score * 1000.0).round() / 1000.0,
        level: level.into(),
        factors: result,
    }
}

fn impact_response(row: store::ImpactAssessmentRow) -> Result<ImpactAssessmentResponse, AppError> {
    let report = row
        .report
        .map(|value| {
            serde_json::from_value::<ImpactAssessmentDraft>(value)
                .map_err(|error| AppError::Validation(error.to_string()))
        })
        .transpose()?;
    Ok(ImpactAssessmentResponse {
        impact_id: row.id,
        status: row.status,
        input: row.input,
        report,
        error: row.error,
        created_at: row.created_at,
        started_at: row.started_at,
        finished_at: row.finished_at,
    })
}

fn code_kind_to_entity(kind: &str) -> &'static str {
    match kind {
        "CLASS" => "Class",
        "METHOD" => "Method",
        "FIELD" => "Field",
        "INTERFACE" => "Interface",
        "ENUM" => "Class",
        _ => "Method",
    }
}

fn required_uuid(input: &Value, name: &str) -> Result<Uuid, AppError> {
    input
        .get(name)
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(|| AppError::Validation(format!("{name} がありません")))
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Impact Draft を Markdown にする。値は API のレポートと同じ。
impl App {
    pub async fn impact_markdown(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        impact_id: Uuid,
    ) -> Result<String, AppError> {
        let response = self.impact_assessment(auth, project, impact_id).await?;
        let Some(report) = response.report else {
            return Err(AppError::Validation("report がありません".into()));
        };
        let value = serde_json::to_value(&report)
            .map_err(|error| AppError::Validation(error.to_string()))?;
        Ok(render_impact_markdown(
            response.impact_id,
            &response.status,
            &value,
        ))
    }

    pub async fn impact_pdf(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        impact_id: Uuid,
    ) -> Result<Vec<u8>, AppError> {
        let markdown = self.impact_markdown(auth, project, impact_id).await?;
        crate::report::render_text_pdf("Systivra 変更影響レポート", &markdown)
    }
}

fn render_impact_markdown(impact_id: Uuid, status: &str, report: &serde_json::Value) -> String {
    let text = |value: &serde_json::Value, key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or("-")
            .to_owned()
    };
    let mut out = String::new();
    out.push_str("# 変更影響レポート\n\n");
    out.push_str(&format!("- 影響 ID: {impact_id}\n- 状態: {status}\n"));
    out.push_str(&format!("- 承認状態: {}\n", text(report, "approval_state")));
    out.push_str(&format!(
        "- コード変更: {} / テスト実行: {}\n",
        report
            .get("code_modified")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        report
            .get("tests_executed")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    ));
    out.push('\n');

    if let Some(narrative) = report.get("narrative").and_then(serde_json::Value::as_str) {
        out.push_str("## AI の要約（INFERRED）\n\n");
        out.push_str(narrative);
        out.push_str("\n\n> AI の推定です。件数・リスク・証拠は下の表を正とします。\n\n");
    }

    if let Some(stats) = report.get("stats") {
        out.push_str(
            "## 件数\n\n| 合計 | コード | 画面 | 設計書 | データ |\n|---|---|---|---|---|\n",
        );
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n\n",
            stats
                .get("total")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0),
            stats
                .get("code")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0),
            stats
                .get("screens")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0),
            stats
                .get("documents")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0),
            stats
                .get("data")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0),
        ));
    }

    if let Some(risk) = report.get("risk") {
        out.push_str(&format!(
            "リスク: {} ({})\n\n",
            risk.get("score")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0),
            risk.get("level")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("-"),
        ));
    }

    if let Some(seeds) = report.get("seeds").and_then(serde_json::Value::as_array) {
        out.push_str(&format!("## シード（{} 件）\n\n", seeds.len()));
        for seed in seeds {
            out.push_str(&format!(
                "- {} `{}` {}{}\n",
                text(seed, "kind"),
                text(seed, "natural_key"),
                text(seed, "path"),
                seed.get("line_start")
                    .and_then(serde_json::Value::as_i64)
                    .map(|line| format!(" L{line}"))
                    .unwrap_or_default(),
            ));
        }
        out.push('\n');
    }
    if let Some(hunks) = report
        .get("changed_hunks")
        .and_then(serde_json::Value::as_array)
    {
        if !hunks.is_empty() {
            out.push_str("## 変更行\n\n");
            for hunk in hunks {
                out.push_str(&format!(
                    "- `{}`: new {}-{}, old {}-{}\n",
                    text(hunk, "path"),
                    hunk.get("new_start")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(0),
                    hunk.get("new_end")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(0),
                    hunk.get("old_start")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(0),
                    hunk.get("old_end")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(0),
                ));
            }
            out.push('\n');
        }
    }

    let items = report
        .get("items")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    out.push_str(&format!("## 影響項目（{} 件）\n\n", items.len()));
    if items.is_empty() {
        out.push_str("証拠のある影響項目はありません。\n\n");
    } else {
        out.push_str("| # | 種別 | 名前 | 深さ | 解決状態 | 信頼度 | 理由 |\n|---|---|---|---|---|---|---|\n");
        for (index, item) in items.iter().enumerate() {
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} |\n",
                index + 1,
                md(&text(item, "category")),
                md(&text(item, "label")),
                item.get("depth")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(0),
                text(item, "resolution"),
                item.get("confidence")
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or(0.0),
                md(&text(item, "why")),
            ));
        }
        out.push('\n');
        out.push_str("### 証拠\n\n");
        for (index, item) in items.iter().enumerate() {
            let evidence = item
                .get("evidence")
                .and_then(serde_json::Value::as_array)
                .and_then(|list| list.first());
            let Some(evidence) = evidence else { continue };
            let quote = evidence
                .get("quote")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let locator = evidence
                .get("locator")
                .map(|value| value.to_string())
                .unwrap_or_else(|| "{}".to_owned());
            out.push_str(&format!(
                "{}. 「{}」 — `{}`\n",
                index + 1,
                md(quote),
                md(&locator)
            ));
        }
        out.push('\n');
    }

    if let Some(unverified) = report
        .get("unverified")
        .and_then(serde_json::Value::as_array)
    {
        if !unverified.is_empty() {
            out.push_str(&format!("## 未検証（{} 件）\n\n", unverified.len()));
            for item in unverified {
                out.push_str(&format!(
                    "- {} — {}\n",
                    md(&text(item, "label")),
                    md(&text(item, "reason"))
                ));
            }
            out.push('\n');
        }
    }

    if let Some(truncated) = report.get("truncated") {
        out.push_str(&format!("## 打切り\n\n`{}`\n\n", truncated));
    }
    if let Some(degraded) = report.get("degraded").and_then(serde_json::Value::as_array) {
        if !degraded.is_empty() {
            let list: Vec<String> = degraded
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect();
            out.push_str(&format!("## 降格\n\n{}\n\n", list.join(", ")));
        }
    }
    out
}

fn md(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

fn extract_narrative(text: &str) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        if let Some(narrative) = value.get("narrative").and_then(Value::as_str) {
            return narrative.chars().take(800).collect();
        }
    }
    text.chars().take(800).collect()
}
