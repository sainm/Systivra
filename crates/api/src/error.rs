//! エラー応答。`application/problem+json` に固定する。
//!
//! 画面が使うのは `code` と `trace_id` だけ（`07-UI.md`）。
//! `title` / `detail` は開発者とログのためのもの。

use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use contracts::{ErrorCode, FieldError, Problem};

use crate::trace::TraceId;

/// ハンドラが返す業務エラー。
#[derive(Debug)]
pub struct ApiError {
    code: ErrorCode,
    trace_id: String,
    detail: Option<String>,
    errors: Vec<FieldError>,
}

impl ApiError {
    pub fn new(code: ErrorCode, trace: &TraceId) -> Self {
        Self {
            code,
            trace_id: trace.as_str().to_owned(),
            detail: None,
            errors: Vec::new(),
        }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn field_error(mut self, e: FieldError) -> Self {
        self.errors.push(e);
        self
    }

    pub fn code(&self) -> ErrorCode {
        self.code
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut p = Problem::new(self.code, self.trace_id);
        if let Some(d) = self.detail {
            p = p.with_detail(d);
        }
        if !self.errors.is_empty() {
            p = p.with_errors(self.errors);
        }

        let status =
            StatusCode::from_u16(self.code.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

        (
            status,
            [(header::CONTENT_TYPE, Problem::CONTENT_TYPE)],
            axum::Json(p),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_problem_content_type() {
        let e = ApiError::new(ErrorCode::NotFound, &TraceId("t".into()));
        let res = e.into_response();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        assert_eq!(res.headers()[header::CONTENT_TYPE], Problem::CONTENT_TYPE);
    }

    #[test]
    fn maps_status_from_code() {
        let e = ApiError::new(ErrorCode::ProviderTimeout, &TraceId("t".into()));
        assert_eq!(e.code().status(), 503);
    }
}
