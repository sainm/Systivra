use chrono::{DateTime, Utc};
use domain::{DocumentId, JobId, ProjectId, RevisionId};
use serde::{Deserialize, Serialize};

use crate::doc_type::DocType;
use crate::job::{JobLinks, JobStatus};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BulkImportMode {
    #[default]
    Full,
    Incremental,
    Reindex,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BulkDuplicate {
    #[default]
    Skip,
    Fail,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct BulkImportRequest {
    pub manifest_url: String,
    #[serde(default)]
    pub mode: BulkImportMode,
    #[serde(default)]
    pub on_duplicate: BulkDuplicate,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BulkImportResponse {
    pub job_id: JobId,
    pub status: JobStatus,
    pub estimated_files: i64,
    pub links: JobLinks,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct UploadUrlRequest {
    pub filename: String,
    pub size_bytes: i64,
    pub mime_type: String,
    #[serde(default)]
    pub source_revision: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct UploadUrlResponse {
    pub upload_url: String,
    pub object_key: String,
    pub expires_at: DateTime<Utc>,
    pub max_size_bytes: i64,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct RegisterDocumentRequest {
    pub object_key: String,
    pub doc_type: DocType,
    pub title: String,
    #[serde(default)]
    pub business_domain: Option<String>,
    #[serde(default)]
    pub acl_groups: Vec<String>,
    #[serde(default)]
    pub source_revision: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RevisionSummary {
    pub id: RevisionId,
    pub source_revision: Option<String>,
    pub content_hash: String,
    pub status: String,
    pub acquired_at: DateTime<Utc>,
    pub parsed_at: Option<DateTime<Utc>>,
    pub file_size: i64,
    pub mime_type: String,
    pub page_count: i64,
    pub chunk_count: i64,
    pub parse_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct DocumentDetail {
    pub id: DocumentId,
    pub project_id: ProjectId,
    pub doc_type: DocType,
    pub title: String,
    pub business_domain: Option<String>,
    pub acl_groups: Vec<String>,
    pub current_revision: Option<RevisionSummary>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct NextLinks {
    pub parse: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RegisterDocumentResponse {
    pub document: DocumentDetail,
    pub next: NextLinks,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct DocumentSummary {
    pub id: DocumentId,
    pub title: String,
    pub doc_type: DocType,
    pub business_domain: Option<String>,
    pub current_revision_id: Option<RevisionId>,
    pub sheet_count: i64,
    pub page_count: i64,
    pub chunk_count: i64,
    pub parse_status: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct DocumentListResponse {
    pub items: Vec<DocumentSummary>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

#[derive(Debug, Clone, Default, Deserialize, utoipa::ToSchema)]
pub struct DocumentListQuery {
    pub doc_type: Option<DocType>,
    pub business_domain: Option<String>,
    pub status: Option<String>,
    pub q: Option<String>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
    pub include: Option<String>,
}
