//! Q-01 変更影響問合せの API 契約（`detail/08-change-inquiry.md`）。
//!
//! 5 セクションは ImpactReport からの投影結果であり、新しい走査はしない。
//! `summary.headline` は件数を差し込む決定論的テンプレートで、AI に書かせない。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct InquiryInput {
    /// NATURAL_LANGUAGE / ENTITY / DIFF
    #[serde(default = "default_kind")]
    pub kind: String,
    #[serde(default)]
    pub entity_ids: Vec<Uuid>,
    #[serde(default)]
    pub base_revision_id: Option<Uuid>,
    #[serde(default)]
    pub head_revision_id: Option<Uuid>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub diff_unified: Option<String>,
    #[serde(default)]
    pub doc_types: Vec<String>,
}

fn default_kind() -> String {
    "NATURAL_LANGUAGE".to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateInquiryRequest {
    pub question: String,
    #[serde(default)]
    pub input: InquiryInput,
    /// QUICK / FULL
    #[serde(default = "default_mode")]
    pub mode: String,
}

fn default_mode() -> String {
    "FULL".to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct InquiryLinks {
    pub events: String,
    pub workbench: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct InquiryAcceptedResponse {
    pub inquiry_id: Uuid,
    pub impact_id: Option<Uuid>,
    pub job_id: Option<Uuid>,
    /// QUEUED / NEEDS_CLARIFICATION / FAILED
    pub status: String,
    pub links: InquiryLinks,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct InquiryCandidate {
    pub entity_id: Uuid,
    pub natural_key: String,
    pub label: String,
    pub score: f64,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct InquiryCounts {
    pub screens: i64,
    pub documents: i64,
    pub code: i64,
    pub tests: i64,
    pub other: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct InquiryHypothesisBasis {
    /// screens / documents / code
    pub section: String,
    pub entity_id: Uuid,
    pub label: String,
    pub quote: String,
    pub locator: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct InquiryHypothesis {
    pub claim: String,
    pub confidence: f32,
    /// 常に INFERRED。AI の出力で RESOLVED に昇格させない。
    pub resolution: String,
    pub basis: Vec<InquiryHypothesisBasis>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct InquirySummary {
    pub headline: String,
    pub approval_state: String,
    pub code_modified: bool,
    pub tests_executed: bool,
    pub risk: Value,
    pub counts: InquiryCounts,
    pub truncated: Value,
    #[serde(default)]
    pub degraded: Vec<String>,
    /// AI が付けた叙述。未生成なら None（空文字にしない）。
    #[serde(default)]
    pub narrative: Option<String>,
    /// AI / null
    #[serde(default)]
    pub narrative_source: Option<String>,
    #[serde(default)]
    pub hypotheses: Vec<InquiryHypothesis>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct InquiryItem {
    pub entity_id: Uuid,
    pub natural_key: String,
    pub label: String,
    pub kind: String,
    pub resolution: String,
    pub confidence: f32,
    pub why: String,
    #[serde(default)]
    pub path: Vec<Uuid>,
    pub quote: String,
    pub locator: Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct InquirySection {
    pub total: i64,
    #[serde(default)]
    pub items: Vec<InquiryItem>,
    #[serde(default)]
    pub degraded: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct InquiryExistingCase {
    pub case_id: Uuid,
    pub code: String,
    pub title: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct InquiryDraftCase {
    pub code: String,
    pub title: String,
    /// 常に DRAFT。承認は人間だけ。
    pub status: String,
    pub covers_rules: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct InquiryTests {
    #[serde(default)]
    pub existing: Vec<InquiryExistingCase>,
    #[serde(default)]
    pub drafts: Vec<InquiryDraftCase>,
    #[serde(default)]
    pub steps: Vec<String>,
    #[serde(default)]
    pub data_boundaries: Vec<String>,
    #[serde(default)]
    pub not_covered: Vec<String>,
    #[serde(default)]
    pub degraded: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct InquiryResponse {
    pub inquiry_id: Uuid,
    pub status: String,
    pub question: String,
    pub input_kind: String,
    pub impact_id: Option<Uuid>,
    pub summary: Option<InquirySummary>,
    pub screens: Option<InquirySection>,
    pub documents: Option<InquirySection>,
    pub code: Option<InquirySection>,
    pub tests: Option<InquiryTests>,
    #[serde(default)]
    pub candidates: Vec<InquiryCandidate>,
    pub error: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct InquiryListResponse {
    pub items: Vec<InquiryResponse>,
}

/// SSE の 1 イベント。`partial` は 5 段のうち確定したものを運ぶ。
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct InquiryEvent {
    /// summary / screens / documents / code / tests / done / error
    pub event: String,
    pub data: Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct DeriveInquiryPlanRequest {
    pub name: Option<String>,
    pub max_drafts: Option<i32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct NarrateInquiryResponse {
    /// AI が未設定なら None。空文字で埋めない。
    pub narrative: Option<String>,
    pub hypotheses: Vec<InquiryHypothesis>,
    pub model: Option<String>,
    #[serde(default)]
    pub degraded: Vec<String>,
}
