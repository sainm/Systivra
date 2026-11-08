//! Phase 6 のユースケース編成。
//!
//! 境界: 権限 → `begin_scoped` → 検証 → 書込 → audit → commit。
//! Q-01 からの派生（`derive_test_plan`）は **test_run を絶対に作らない**（R-16）。
//! Runner は未接続なので、実行結果は `:import` で `execution_source` 付きで
//! 受け取る。PASSED をこの層で生成することはない。

use std::collections::{BTreeMap, BTreeSet};

use contracts::{
    ApproveTestCaseResponse, CoverageView, CreateDefectRequest, CreateTestRunRequest,
    CreateTestRunResponse, DefectCandidateView, DeriveTestPlanRequest, DeriveTestPlanResponse,
    ExtractRulesRequest, ExtractRulesResponse, ImportExecutionsRequest, ImportExecutionsResponse,
    RecomputeCoverageRequest, TestArtifact, TestCaseDsl, TestCaseView, TestDataView,
    TestEnvironment, TestExecutionView, TestOverviewResponse, TestPlanScope, TestPlanView,
    TestRunView, ValidationRuleView,
};
use domain::{Permission, ProjectId};
use engine_test as engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{App, AppError, Authenticated};

fn default_env() -> TestEnvironment {
    TestEnvironment {
        id: "env-local-01".to_owned(),
        base_url: "http://127.0.0.1:18080/app".to_owned(),
        guard_kind: "STAGING".to_owned(),
        allowed_origins: vec!["127.0.0.1".to_owned(), "localhost".to_owned()],
        parallel: 4,
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from_digit((byte >> 4) as u32, 16).unwrap_or('0'));
        out.push(char::from_digit((byte & 0x0f) as u32, 16).unwrap_or('0'));
    }
    out
}

fn stable_uuid(project: ProjectId, parts: &[&str]) -> Uuid {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0u8]);
    }
    let digest = hasher.finalize();
    Uuid::new_v5(&project.0, &digest[..16])
}

fn dsl_from(value: &Value) -> TestCaseDsl {
    serde_json::from_value(value.clone()).unwrap_or_default()
}

fn artifacts_from(value: &Value) -> Vec<TestArtifact> {
    serde_json::from_value(value.clone()).unwrap_or_default()
}

fn rule_view(row: &store::ValidationRuleRow) -> ValidationRuleView {
    ValidationRuleView {
        id: row.id,
        field_ref: row.field_ref.clone(),
        kind: row.kind.clone(),
        condition: row.condition.clone(),
        source: row.source.clone(),
        resolution: row.resolution.clone(),
        evidence_ids: row.evidence_ids.clone(),
        created_at: row.created_at,
    }
}

fn plan_view(row: &store::TestPlanRow) -> TestPlanView {
    TestPlanView {
        id: row.id,
        code: row.code.clone(),
        name: row.name.clone(),
        status: row.status.clone(),
        origin: row.origin.clone(),
        impact_id: row.impact_id,
        scope: serde_json::from_value(row.scope.clone()).unwrap_or_default(),
        created_at: row.created_at,
    }
}

fn case_view(row: &store::TestCaseRow, env: &TestEnvironment) -> TestCaseView {
    let dsl = dsl_from(&row.dsl);
    TestCaseView {
        id: row.id,
        scenario_id: row.scenario_id,
        code: row.code.clone(),
        title: row.title.clone(),
        status: row.status.clone(),
        validation_errors: engine::validate_dsl(&dsl, env),
        dsl,
        covers_rules: row.covers_rules.clone(),
        covers_impacts: row.covers_impacts.clone(),
        covers_screens: row.covers_screens.clone(),
        evidence_ids: row.evidence_ids.clone(),
        destructive: row.destructive,
    }
}

fn run_view(row: &store::TestRunRow) -> TestRunView {
    TestRunView {
        id: row.id,
        name: row.name.clone(),
        status: row.status.clone(),
        environment_id: row.environment_id.clone(),
        guard_kind: row.guard_kind.clone(),
        lock_acquired: row.lock_acquired,
        case_count: row.case_count,
        job_id: row.job_id,
        started_at: row.started_at,
        finished_at: row.finished_at,
        created_at: row.created_at,
    }
}

fn execution_view(row: &store::TestExecutionRow) -> TestExecutionView {
    TestExecutionView {
        id: row.id,
        test_run_id: row.test_run_id,
        test_case_id: row.test_case_id,
        attempt: row.attempt,
        status: row.status.clone(),
        execution_source: row.execution_source.clone(),
        failure_kind: row.failure_kind.clone(),
        failure_step: row.failure_step,
        fingerprint: row.fingerprint.clone(),
        artifacts: artifacts_from(&row.artifacts),
    }
}

fn defect_view(row: &store::DefectRow) -> DefectCandidateView {
    DefectCandidateView {
        id: row.id,
        fingerprint: row.fingerprint.clone(),
        title: row.title.clone(),
        severity: row.severity.clone(),
        status: row.status.clone(),
        recurrence: row.recurrence,
        execution_id: row.execution_id,
        impact_item_id: row.impact_item_id,
        created_at: row.created_at,
    }
}

fn coverage_view(row: &store::CoverageSnapshotRow) -> CoverageView {
    CoverageView {
        test_run_id: row.test_run_id,
        computed_at: row.computed_at,
        detail: serde_json::from_value(row.detail.clone()).unwrap_or_default(),
    }
}

/// Requirement がルールから推定されたものなら、その旨を被覆の detail に残す。
/// 数字だけ見て「要件を検証した」と誤読されないようにする。
fn annotate_requirement_source(result: &mut engine::CoverageResult, graph_available: bool) {
    if !graph_available {
        return;
    }
    result
        .requirement
        .degraded
        .push("REQUIREMENT_INFERRED_FROM_RULE".to_owned());
    result.requirement.degraded.sort();
    result.requirement.degraded.dedup();
    result
        .degraded
        .push("REQUIREMENT_INFERRED_FROM_RULE".to_owned());
    result.degraded.sort();
    result.degraded.dedup();
}

/// run が計画に紐づくなら、その計画のケースだけを対象にする。
fn in_plan(row: &store::TestCaseRow, plan_id: Option<Uuid>) -> bool {
    plan_id.map(|plan| row.plan_id == plan).unwrap_or(true)
}

fn has_covers(row: &store::TestCaseRow) -> bool {
    !row.covers_rules.is_empty() || !row.covers_impacts.is_empty() || !row.covers_screens.is_empty()
}

/// グラフの `TestCase -[:VERIFIES]-> Requirement` 辺。無い場合は縮退する。
#[derive(Debug, Clone, Default)]
struct CoverageGraph {
    requirement_ids: Vec<Uuid>,
    /// `(test_case_id, requirement_entity_id)`
    edges: Vec<(Uuid, Uuid)>,
    available: bool,
}

impl CoverageGraph {
    fn new(requirement_ids: Vec<Uuid>, edges: Vec<(Uuid, Uuid)>) -> Self {
        let available = !requirement_ids.is_empty() && !edges.is_empty();
        Self {
            requirement_ids,
            edges,
            available,
        }
    }
}

fn coverage_inputs(
    cases: &[store::TestCaseRow],
    executions: &[store::TestExecutionRow],
    run_id: Uuid,
    graph: &CoverageGraph,
) -> (engine::CoverageTotals, Vec<engine::CoverageCase>) {
    let mut rule_ids = BTreeSet::new();
    let mut screen_ids = BTreeSet::new();
    let mut impact_ids = BTreeSet::new();
    let mut regression_case_ids = BTreeSet::new();
    for case in cases {
        rule_ids.extend(case.covers_rules.iter().copied());
        screen_ids.extend(case.covers_screens.iter().cloned());
        impact_ids.extend(case.covers_impacts.iter().copied());
        if case.status == "APPROVED" && has_covers(case) {
            regression_case_ids.insert(case.id);
        }
    }

    let by_case: BTreeMap<Uuid, Vec<&store::TestExecutionRow>> = executions
        .iter()
        .filter(|e| e.test_run_id == run_id)
        .fold(BTreeMap::new(), |mut map, execution| {
            map.entry(execution.test_case_id)
                .or_default()
                .push(execution);
            map
        });

    let coverage_cases: Vec<engine::CoverageCase> = cases
        .iter()
        .map(|case| {
            let latest = by_case
                .get(&case.id)
                .and_then(|rows| rows.iter().max_by_key(|row| row.attempt).copied());
            let status = latest.map(|row| row.status.as_str()).unwrap_or("");
            engine::CoverageCase {
                case_id: case.id,
                status: case.status.clone(),
                covers_rules: case.covers_rules.clone(),
                covers_impacts: case.covers_impacts.clone(),
                covers_screens: case.covers_screens.clone(),
                executed: latest.is_some() && status != "PENDING",
                passed: status == "PASSED",
                skipped: matches!(status, "SKIPPED" | "BLOCKED"),
            }
        })
        .collect();

    // Requirement はグラフの VERIFIES 辺から取る。辺が無いときだけ縮退し、
    // 近似値で埋めない（detail/09 §9.2）。
    let passed: BTreeSet<Uuid> = coverage_cases
        .iter()
        .filter(|case| case.executed && case.passed && !case.skipped)
        .map(|case| case.case_id)
        .collect();
    let requirement_covered: Vec<Uuid> = if graph.available {
        graph
            .edges
            .iter()
            .filter(|(case_id, _)| passed.contains(case_id))
            .map(|(_, requirement_id)| *requirement_id)
            .collect()
    } else {
        Vec::new()
    };
    let totals = engine::CoverageTotals {
        rule_ids: rule_ids.into_iter().collect(),
        screen_ids: screen_ids.into_iter().collect(),
        impact_ids: impact_ids.into_iter().collect(),
        requirement_ids: if graph.available {
            graph.requirement_ids.clone()
        } else {
            Vec::new()
        },
        regression_case_ids: regression_case_ids.into_iter().collect(),
        graph_verifies_available: graph.available,
        requirement_covered,
    };
    (totals, coverage_cases)
}

impl App {
    async fn coverage_graph(
        &self,
        tx: &mut store::Tx<'_>,
        project_id: Uuid,
    ) -> Result<CoverageGraph, AppError> {
        let requirement_ids = self.store().test_requirement_ids(tx, project_id).await?;
        let edges = self.store().test_verifies_edges(tx, project_id).await?;
        Ok(CoverageGraph::new(requirement_ids, edges))
    }

    async fn require_test_read(
        &self,
        auth: &Authenticated,
        project: ProjectId,
    ) -> Result<(), AppError> {
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
        if !Permission::TestAssetEdit.access_for(role).can_read() {
            return Err(AppError::Forbidden("test:read がありません".into()));
        }
        Ok(())
    }

    async fn require_test_write(
        &self,
        auth: &Authenticated,
        project: ProjectId,
    ) -> Result<(), AppError> {
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
        if !Permission::TestAssetEdit.access_for(role).can_write() {
            return Err(AppError::Forbidden("test:write がありません".into()));
        }
        Ok(())
    }

    pub async fn require_test_run(
        &self,
        auth: &Authenticated,
        project: ProjectId,
    ) -> Result<(), AppError> {
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
        if !Permission::TestRun.access_for(role).can_write() {
            return Err(AppError::Forbidden("test:execute がありません".into()));
        }
        Ok(())
    }

    async fn require_test_approve(
        &self,
        auth: &Authenticated,
        project: ProjectId,
    ) -> Result<(), AppError> {
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
        if !Permission::TestCaseApprove.access_for(role).can_write() {
            return Err(AppError::Forbidden("test:approve がありません".into()));
        }
        Ok(())
    }

    fn audit_draft(
        project: ProjectId,
        auth: &Authenticated,
        action: &str,
        target_type: &str,
        target_id: Uuid,
        detail: Value,
        trace: Option<String>,
    ) -> store::AuditDraft {
        store::AuditDraft {
            project_id: Some(project.0),
            actor_type: "USER".into(),
            actor_id: auth.user_id.to_string(),
            action: action.into(),
            target_type: Some(target_type.into()),
            target_id: Some(target_id.to_string()),
            input_digest: None,
            result: "OK".into(),
            detail,
            trace_id: trace,
        }
    }

    // ── ルール抽出 ────────────────────────────────────────

    pub async fn extract_test_rules(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        request: ExtractRulesRequest,
        trace: Option<String>,
    ) -> Result<ExtractRulesResponse, AppError> {
        self.require_test_write(auth, project).await?;
        if request.sources.is_empty() {
            return Err(AppError::Validation("sources が空です".into()));
        }
        let inputs: Vec<engine::RuleInput> = request
            .sources
            .iter()
            .map(|source| engine::RuleInput {
                kind: source.kind.clone(),
                field_ref: source.field_ref.clone(),
                text: source.text.clone(),
                evidence_ids: source.evidence_ids.clone(),
            })
            .collect();
        let extraction = engine::extract_rules(project.0, &inputs);
        let new_rules: Vec<store::NewValidationRule> = extraction
            .rules
            .iter()
            .map(|rule| store::NewValidationRule {
                id: rule.id,
                project_id: project.0,
                field_ref: rule.field_ref.clone(),
                kind: rule.kind.clone(),
                condition: rule.condition.clone(),
                source: rule.source.clone(),
                evidence_ids: rule.evidence_ids.clone(),
                resolution: rule.resolution.clone(),
                condition_key: rule.condition_key.clone(),
            })
            .collect();

        let mut tx = self.store().begin_scoped(project).await?;
        self.store()
            .insert_validation_rules(&mut tx, &new_rules)
            .await?;
        let rows = self
            .store()
            .list_validation_rules(&mut tx, project.0)
            .await?;
        self.store()
            .append_audit(
                &mut tx,
                &Self::audit_draft(
                    project,
                    auth,
                    "test.rules.extract",
                    "test.validation_rule",
                    project.0,
                    json!({
                        "sources": request.sources.len(),
                        "parsed": extraction.rules.len(),
                        "conflicts": extraction.conflicts,
                        "unparsed": extraction.unparsed,
                    }),
                    trace,
                ),
            )
            .await?;
        store::commit(tx).await?;

        Ok(ExtractRulesResponse {
            rules: rows.iter().map(rule_view).collect(),
            conflicts: extraction.conflicts,
            unparsed: extraction.unparsed,
            degraded: extraction.degraded(),
        })
    }

    // ── Q-01 → 計画（実行しない） ──────────────────────────

    pub async fn derive_test_plan(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        request: DeriveTestPlanRequest,
        trace: Option<String>,
    ) -> Result<DeriveTestPlanResponse, AppError> {
        self.require_test_write(auth, project).await?;
        // Q-01 経由の既定は 20 件（detail/08 §6.2）。T-04 の詳細生成では
        // 負荷試験用に 200 件まで許す。上限を超える値は黙って切り詰める。
        let max_drafts = request.max_drafts.unwrap_or(20).clamp(1, 200) as usize;

        let mut tx = self.store().begin_scoped(project).await?;
        let rule_rows = self
            .store()
            .list_validation_rules(&mut tx, project.0)
            .await?;
        let rules: Vec<engine::ExtractedRule> = rule_rows
            .iter()
            .map(|row| engine::ExtractedRule {
                id: row.id,
                field_ref: row.field_ref.clone(),
                kind: row.kind.clone(),
                condition: row.condition.clone(),
                source: row.source.clone(),
                resolution: row.resolution.clone(),
                evidence_ids: row.evidence_ids.clone(),
                condition_key: format!("{}:{}", row.kind, row.condition),
                engine_version: engine::ENGINE_VERSION.to_owned(),
            })
            .collect();

        let mut impact_items = Vec::new();
        let mut degraded = Vec::new();
        if let Some(impact_id) = request.impact_id {
            let draft = self
                .store()
                .impact_assessment(&mut tx, project.0, impact_id)
                .await?
                .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
            let Some(report) = draft.report else {
                return Err(AppError::Validation(
                    "Impact Draft に report がありません".into(),
                ));
            };
            let items = report
                .get("items")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for item in items {
                let Some(id) = item
                    .get("item_id")
                    .and_then(Value::as_str)
                    .and_then(|v| Uuid::parse_str(v).ok())
                else {
                    continue;
                };
                let label = item
                    .get("label")
                    .and_then(Value::as_str)
                    .unwrap_or("(unnamed)")
                    .to_owned();
                let category = item
                    .get("category")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                let screen = category
                    .to_ascii_uppercase()
                    .contains("SCREEN")
                    .then(|| label.clone());
                impact_items.push(engine::ImpactItemSeed {
                    id,
                    label,
                    kind: category,
                    screen,
                });
            }
            if impact_items.is_empty() {
                degraded.push("impact_items_unavailable".to_owned());
            }
        } else {
            degraded.push("impact_id_not_supplied".to_owned());
        }

        let rules_digest = rules
            .iter()
            .map(|r| format!("{}:{}:{}", r.id, r.kind, r.condition))
            .collect::<Vec<_>>()
            .join("|");
        // 追記先の指定があれば、その計画を使う（新規作成しない）。
        // 承認済み計画へ草稿を混ぜない（実行対象が黙って増えるのを防ぐ）。
        let existing = match request.plan_id {
            Some(plan_id) => {
                let plan = self
                    .store()
                    .test_plan(&mut tx, project.0, plan_id)
                    .await?
                    .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
                if plan.status != "DRAFT" {
                    return Err(AppError::Conflict(
                        contracts::ErrorCode::ConflictRevisionSuperseded,
                    ));
                }
                Some(plan)
            }
            None => None,
        };
        let appended = existing.is_some();
        let (plan_id, plan_code) = match &existing {
            Some(plan) => (plan.id, plan.code.clone()),
            None => {
                let plan_id = stable_uuid(
                    project,
                    &[
                        "plan",
                        &request.impact_id.map(|v| v.to_string()).unwrap_or_default(),
                        &rules_digest,
                        &max_drafts.to_string(),
                    ],
                );
                let plan_code = format!(
                    "TP-{}",
                    &hex_lower(&Sha256::digest(plan_id.as_bytes()))[..8]
                );
                (plan_id, plan_code)
            }
        };

        let derivation = engine::derive_assets(
            project.0,
            &engine::DeriveInput {
                rules: rules.clone(),
                impact_items: impact_items.clone(),
                max_drafts,
                base_path: request
                    .base_path
                    .clone()
                    .unwrap_or_else(|| "/sample/user/edit".to_owned()),
                plan_code: plan_code.clone(),
            },
        );

        let scope = TestPlanScope {
            impact_id: request.impact_id,
            rule_ids: rules.iter().map(|r| r.id).collect(),
            impact_item_ids: impact_items.iter().map(|i| i.id).collect(),
            not_covered: derivation.not_covered.clone(),
        };
        if !appended {
            self.store()
                .insert_test_plan(
                    &mut tx,
                    &store::NewTestPlan {
                        id: plan_id,
                        project_id: project.0,
                        code: plan_code.clone(),
                        name: request
                            .name
                            .unwrap_or_else(|| "Q-01 から派生したテスト計画".to_owned()),
                        status: "DRAFT".to_owned(),
                        scope: serde_json::to_value(&scope).unwrap_or(json!({})),
                        impact_id: request.impact_id,
                        origin: if request.impact_id.is_some() {
                            "Q01_DERIVED".to_owned()
                        } else {
                            "MANUAL".to_owned()
                        },
                    },
                )
                .await?;
        }
        let scenarios: Vec<store::NewTestScenario> = derivation
            .scenarios
            .iter()
            .map(|s| store::NewTestScenario {
                id: s.id,
                project_id: project.0,
                plan_id,
                code: s.code.clone(),
                title: s.title.clone(),
                kind: s.kind.clone(),
                evidence_ids: s.evidence_ids.clone(),
            })
            .collect();
        self.store()
            .insert_test_scenarios(&mut tx, &scenarios)
            .await?;

        let cases: Vec<store::NewTestCase> = derivation
            .cases
            .iter()
            .map(|c| store::NewTestCase {
                id: c.id,
                project_id: project.0,
                scenario_id: c.scenario_id,
                code: c.code.clone(),
                title: c.title.clone(),
                status: c.status.clone(),
                dsl: serde_json::to_value(&c.dsl).unwrap_or(json!({})),
                preconditions: serde_json::to_value(&c.dsl.preconditions).unwrap_or(json!([])),
                expected: json!([]),
                covers_rules: c.covers_rules.clone(),
                covers_impacts: c.covers_impacts.clone(),
                covers_screens: c.covers_screens.clone(),
                automation: json!({"type": "playwright", "selector_strategy": c.dsl.selector_strategy}),
                destructive: c.destructive,
                generated_by: None,
                evidence_ids: c.evidence_ids.clone(),
            })
            .collect();
        self.store().insert_test_cases(&mut tx, &cases).await?;

        let mut steps = Vec::new();
        for case in &derivation.cases {
            for (index, step) in case.dsl.steps.iter().enumerate() {
                steps.push(store::NewTestStep {
                    id: stable_uuid(project, &[&case.code, "step", &index.to_string()]),
                    project_id: project.0,
                    case_id: case.id,
                    ordinal: index as i32,
                    action: step.action.clone(),
                    target: serde_json::to_value(&step.target).unwrap_or(json!({})),
                    value: step.value.clone(),
                });
            }
        }
        self.store().insert_test_steps(&mut tx, &steps).await?;

        let sets: Vec<store::NewTestDataSet> = derivation
            .data_sets
            .iter()
            .map(|d| store::NewTestDataSet {
                id: d.id,
                project_id: project.0,
                case_id: d.case_id,
                label: d.label.clone(),
                kind: d.kind.clone(),
                rule_id: None,
            })
            .collect();
        let data_rows: Vec<store::NewTestDataRow> = derivation
            .data_sets
            .iter()
            .map(|d| store::NewTestDataRow {
                id: stable_uuid(project, &[&d.label, "row", &d.case_id.to_string()]),
                project_id: project.0,
                data_set_id: d.id,
                field_ref: d.field_ref.clone(),
                value: d.value.clone(),
            })
            .collect();
        self.store()
            .insert_test_data(&mut tx, &sets, &data_rows)
            .await?;

        // TestCase / Requirement / VERIFIES を PG グラフへ投影する。
        // Requirement は現時点ではルールから推定したもの（resolution を引き継ぐ）。
        let requirements: Vec<store::TestGraphEntity> = rule_rows
            .iter()
            .map(|rule| store::TestGraphEntity {
                id: stable_uuid(project, &["requirement", &rule.id.to_string()]),
                natural_key: format!("rule:{}", rule.id),
                name: format!("{} {}", rule.field_ref, rule.condition),
                resolution: rule.resolution.clone(),
            })
            .collect();
        let graph_cases: Vec<store::TestGraphEntity> = cases
            .iter()
            .map(|case| store::TestGraphEntity {
                id: stable_uuid(project, &["testcase", &case.id.to_string()]),
                natural_key: case.id.to_string(),
                name: case.title.clone(),
                resolution: "INFERRED".to_owned(),
            })
            .collect();
        let mut verifies = Vec::new();
        for case in &cases {
            let src = stable_uuid(project, &["testcase", &case.id.to_string()]);
            for rule_id in &case.covers_rules {
                verifies.push((
                    src,
                    stable_uuid(project, &["requirement", &rule_id.to_string()]),
                ));
            }
        }
        self.store()
            .project_test_graph(&mut tx, project.0, &requirements, &graph_cases, &verifies)
            .await?;

        if let Some(plan) = &existing {
            // 追記: 既存 scope と今回の scope を統合する。impact_id は最初の
            // 派生元を残す（計画の出自が呼び出しごとに揺れないように）。
            let mut merged: TestPlanScope =
                serde_json::from_value(plan.scope.clone()).unwrap_or_default();
            if merged.impact_id.is_none() {
                merged.impact_id = scope.impact_id;
            }
            for rule_id in &scope.rule_ids {
                if !merged.rule_ids.contains(rule_id) {
                    merged.rule_ids.push(*rule_id);
                }
            }
            for item_id in &scope.impact_item_ids {
                if !merged.impact_item_ids.contains(item_id) {
                    merged.impact_item_ids.push(*item_id);
                }
            }
            for reason in &scope.not_covered {
                if !merged.not_covered.contains(reason) {
                    merged.not_covered.push(reason.clone());
                }
            }
            self.store()
                .update_test_plan_scope(
                    &mut tx,
                    project.0,
                    plan_id,
                    &serde_json::to_value(&merged).unwrap_or(json!({})),
                )
                .await?;
        }

        let plan_case_count = self
            .store()
            .list_test_cases(&mut tx, project.0)
            .await?
            .iter()
            .filter(|case| case.plan_id == plan_id)
            .count();

        self.store()
            .append_audit(
                &mut tx,
                &Self::audit_draft(
                    project,
                    auth,
                    "test.plan.derive",
                    "test.test_plan",
                    plan_id,
                    json!({
                        "impact_id": request.impact_id,
                        "appended": appended,
                        "drafts": derivation.cases.len(),
                        "plan_case_count": plan_case_count,
                        "truncated": derivation.truncated,
                        "test_run_created": false,
                    }),
                    trace,
                ),
            )
            .await?;
        store::commit(tx).await?;

        degraded.extend(derivation.degraded);
        degraded.sort();
        degraded.dedup();
        Ok(DeriveTestPlanResponse {
            plan_id,
            appended,
            scenario_count: derivation.scenarios.len(),
            draft_case_count: derivation.cases.len(),
            plan_case_count,
            drafts_truncated: derivation.truncated,
            test_run_created: false,
            degraded,
        })
    }

    // ── 一括取得 ──────────────────────────────────────────

    pub async fn test_overview(
        &self,
        auth: &Authenticated,
        project: ProjectId,
    ) -> Result<TestOverviewResponse, AppError> {
        self.require_test_read(auth, project).await?;
        let env = default_env();
        let mut tx = self.store().begin_scoped(project).await?;
        let rules = self
            .store()
            .list_validation_rules(&mut tx, project.0)
            .await?;
        let plans = self.store().list_test_plans(&mut tx, project.0).await?;
        let cases = self.store().list_test_cases(&mut tx, project.0).await?;
        let data = self.store().list_test_data(&mut tx, project.0).await?;
        let runs = self.store().list_test_runs(&mut tx, project.0).await?;
        let executions = self
            .store()
            .list_test_executions(&mut tx, project.0)
            .await?;
        let defects = self.store().list_defects(&mut tx, project.0).await?;
        let snapshot = self
            .store()
            .latest_coverage_snapshot(&mut tx, project.0)
            .await?;
        let graph = self.coverage_graph(&mut tx, project.0).await?;
        store::commit(tx).await?;

        let mut degraded = vec!["runner_external_process".to_owned()];
        if !graph.available {
            degraded.push("graph_verifies_unavailable".to_owned());
        }
        if cases.iter().all(|c| c.status != "APPROVED") {
            degraded.push("no_approved_cases".to_owned());
        }
        if rules.is_empty() {
            degraded.push("rules_not_indexed".to_owned());
        }
        degraded.sort();
        degraded.dedup();

        Ok(TestOverviewResponse {
            rules: rules.iter().map(rule_view).collect(),
            plans: plans.iter().map(plan_view).collect(),
            cases: cases.iter().map(|c| case_view(c, &env)).collect(),
            data: data
                .into_iter()
                .map(|d| TestDataView {
                    case_id: d.case_id,
                    label: d.label,
                    kind: d.kind,
                    field_ref: d.field_ref,
                    value: d.value,
                })
                .collect(),
            runs: runs.iter().map(run_view).collect(),
            executions: executions.iter().map(execution_view).collect(),
            defects: defects.iter().map(defect_view).collect(),
            coverage: snapshot.as_ref().map(coverage_view),
            degraded,
        })
    }

    // ── 承認 ──────────────────────────────────────────────

    pub async fn approve_test_case(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        case_id: Uuid,
        trace: Option<String>,
    ) -> Result<ApproveTestCaseResponse, AppError> {
        self.require_test_approve(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let case = self
            .store()
            .test_case(&mut tx, project.0, case_id)
            .await?
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
        let errors = engine::validate_dsl(&dsl_from(&case.dsl), &default_env());
        if !errors.is_empty() {
            // API 側で 422 (SCHEMA_VIOLATION) に変換する。ここでは承認しない。
            store::commit(tx).await?;
            return Ok(ApproveTestCaseResponse {
                case_id,
                status: case.status,
                validation_errors: errors,
            });
        }
        let changed = self
            .store()
            .approve_test_case(&mut tx, project.0, case_id)
            .await?;
        self.store()
            .append_audit(
                &mut tx,
                &Self::audit_draft(
                    project,
                    auth,
                    "test.case.approve",
                    "test.test_case",
                    case_id,
                    json!({"changed": changed}),
                    trace,
                ),
            )
            .await?;
        store::commit(tx).await?;
        Ok(ApproveTestCaseResponse {
            case_id,
            status: if changed || case.status == "APPROVED" {
                "APPROVED".to_owned()
            } else {
                case.status
            },
            validation_errors: Vec::new(),
        })
    }

    // ── run 起票と環境ロック ───────────────────────────────

    pub async fn create_test_run(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        request: CreateTestRunRequest,
        trace: Option<String>,
    ) -> Result<CreateTestRunResponse, AppError> {
        self.require_test_run(auth, project).await?;
        if request.name.trim().is_empty() || request.environment.id.trim().is_empty() {
            return Err(AppError::Validation(
                "name / environment.id は必須です".into(),
            ));
        }
        if !matches!(
            request.environment.guard_kind.as_str(),
            "LOCAL" | "STAGING" | "PRODUCTION"
        ) {
            return Err(AppError::Validation("guard_kind が不正です".into()));
        }
        // 04 §4.7「正式環境でのテスト実行は承認が必要」。REST からの
        // PRODUCTION 実行に承認を通す経路はまだ繋いでいないので、**受け付けない**。
        // 素通しにすると、権限を持つ人が承認なしで本番に対して実行できてしまう。
        if request.environment.guard_kind == "PRODUCTION" {
            return Err(AppError::Validation(
                "PRODUCTION 環境の実行は承認フローが未接続のため受け付けません".into(),
            ));
        }
        if !(1..=16).contains(&request.environment.parallel) {
            return Err(AppError::Validation("parallel は 1〜16 の範囲です".into()));
        }

        let run_id = stable_uuid(
            project,
            &[
                "run",
                &request.name,
                &request.environment.id,
                &request.plan_id.map(|v| v.to_string()).unwrap_or_default(),
            ],
        );
        let environment_value = serde_json::to_value(&request.environment)
            .map_err(|error| AppError::Validation(error.to_string()))?;

        let mut tx = self.store().begin_scoped(project).await?;
        let cases = self.store().list_test_cases(&mut tx, project.0).await?;
        let case_count = cases
            .iter()
            .filter(|c| c.status == "APPROVED" && has_covers(c) && in_plan(c, request.plan_id))
            .count() as i32;
        let holder = self
            .store()
            .env_lock_holder(&mut tx, project.0, &request.environment.id)
            .await?;
        let queued_behind = holder.is_some();
        self.store()
            .insert_test_run(
                &mut tx,
                &store::NewTestRun {
                    id: run_id,
                    project_id: project.0,
                    plan_id: request.plan_id,
                    name: request.name.clone(),
                    environment: environment_value,
                    environment_id: request.environment.id.clone(),
                    status: "QUEUED".to_owned(),
                    lock_acquired: !queued_behind,
                },
            )
            .await?;

        let mut job_id = None;
        if !queued_behind {
            let enqueued = self
                .store()
                .enqueue_job(
                    &mut tx,
                    &store::NewJob {
                        project_id: project.0,
                        kind: "TEST_RUN".into(),
                        idempotency_key: format!("test-run:{run_id}"),
                        input: json!({"run_id": run_id}),
                    },
                )
                .await?;
            job_id = Some(enqueued.job.id);
            self.store()
                .set_test_run_job(&mut tx, project.0, run_id, enqueued.job.id)
                .await?;
        }
        self.store()
            .append_audit(
                &mut tx,
                &Self::audit_draft(
                    project,
                    auth,
                    "test.run.create",
                    "test.test_run",
                    run_id,
                    json!({
                        "environment_id": request.environment.id,
                        "guard_kind": request.environment.guard_kind,
                        "queued_behind": queued_behind,
                        "case_count": case_count,
                    }),
                    trace,
                ),
            )
            .await?;
        store::commit(tx).await?;

        Ok(CreateTestRunResponse {
            run_id,
            job_id,
            status: "QUEUED".to_owned(),
            queued_behind,
            case_count,
        })
    }

    /// worker 用。Runner を起動せず、対象ケース数を確定して READY にするだけ。
    /// READY は「実行待ち」であって「実行済み」ではない。
    pub async fn prepare_test_run_from_job(
        &self,
        project: ProjectId,
        job_id: Uuid,
        input: &Value,
    ) -> Result<i32, AppError> {
        let run_id = input
            .get("run_id")
            .and_then(Value::as_str)
            .and_then(|v| Uuid::parse_str(v).ok())
            .ok_or_else(|| AppError::Validation("run_id がありません".into()))?;
        let mut tx = self.store().begin_scoped(project).await?;
        let cases = self.store().list_test_cases(&mut tx, project.0).await?;
        let run = self
            .store()
            .test_run(&mut tx, project.0, run_id)
            .await?
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
        let count = cases
            .iter()
            .filter(|c| c.status == "APPROVED" && has_covers(c) && in_plan(c, run.plan_id))
            .count() as i32;
        self.store()
            .set_test_run_case_count(&mut tx, project.0, run_id, count)
            .await?;
        self.store()
            .append_audit(
                &mut tx,
                &store::AuditDraft {
                    project_id: Some(project.0),
                    actor_type: "SYSTEM".into(),
                    actor_id: job_id.to_string(),
                    action: "test.run.prepare".into(),
                    target_type: Some("test.test_run".into()),
                    target_id: Some(run_id.to_string()),
                    input_digest: None,
                    result: "READY".into(),
                    detail: json!({"case_count": count, "runner": "EXTERNAL_PROCESS"}),
                    trace_id: None,
                },
            )
            .await?;
        store::commit(tx).await?;
        Ok(count)
    }

    // ── 実行結果の取り込み ─────────────────────────────────

    pub async fn import_test_executions(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        run_id: Uuid,
        request: ImportExecutionsRequest,
        trace: Option<String>,
    ) -> Result<ImportExecutionsResponse, AppError> {
        self.require_test_run(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let run = self
            .store()
            .test_run(&mut tx, project.0, run_id)
            .await?
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
        let cases = self.store().list_test_cases(&mut tx, project.0).await?;
        let approved: BTreeMap<Uuid, &store::TestCaseRow> = cases
            .iter()
            .filter(|c| c.status == "APPROVED" && has_covers(c) && in_plan(c, run.plan_id))
            .map(|c| (c.id, c))
            .collect();
        let mut rejected = Vec::new();
        let mut accepted = 0usize;
        let mut defect_candidates = 0usize;

        for item in &request.executions {
            let Some(case) = approved.get(&item.case_id).copied() else {
                rejected.push(format!("{}: case_not_approved_or_unknown", item.case_id));
                continue;
            };
            let mut status = item.status.clone();
            if engine::guard_execution(
                case.destructive,
                &TestEnvironment {
                    guard_kind: run.guard_kind.clone(),
                    ..default_env()
                },
            ) {
                status = "SKIPPED".to_owned();
            }
            let attempt1 = self
                .store()
                .attempt1_failure_kind(&mut tx, project.0, run_id, item.case_id)
                .await?;
            let mut normalized = item.clone();
            normalized.status = status.clone();
            let errors = engine::validate_import(
                &normalized,
                &engine::ImportContext {
                    case_status: case.status.clone(),
                    destructive: case.destructive,
                    guard_kind: run.guard_kind.clone(),
                    attempt1_failure_kind: attempt1,
                },
            );
            if !errors.is_empty() {
                rejected.push(format!("{}: {}", item.case_id, errors.join(",")));
                continue;
            }

            let fingerprint =
                engine::is_defect_candidate(item.failure_kind.as_deref()).then(|| {
                    engine::fingerprint(
                        item.case_id,
                        item.failure_kind.as_deref().unwrap_or("UNKNOWN"),
                        item.failure_step.unwrap_or(0),
                        item.expected.as_deref(),
                        item.actual.as_deref(),
                        false,
                    )
                });
            let id = stable_uuid(
                project,
                &[
                    &run_id.to_string(),
                    &item.case_id.to_string(),
                    &item.attempt.to_string(),
                ],
            );
            self.store()
                .insert_test_execution(
                    &mut tx,
                    &store::NewTestExecution {
                        id,
                        project_id: project.0,
                        test_run_id: run_id,
                        test_case_id: item.case_id,
                        attempt: item.attempt,
                        status: status.clone(),
                        execution_source: item.execution_source.clone(),
                        failure_kind: item.failure_kind.clone(),
                        failure_step: item.failure_step,
                        expected: item.expected.clone(),
                        actual: item.actual.clone(),
                        duration_ms: item.duration_ms,
                        artifacts: serde_json::to_value(&item.artifacts).unwrap_or(json!([])),
                        fingerprint: fingerprint.clone(),
                    },
                )
                .await?;
            accepted += 1;

            if let Some(fp) = fingerprint {
                let severity = match item.failure_kind.as_deref() {
                    Some("NETWORK" | "TIMEOUT") => "HIGH",
                    Some("SELECTOR_NOT_FOUND") => "MEDIUM",
                    _ => "MEDIUM",
                };
                self.store()
                    .upsert_defect(
                        &mut tx,
                        project.0,
                        stable_uuid(project, &["defect", &fp]),
                        &fp,
                        &format!("{} の実行失敗", case.title),
                        severity,
                        id,
                        case.covers_impacts.first().copied(),
                    )
                    .await?;
                defect_candidates += 1;
            }
        }

        // 全承認ケースに終端状態が揃ったら run を DONE にし、環境ロックを解放して
        // 次の QUEUED run を 1 件だけ起動する（R-12）。
        let all = self
            .store()
            .list_test_executions(&mut tx, project.0)
            .await?;
        let finished: BTreeSet<Uuid> = all
            .iter()
            .filter(|e| {
                e.test_run_id == run_id
                    && matches!(
                        e.status.as_str(),
                        "PASSED" | "FAILED" | "SKIPPED" | "BLOCKED"
                    )
            })
            .map(|e| e.test_case_id)
            .collect();
        let run_status = if !approved.is_empty() && approved.keys().all(|id| finished.contains(id))
        {
            self.store()
                .finish_test_run(&mut tx, project.0, run_id, "DONE")
                .await?;
            if let Some(next_id) = self
                .store()
                .activate_next_queued_run(&mut tx, project.0, &run.environment_id)
                .await?
            {
                let enqueued = self
                    .store()
                    .enqueue_job(
                        &mut tx,
                        &store::NewJob {
                            project_id: project.0,
                            kind: "TEST_RUN".into(),
                            idempotency_key: format!("test-run:{next_id}"),
                            input: json!({"run_id": next_id}),
                        },
                    )
                    .await?;
                self.store()
                    .set_test_run_job(&mut tx, project.0, next_id, enqueued.job.id)
                    .await?;
            }
            "DONE".to_owned()
        } else {
            run.status.clone()
        };

        // 被覆は import のたびに冪等再計算する。
        let graph = self.coverage_graph(&mut tx, project.0).await?;
        let all_executions = self
            .store()
            .list_test_executions(&mut tx, project.0)
            .await?;
        let (totals, coverage_cases) = coverage_inputs(&cases, &all_executions, run_id, &graph);
        let mut result = engine::compute_coverage(&totals, &coverage_cases);
        annotate_requirement_source(&mut result, graph.available);
        let detail = serde_json::to_value(&result)
            .map_err(|error| AppError::Validation(error.to_string()))?;
        self.store()
            .upsert_coverage_snapshot(
                &mut tx,
                project.0,
                stable_uuid(project, &["coverage", &run_id.to_string()]),
                run_id,
                &json!({"test_run_id": run_id}),
                &detail,
            )
            .await?;

        self.store()
            .append_audit(
                &mut tx,
                &Self::audit_draft(
                    project,
                    auth,
                    "test.executions.import",
                    "test.test_run",
                    run_id,
                    json!({"accepted": accepted, "rejected": rejected.len()}),
                    trace,
                ),
            )
            .await?;
        store::commit(tx).await?;

        Ok(ImportExecutionsResponse {
            accepted,
            rejected,
            defect_candidates,
            run_status,
        })
    }

    // ── 被覆の再計算（冪等） ───────────────────────────────

    pub async fn recompute_test_coverage(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        request: RecomputeCoverageRequest,
    ) -> Result<CoverageView, AppError> {
        self.require_test_read(auth, project).await?;
        let mut tx = self.store().begin_scoped(project).await?;
        let cases = self.store().list_test_cases(&mut tx, project.0).await?;
        let executions = self
            .store()
            .list_test_executions(&mut tx, project.0)
            .await?;
        let graph = self.coverage_graph(&mut tx, project.0).await?;
        let (totals, coverage_cases) =
            coverage_inputs(&cases, &executions, request.test_run_id, &graph);
        let mut result = engine::compute_coverage(&totals, &coverage_cases);
        annotate_requirement_source(&mut result, graph.available);
        let detail = serde_json::to_value(&result)
            .map_err(|error| AppError::Validation(error.to_string()))?;
        self.store()
            .upsert_coverage_snapshot(
                &mut tx,
                project.0,
                stable_uuid(project, &["coverage", &request.test_run_id.to_string()]),
                request.test_run_id,
                &json!({"test_run_id": request.test_run_id}),
                &detail,
            )
            .await?;
        let snapshot = self
            .store()
            .coverage_snapshot(&mut tx, project.0, request.test_run_id)
            .await?;
        store::commit(tx).await?;
        snapshot
            .as_ref()
            .map(coverage_view)
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))
    }

    // ── 欠陥起票（人間操作） ───────────────────────────────

    pub async fn create_test_defect(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        request: CreateDefectRequest,
        trace: Option<String>,
    ) -> Result<DefectCandidateView, AppError> {
        self.require_test_write(auth, project).await?;
        if !matches!(
            request.severity.as_str(),
            "LOW" | "MEDIUM" | "HIGH" | "CRITICAL"
        ) {
            return Err(AppError::Validation("severity が不正です".into()));
        }
        let mut tx = self.store().begin_scoped(project).await?;
        let execution = self
            .store()
            .test_execution(&mut tx, project.0, request.execution_id)
            .await?
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))?;
        let fingerprint = execution.fingerprint.clone().ok_or_else(|| {
            AppError::Validation("欠陥候補にならない実行です（fingerprint 無し）".into())
        })?;
        self.store()
            .upsert_defect(
                &mut tx,
                project.0,
                stable_uuid(project, &["defect", &fingerprint]),
                &fingerprint,
                &request.title,
                &request.severity,
                execution.id,
                request.impact_item_id,
            )
            .await?;
        let defects = self.store().list_defects(&mut tx, project.0).await?;
        self.store()
            .append_audit(
                &mut tx,
                &Self::audit_draft(
                    project,
                    auth,
                    "test.defect.create",
                    "test.defect",
                    execution.id,
                    json!({"fingerprint": fingerprint}),
                    trace,
                ),
            )
            .await?;
        store::commit(tx).await?;
        defects
            .iter()
            .find(|d| d.fingerprint == fingerprint)
            .map(defect_view)
            .ok_or(AppError::NotFound(contracts::ErrorCode::NotFound))
    }
}
