//! HTTP 境界のエラーコード。
//!
//! 正本：`detail/04-api-contract.md` §0.1。
//! ここに無いコードを HTTP レスポンスへ出さない。Excel パーサーの
//! `E-XLSX-xxx` は内部コードで、SSE / ジョブの `errors[]` にだけ現れる。

use std::fmt;

/// 業務エラーコード。`code` フィールドになる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    ValidationSchema,
    AuthTokenExpired,
    AuthSsoUnavailable,
    AuthRateLimited,
    AuthLogoutTokenInvalid,
    ForbiddenProjectScope,
    NotFound,
    NotFoundDocument,
    NotFoundChunk,
    NotFoundEvidence,
    NotFoundJob,
    ConflictRevisionSuperseded,
    ConflictAlreadyImported,
    CapabilityUnsupported,
    ProviderTimeout,
    ProviderRateLimited,
    BudgetExceeded,
    JobCancelled,
    EvidenceQuoteMismatch,
    SchemaViolation,
    AgentCapabilityDenied,
    PolicyApprovalRequired,
    Internal,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ValidationSchema => "VALIDATION_SCHEMA",
            Self::AuthTokenExpired => "AUTH_TOKEN_EXPIRED",
            Self::AuthSsoUnavailable => "AUTH_SSO_UNAVAILABLE",
            Self::AuthRateLimited => "AUTH_RATE_LIMITED",
            Self::AuthLogoutTokenInvalid => "AUTH_LOGOUT_TOKEN_INVALID",
            Self::ForbiddenProjectScope => "FORBIDDEN_PROJECT_SCOPE",
            Self::NotFound => "NOT_FOUND",
            Self::NotFoundDocument => "NOT_FOUND_DOCUMENT",
            Self::NotFoundChunk => "NOT_FOUND_CHUNK",
            Self::NotFoundEvidence => "NOT_FOUND_EVIDENCE",
            Self::NotFoundJob => "NOT_FOUND_JOB",
            Self::ConflictRevisionSuperseded => "CONFLICT_REVISION_SUPERSEDED",
            Self::ConflictAlreadyImported => "CONFLICT_ALREADY_IMPORTED",
            Self::CapabilityUnsupported => "CAPABILITY_UNSUPPORTED",
            Self::ProviderTimeout => "PROVIDER_TIMEOUT",
            Self::ProviderRateLimited => "PROVIDER_RATE_LIMITED",
            Self::BudgetExceeded => "BUDGET_EXCEEDED",
            Self::JobCancelled => "JOB_CANCELLED",
            Self::EvidenceQuoteMismatch => "EVIDENCE_QUOTE_MISMATCH",
            Self::SchemaViolation => "SCHEMA_VIOLATION",
            Self::AgentCapabilityDenied => "AGENT_CAPABILITY_DENIED",
            Self::PolicyApprovalRequired => "POLICY_APPROVAL_REQUIRED",
            Self::Internal => "INTERNAL",
        }
    }

    /// HTTP status。表と 1:1 で対応させる。
    pub fn status(self) -> u16 {
        match self {
            Self::ValidationSchema => 400,
            Self::AuthTokenExpired => 401,
            Self::AuthSsoUnavailable => 503,
            Self::AuthRateLimited => 429,
            Self::AuthLogoutTokenInvalid => 400,
            Self::ForbiddenProjectScope => 403,
            Self::AgentCapabilityDenied => 403,
            Self::NotFound
            | Self::NotFoundDocument
            | Self::NotFoundChunk
            | Self::NotFoundEvidence
            | Self::NotFoundJob => 404,
            Self::ConflictRevisionSuperseded => 409,
            Self::ConflictAlreadyImported => 409,
            Self::CapabilityUnsupported => 409,
            Self::JobCancelled => 410,
            Self::BudgetExceeded => 422,
            Self::EvidenceQuoteMismatch => 422,
            Self::SchemaViolation => 422,
            Self::PolicyApprovalRequired => 423,
            Self::ProviderRateLimited => 429,
            Self::ProviderTimeout => 503,
            Self::Internal => 500,
        }
    }

    /// クライアントが再試行してよいか。`Retry-After` を付ける判断にも使う。
    pub fn retryable(self) -> bool {
        matches!(
            self,
            Self::ProviderTimeout | Self::ProviderRateLimited | Self::AuthRateLimited
        )
    }

    /// 既定の `title`。画面にそのまま出さない（画面は `code` と `trace_id` を出す。`07-UI.md`）。
    pub fn title(self) -> &'static str {
        match self {
            Self::ValidationSchema => "Invalid request",
            Self::AuthTokenExpired => "Authentication expired",
            Self::AuthSsoUnavailable => "SSO unavailable",
            Self::AuthRateLimited => "Rate limited",
            Self::AuthLogoutTokenInvalid => "Invalid logout token",
            Self::ForbiddenProjectScope => "Forbidden",
            Self::NotFound => "Not found",
            Self::NotFoundDocument => "Document not found",
            Self::NotFoundChunk => "Chunk not found",
            Self::NotFoundEvidence => "Evidence not found",
            Self::NotFoundJob => "Job not found",
            Self::ConflictRevisionSuperseded => "Conflict",
            Self::ConflictAlreadyImported => "Already imported",
            Self::CapabilityUnsupported => "Capability unsupported",
            Self::ProviderTimeout => "Upstream timeout",
            Self::ProviderRateLimited => "Rate limited",
            Self::BudgetExceeded => "Budget exceeded",
            Self::JobCancelled => "Job cancelled",
            Self::EvidenceQuoteMismatch => "Evidence quote mismatch",
            Self::SchemaViolation => "Schema violation",
            Self::AgentCapabilityDenied => "Capability denied",
            Self::PolicyApprovalRequired => "Approval required",
            Self::Internal => "Internal error",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_matches_contract_table() {
        assert_eq!(ErrorCode::ValidationSchema.status(), 400);
        assert_eq!(ErrorCode::AuthTokenExpired.status(), 401);
        assert_eq!(ErrorCode::AuthSsoUnavailable.status(), 503);
        assert_eq!(ErrorCode::AuthRateLimited.status(), 429);
        assert_eq!(ErrorCode::AuthLogoutTokenInvalid.status(), 400);
        assert_eq!(ErrorCode::ForbiddenProjectScope.status(), 403);
        assert_eq!(ErrorCode::NotFound.status(), 404);
        assert_eq!(ErrorCode::JobCancelled.status(), 410);
        assert_eq!(ErrorCode::EvidenceQuoteMismatch.status(), 422);
        assert_eq!(ErrorCode::PolicyApprovalRequired.status(), 423);
        assert_eq!(ErrorCode::ProviderRateLimited.status(), 429);
        assert_eq!(ErrorCode::ProviderTimeout.status(), 503);
    }

    #[test]
    fn only_upstream_failures_are_retryable() {
        assert!(ErrorCode::ProviderTimeout.retryable());
        assert!(ErrorCode::ProviderRateLimited.retryable());
        assert!(!ErrorCode::ValidationSchema.retryable());
        assert!(!ErrorCode::NotFound.retryable());
    }
}
