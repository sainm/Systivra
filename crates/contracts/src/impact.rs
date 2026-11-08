use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{JobLinks, JobStatus};

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateImpactAssessmentRequest {
    pub seed: ImpactSeedRequest,
    #[serde(default)]
    pub options: ImpactOptionsRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ImpactSeedRequest {
    pub kind: String,
    #[serde(default)]
    pub entity_ids: Vec<Uuid>,
    pub base_revision_id: Option<Uuid>,
    pub head_revision_id: Option<Uuid>,
    #[serde(default)]
    pub paths: Vec<String>,
    /// Optional Git unified diff. When present, changed line ranges are parsed
    /// from the patch instead of recomputing them from base/head snapshots.
    #[serde(default)]
    pub diff_unified: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ImpactOptionsRequest {
    pub max_depth: i32,
    pub hub_threshold: i32,
    pub max_items: i32,
    pub include_documents: bool,
}

impl Default for ImpactOptionsRequest {
    fn default() -> Self {
        Self {
            max_depth: 6,
            hub_threshold: 200,
            max_items: 500,
            include_documents: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ImpactAssessmentAcceptedResponse {
    pub impact_id: Uuid,
    pub job_id: Uuid,
    pub status: JobStatus,
    pub links: JobLinks,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ImpactAssessmentSummary {
    pub impact_id: Uuid,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub total: i64,
    pub degraded: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ImpactAssessmentListResponse {
    pub items: Vec<ImpactAssessmentSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ImpactSeedSummary {
    pub source: String,
    pub natural_key: String,
    pub kind: String,
    pub label: String,
    pub entity_id: Option<Uuid>,
    pub path: Option<String>,
    pub line_start: Option<i32>,
    pub line_end: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ImpactLineHunk {
    pub path: String,
    pub old_start: i32,
    pub old_end: i32,
    pub new_start: i32,
    pub new_end: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ImpactEvidence {
    pub evidence_id: Option<Uuid>,
    pub source_type: String,
    pub quote: String,
    pub locator: Value,
    pub resolution: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ImpactPathEdge {
    pub edge_id: Uuid,
    pub src: Uuid,
    pub dst: Uuid,
    pub relation: String,
    pub resolution: String,
    pub confidence: f32,
    pub evidence: Option<ImpactEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ImpactItem {
    pub item_id: Uuid,
    pub level: String,
    pub category: String,
    pub direction: String,
    pub entity_id: Uuid,
    pub natural_key: String,
    pub label: String,
    pub entity_kind: String,
    pub depth: i32,
    pub resolution: String,
    pub confidence: f32,
    pub why: String,
    pub path: Vec<Uuid>,
    pub path_edges: Vec<ImpactPathEdge>,
    pub evidence: Vec<ImpactEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ImpactUnverifiedItem {
    pub entity_id: Option<Uuid>,
    pub natural_key: String,
    pub label: String,
    pub category: String,
    pub why: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ImpactStats {
    pub total: i64,
    pub code: i64,
    pub screens: i64,
    pub documents: i64,
    pub data: i64,
    pub resolved: i64,
    pub partial: i64,
    pub inferred: i64,
    pub unresolved: i64,
    pub conflict: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ImpactTruncation {
    pub hubs: i64,
    pub depth: bool,
    pub budget: bool,
    pub no_logical_change: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ImpactRiskFactor {
    pub name: String,
    pub weight: f32,
    pub value: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ImpactRisk {
    pub score: f32,
    pub level: String,
    pub factors: Vec<ImpactRiskFactor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ImpactAssessmentDraft {
    pub seeds: Vec<ImpactSeedSummary>,
    #[serde(default)]
    pub changed_hunks: Vec<ImpactLineHunk>,
    pub items: Vec<ImpactItem>,
    pub unverified: Vec<ImpactUnverifiedItem>,
    pub stats: ImpactStats,
    pub risk: ImpactRisk,
    pub truncated: ImpactTruncation,
    pub degraded: Vec<String>,
    pub base_revision_id: Option<Uuid>,
    pub head_revision_id: Option<Uuid>,
    pub engine_version: String,
    pub ontology_version: String,
    pub approval_state: String,
    pub code_modified: bool,
    pub tests_executed: bool,
    /// AI が付けた叙述。未生成なら None（空文字にしない）。
    #[serde(default)]
    pub narrative: Option<String>,
    /// AI / null
    #[serde(default)]
    pub narrative_source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ImpactAssessmentResponse {
    pub impact_id: Uuid,
    pub status: String,
    pub input: Value,
    pub report: Option<ImpactAssessmentDraft>,
    pub error: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct ImpactAssessmentListQuery {
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct NarrateImpactResponse {
    pub narrative: Option<String>,
    pub model: Option<String>,
    #[serde(default)]
    pub degraded: Vec<String>,
}
