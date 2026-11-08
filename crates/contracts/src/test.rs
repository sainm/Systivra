//! Phase 6: Test Engineering の API 契約。
//!
//! 状態・種別は既存の `contracts` と同じく `String` + DB の CHECK 制約で表す。
//! DSL と環境は `serde_json::Value` に逃がさず具体型にする（保存時検証を
//! `engine-test` で一意にするため）。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

// ── ルール抽出 ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RuleExtractionSource {
    /// CODE / SPEC / INFERRED
    pub kind: String,
    pub field_ref: String,
    /// Java 検証コード片、JSP 属性、設計書の記述など。
    pub text: String,
    #[serde(default)]
    pub evidence_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ExtractRulesRequest {
    pub sources: Vec<RuleExtractionSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ValidationRuleView {
    pub id: Uuid,
    pub field_ref: String,
    pub kind: String,
    pub condition: String,
    pub source: String,
    pub resolution: String,
    pub evidence_ids: Vec<Uuid>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ExtractRulesResponse {
    pub rules: Vec<ValidationRuleView>,
    pub conflicts: usize,
    pub unparsed: usize,
    #[serde(default)]
    pub degraded: Vec<String>,
}

// ── DSL ────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct DslStep {
    pub action: String,
    #[serde(default)]
    pub target: BTreeMap<String, String>,
    pub value: Option<String>,
    /// `wait_for` の固定待機ミリ秒。5,000 上限。
    pub ms: Option<i32>,
    /// assert_text / assert_url の一致方式。exact / contains / regex。
    pub match_mode: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct TestCaseDsl {
    pub v: i32,
    pub case_code: String,
    pub title: String,
    #[serde(default)]
    pub preconditions: Vec<String>,
    #[serde(default)]
    pub steps: Vec<DslStep>,
    pub data: Option<String>,
    #[serde(default = "default_selector_strategy")]
    pub selector_strategy: String,
}

fn default_selector_strategy() -> String {
    "role_first".to_owned()
}

// ── 計画・ケース ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct DeriveTestPlanRequest {
    /// Q-01 の Impact Draft。`None` の場合は既存ルールのみから派生する。
    pub impact_id: Option<Uuid>,
    /// 既存の計画へ**追記**する場合の計画 ID。指定すると計画を新規作成しない。
    /// `test.case_generate` はこちらを使う（`test.plan_generate` は新規作成）。
    #[serde(default)]
    pub plan_id: Option<Uuid>,
    pub name: Option<String>,
    /// 草稿ケースの上限。detail/08 §6.2 の 20 件を既定とする。
    pub max_drafts: Option<i32>,
    #[serde(default)]
    pub base_path: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct TestPlanScope {
    pub impact_id: Option<Uuid>,
    #[serde(default)]
    pub rule_ids: Vec<Uuid>,
    #[serde(default)]
    pub impact_item_ids: Vec<Uuid>,
    #[serde(default)]
    pub not_covered: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct TestPlanView {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub status: String,
    pub origin: String,
    pub impact_id: Option<Uuid>,
    pub scope: TestPlanScope,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct DeriveTestPlanResponse {
    pub plan_id: Uuid,
    /// 既存計画への追記だったか（新規作成なら false）。
    pub appended: bool,
    pub scenario_count: usize,
    /// **この呼び出しで**生成した草稿ケース数。
    pub draft_case_count: usize,
    /// 追記後に計画が持つケース総数。
    pub plan_case_count: usize,
    pub drafts_truncated: usize,
    /// R-16: Q-01 からの派生では test_run を絶対に作らない。
    pub test_run_created: bool,
    #[serde(default)]
    pub degraded: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct TestCaseView {
    pub id: Uuid,
    pub scenario_id: Uuid,
    pub code: String,
    pub title: String,
    pub status: String,
    pub dsl: TestCaseDsl,
    pub covers_rules: Vec<Uuid>,
    pub covers_impacts: Vec<Uuid>,
    pub covers_screens: Vec<String>,
    pub evidence_ids: Vec<Uuid>,
    pub destructive: bool,
    /// DSL 保存時検証の結果。空なら検証通過。
    pub validation_errors: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct TestDataView {
    pub case_id: Uuid,
    pub label: String,
    pub kind: String,
    pub field_ref: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ApproveTestCaseResponse {
    pub case_id: Uuid,
    pub status: String,
    #[serde(default)]
    pub validation_errors: Vec<String>,
}

// ── 実行 ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct TestEnvironment {
    pub id: String,
    pub base_url: String,
    /// LOCAL / STAGING / PRODUCTION
    #[serde(default = "default_guard_kind")]
    pub guard_kind: String,
    #[serde(default)]
    pub allowed_origins: Vec<String>,
    #[serde(default = "default_parallel")]
    pub parallel: i32,
}

fn default_guard_kind() -> String {
    "STAGING".to_owned()
}
fn default_parallel() -> i32 {
    4
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateTestRunRequest {
    pub name: String,
    pub plan_id: Option<Uuid>,
    pub environment: TestEnvironment,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateTestRunResponse {
    pub run_id: Uuid,
    /// 環境ロック待ちのときは None。
    pub job_id: Option<Uuid>,
    pub status: String,
    pub queued_behind: bool,
    pub case_count: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct TestRunView {
    pub id: Uuid,
    pub name: String,
    pub status: String,
    pub environment_id: String,
    pub guard_kind: String,
    pub lock_acquired: bool,
    pub case_count: i32,
    pub job_id: Option<Uuid>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct TestArtifact {
    pub kind: String,
    pub uri: String,
    pub bytes: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ExpireArtifactsRequest {
    #[serde(default)]
    pub retention_days: Option<i64>,
    #[serde(default)]
    pub trace_days: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ArtifactCleanupResponse {
    pub scanned: i64,
    pub expired: i64,
    pub deleted_files: i64,
    pub bytes_reclaimed: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ExecutionImport {
    pub case_id: Uuid,
    #[serde(default = "default_attempt")]
    pub attempt: i16,
    pub status: String,
    /// IMPORTED / PLAYWRIGHT / SIMULATED。Runner 未接続を隠さない。
    #[serde(default = "default_execution_source")]
    pub execution_source: String,
    pub failure_kind: Option<String>,
    pub failure_step: Option<i32>,
    pub expected: Option<String>,
    pub actual: Option<String>,
    pub duration_ms: Option<i32>,
    #[serde(default)]
    pub artifacts: Vec<TestArtifact>,
}

fn default_attempt() -> i16 {
    1
}
fn default_execution_source() -> String {
    "IMPORTED".to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ImportExecutionsRequest {
    pub executions: Vec<ExecutionImport>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ImportExecutionsResponse {
    pub accepted: usize,
    pub rejected: Vec<String>,
    pub defect_candidates: usize,
    pub run_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct TestExecutionView {
    pub id: Uuid,
    pub test_run_id: Uuid,
    pub test_case_id: Uuid,
    pub attempt: i16,
    pub status: String,
    pub execution_source: String,
    pub failure_kind: Option<String>,
    pub failure_step: Option<i32>,
    pub fingerprint: Option<String>,
    pub artifacts: Vec<TestArtifact>,
}

// ── 被覆・欠陥 ──────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CoverageMetric {
    pub num: i64,
    pub den: i64,
    pub ratio: f64,
    #[serde(default)]
    pub degraded: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CoverageExcluded {
    pub draft_cases: i64,
    pub never_executed: i64,
    pub skipped: i64,
    pub covers_empty: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CoverageDetail {
    pub requirement: CoverageMetric,
    pub rule: CoverageMetric,
    pub screen: CoverageMetric,
    pub impact: CoverageMetric,
    pub regression: CoverageMetric,
    pub excluded: CoverageExcluded,
    #[serde(default)]
    pub degraded: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CoverageView {
    pub test_run_id: Uuid,
    pub computed_at: DateTime<Utc>,
    pub detail: CoverageDetail,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RecomputeCoverageRequest {
    pub test_run_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct DefectCandidateView {
    pub id: Uuid,
    pub fingerprint: String,
    pub title: String,
    pub severity: String,
    pub status: String,
    pub recurrence: i32,
    pub execution_id: Option<Uuid>,
    pub impact_item_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateDefectRequest {
    pub execution_id: Uuid,
    pub title: String,
    pub severity: String,
    pub impact_item_id: Option<Uuid>,
}

// ── 画面の一括取得 ──────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct TestOverviewResponse {
    pub rules: Vec<ValidationRuleView>,
    pub plans: Vec<TestPlanView>,
    pub cases: Vec<TestCaseView>,
    pub data: Vec<TestDataView>,
    pub runs: Vec<TestRunView>,
    pub executions: Vec<TestExecutionView>,
    pub defects: Vec<DefectCandidateView>,
    pub coverage: Option<CoverageView>,
    #[serde(default)]
    pub degraded: Vec<String>,
}
