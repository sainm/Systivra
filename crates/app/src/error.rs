use contracts::ErrorCode;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("store error: {0}")]
    Store(#[from] store::StoreError),

    #[error("auth error: {0}")]
    Auth(#[from] crate::AuthError),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("validation: {0}")]
    Validation(String),

    #[error("not found: {0:?}")]
    NotFound(ErrorCode),

    #[error("conflict: {0:?}")]
    Conflict(ErrorCode),

    #[error("capability unsupported: {0}")]
    CapabilityUnsupported(String),

    #[error("forbidden: {0}")]
    Forbidden(String),
}

impl AppError {
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::Validation(_) => ErrorCode::ValidationSchema,
            Self::NotFound(code) => *code,
            Self::Conflict(code) => *code,
            Self::CapabilityUnsupported(_) => ErrorCode::CapabilityUnsupported,
            Self::Forbidden(_) => ErrorCode::ForbiddenProjectScope,
            Self::Store(_) | Self::Auth(_) | Self::Io(_) => ErrorCode::Internal,
        }
    }

    pub fn detail(&self) -> String {
        self.to_string()
    }
}
