use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{JobLinks, JobStatus};

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateCodeRepositoryRequest {
    /// `DIR` / `SNAPSHOT` / `GIT`. `SVN` is explicitly unsupported in Phase 4.
    pub kind: String,
    /// Data-directory-relative path, `file://` URL, or enabled closed-network Git URL.
    pub path: String,
    /// Git default branch. Ignored for `DIR` / `SNAPSHOT`.
    pub default_branch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CodeRepositorySummary {
    pub id: Uuid,
    pub kind: String,
    pub path: String,
    pub default_branch: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CodeRepositoryListResponse {
    pub items: Vec<CodeRepositorySummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SyncCodeRevisionRequest {
    pub repository_id: Uuid,
    pub revision: String,
    pub committed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SyncCodeRevisionResponse {
    pub job_id: Uuid,
    pub status: JobStatus,
    pub repository_id: Uuid,
    pub revision: String,
    pub links: JobLinks,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CodeRevisionSummary {
    pub id: Uuid,
    pub repository_id: Uuid,
    pub revision: String,
    pub parent_id: Option<Uuid>,
    pub tree_digest: String,
    pub is_baseline: bool,
    pub committed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub file_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CodeRevisionListResponse {
    pub items: Vec<CodeRevisionSummary>,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct CodeRevisionQuery {
    pub repository_id: Option<Uuid>,
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateCodeAnalysisRequest {
    pub revision_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CodeAnalysisAcceptedResponse {
    pub job_id: Uuid,
    pub run_id: Uuid,
    pub status: JobStatus,
    pub links: JobLinks,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CodeAnalysisRunResponse {
    pub id: Uuid,
    pub revision_id: Uuid,
    pub status: String,
    pub classpath_complete: bool,
    pub files_analyzed: i64,
    pub files_skipped: i64,
    pub files_failed: i64,
    pub symbols: i64,
    pub facts: i64,
    pub unresolved: i64,
    pub unresolved_rate: f64,
    pub degraded_reasons: Vec<String>,
    pub engine_versions: Value,
    pub stats: Value,
    pub error: Option<Value>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CodeAnalysisRunListResponse {
    pub items: Vec<CodeAnalysisRunResponse>,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct CodeSymbolQuery {
    pub revision_id: Option<Uuid>,
    pub fqn: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CodeSymbolSummary {
    pub id: Uuid,
    pub file_id: Uuid,
    pub kind: String,
    pub fqn: String,
    pub simple_name: String,
    pub signature: Option<String>,
    pub line_start: i32,
    pub line_end: i32,
    pub byte_start: i64,
    pub byte_end: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CodeSymbolListResponse {
    pub items: Vec<CodeSymbolSummary>,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct CodeFactQuery {
    pub revision_id: Option<Uuid>,
    pub subject: Option<String>,
    pub relation: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CodeEvidence {
    pub id: Uuid,
    pub role: String,
    pub quote: String,
    pub locator: Value,
    pub resolution: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CodeFactSummary {
    pub id: Uuid,
    pub revision_id: Uuid,
    pub subject_kind: String,
    pub subject_key: String,
    pub relation: String,
    pub object_kind: String,
    pub object_key: String,
    pub resolution: String,
    pub confidence: f32,
    pub analysis_engine: String,
    pub engine_version: String,
    pub unresolved_reason: Option<String>,
    pub candidates: Value,
    pub evidence: Vec<CodeEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CodeFactListResponse {
    pub items: Vec<CodeFactSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CodeSourceFileSummary {
    pub id: Uuid,
    pub path: String,
    pub language: String,
    pub size_bytes: i64,
    pub loc: i32,
    pub encoding: String,
    pub exact_offsets: bool,
    pub content_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CodeSourceFileListResponse {
    pub items: Vec<CodeSourceFileSummary>,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct CodeSourceContentQuery {
    pub start: Option<u64>,
    pub end: Option<u64>,
}
