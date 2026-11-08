use chrono::{DateTime, Utc};
use domain::{ChunkId, DocumentId, EvidenceId, JobId, RevisionId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::doc_type::DocType;
use crate::job::JobLinks;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum SearchMode {
    #[default]
    Hybrid,
    Lexical,
    Vector,
}

#[derive(Debug, Clone, Default, Deserialize, utoipa::ToSchema)]
pub struct SearchFilters {
    #[serde(default)]
    pub doc_types: Vec<DocType>,
    #[serde(default)]
    pub business_domain: Option<String>,
    #[serde(default)]
    pub revision_id: Option<RevisionId>,
}

/// 既存 chunk の `tokenized` を作り直す保守操作の結果。
/// `fts` は `tokenized` の生成列なので、更新すれば索引も追随する。
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RetokenizeResponse {
    /// 実際に値が変わった chunk 数。
    pub updated: usize,
    /// 今回処理対象にした chunk 数。
    pub examined: usize,
    /// まだ未処理の chunk 数。0 になるまで呼ぶ。
    pub remaining: usize,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct SearchRequest {
    pub query: String,
    #[serde(default)]
    pub mode: SearchMode,
    #[serde(default)]
    pub filters: SearchFilters,
    #[serde(default = "default_top_k")]
    pub top_k: i64,
    #[serde(default)]
    pub rerank: bool,
    #[serde(default)]
    pub graph_assist: bool,
    #[serde(default)]
    pub include: Vec<String>,
}

fn default_top_k() -> i64 {
    12
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SearchDocumentRef {
    pub id: DocumentId,
    pub title: String,
    pub doc_type: DocType,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SearchRevisionRef {
    pub id: RevisionId,
    pub source_revision: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SearchResult {
    pub chunk_id: ChunkId,
    pub document: SearchDocumentRef,
    pub revision: SearchRevisionRef,
    pub locator: serde_json::Value,
    pub quote: String,
    pub score: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rerank_score: Option<f64>,
    pub sources: Vec<String>,
    pub evidence_id: Option<EvidenceId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SearchResponse {
    pub search_id: String,
    pub lexical_mode: String,
    pub degraded: Vec<String>,
    pub results: Vec<SearchResult>,
    pub total_candidates: i64,
    pub latency_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ChunkNeighbors {
    pub prev: Vec<ChunkId>,
    pub next: Vec<ChunkId>,
    pub parent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ParseRunRef {
    pub parser: String,
    pub parser_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ChunkResponse {
    pub chunk_id: ChunkId,
    pub chunk_type: String,
    pub text: String,
    pub locator: serde_json::Value,
    pub token_count: i64,
    pub char_count: i64,
    pub neighbors: ChunkNeighbors,
    pub parse_run: ParseRunRef,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct EvidenceResponse {
    pub evidence_id: EvidenceId,
    pub source_type: String,
    pub document_id: Option<DocumentId>,
    pub revision_id: Option<RevisionId>,
    pub locator: serde_json::Value,
    pub quote: String,
    pub quote_hash: String,
    pub context_before: String,
    pub context_after: String,
    pub resolution: String,
    pub confidence: f32,
    pub document_title: Option<String>,
    pub mime_type: Option<String>,
    pub page_count: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AuditEntry {
    pub id: i64,
    pub actor_type: String,
    pub actor_id: String,
    pub action: String,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub result: String,
    pub trace_id: Option<String>,
    pub detail: serde_json::Value,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AuditListResponse {
    pub items: Vec<AuditEntry>,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct CreateIndexBuildRequest {
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct IndexBuildSummary {
    pub id: Uuid,
    pub model_id: Uuid,
    pub model_name: String,
    pub revision: String,
    pub status: String,
    pub chunk_count: i64,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub is_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct IndexBuildListResponse {
    pub items: Vec<IndexBuildSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateIndexBuildResponse {
    pub build: IndexBuildSummary,
    pub job_id: JobId,
    pub links: JobLinks,
}

#[derive(Debug, Clone, Default, Deserialize, utoipa::ToSchema)]
pub struct TreeQuery {
    pub revision_id: Option<RevisionId>,
    pub parent: Option<String>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct TreeNode {
    pub id: Uuid,
    pub kind: String,
    pub name: Option<String>,
    pub locator: Value,
    pub child_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct TreeResponse {
    pub nodes: Vec<TreeNode>,
    pub next_cursor: Option<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Default, Deserialize, utoipa::ToSchema)]
pub struct DocumentChunkQuery {
    pub revision_id: Option<RevisionId>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct DocumentChunkSummary {
    pub id: Uuid,
    pub chunk_type: String,
    pub ordinal: i32,
    pub text: String,
    pub locator: Value,
    pub token_count: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct DocumentChunkListResponse {
    pub items: Vec<DocumentChunkSummary>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
}
