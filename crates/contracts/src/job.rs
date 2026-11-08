use chrono::{DateTime, Utc};
use domain::JobId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum JobStatus {
    Pending,
    Queued,
    Running,
    WaitingApproval,
    Succeeded,
    SucceededWithErrors,
    Failed,
    Cancelled,
    Timeout,
}

impl JobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Queued => "QUEUED",
            Self::Running => "RUNNING",
            Self::WaitingApproval => "WAITING_APPROVAL",
            Self::Succeeded => "SUCCEEDED",
            Self::SucceededWithErrors => "SUCCEEDED_WITH_ERRORS",
            Self::Failed => "FAILED",
            Self::Cancelled => "CANCELLED",
            Self::Timeout => "TIMEOUT",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "PENDING" => Self::Pending,
            "QUEUED" => Self::Queued,
            "RUNNING" => Self::Running,
            "WAITING_APPROVAL" => Self::WaitingApproval,
            "SUCCEEDED" => Self::Succeeded,
            "SUCCEEDED_WITH_ERRORS" => Self::SucceededWithErrors,
            "FAILED" => Self::Failed,
            "CANCELLED" => Self::Cancelled,
            "TIMEOUT" => Self::Timeout,
            _ => return None,
        })
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded
                | Self::SucceededWithErrors
                | Self::Failed
                | Self::Cancelled
                | Self::Timeout
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct JobProgress {
    pub phase: String,
    pub files_done: i64,
    pub files_total: i64,
    pub cells_done: i64,
    pub cells_total: i64,
    pub chunks_done: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct JobErrorItem {
    pub path: Option<String>,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct JobLinks {
    pub events: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct JobResponse {
    pub job_id: JobId,
    pub kind: String,
    pub status: JobStatus,
    pub progress: JobProgress,
    pub started_at: Option<DateTime<Utc>>,
    pub eta_seconds: Option<i64>,
    pub errors: Vec<JobErrorItem>,
    pub links: JobLinks,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ParseAcceptedResponse {
    pub job_id: JobId,
    pub status: JobStatus,
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct JobEventPayload {
    pub id: i64,
    pub event: String,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct IdempotentJob {
    pub job_id: JobId,
    pub status: JobStatus,
    pub created: bool,
}
