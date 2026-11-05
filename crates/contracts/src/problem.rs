//! `application/problem+json`（RFC 7807）の封筒。
//!
//! 正本：`detail/04-api-contract.md` §0.1。
//! `code` / `trace_id` / `errors[]` は**必ず**入れる。画面はこの 3 つだけを使う。

use serde::{Deserialize, Serialize};

use crate::error_code::ErrorCode;

/// 項目単位のエラー。`field` は JSON Pointer か API のフィールド名。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FieldError {
    pub field: String,
    pub code: String,
    pub message: String,
}

impl FieldError {
    pub fn new(
        field: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            field: field.into(),
            code: code.into(),
            message: message.into(),
        }
    }
}

/// エラー応答の本体。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Problem {
    /// `https://systivra.example/errors/<CODE>`
    #[serde(rename = "type")]
    pub type_uri: String,
    pub title: String,
    pub status: u16,
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    pub trace_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<FieldError>,
}

impl Problem {
    /// `trace_id` は必須。空文字を渡せる余地を残さないため引数で受ける。
    pub fn new(code: ErrorCode, trace_id: impl Into<String>) -> Self {
        Self {
            type_uri: format!("https://systivra.example/errors/{}", code.as_str()),
            title: code.title().to_string(),
            status: code.status(),
            code: code.as_str().to_string(),
            detail: None,
            instance: None,
            trace_id: trace_id.into(),
            errors: Vec::new(),
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn with_instance(mut self, instance: impl Into<String>) -> Self {
        self.instance = Some(instance.into());
        self
    }

    pub fn with_errors(mut self, errors: Vec<FieldError>) -> Self {
        self.errors = errors;
        self
    }

    /// メディアタイプ。axum 側でこれを返す。
    pub const CONTENT_TYPE: &'static str = "application/problem+json";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_to_the_contracted_shape() {
        let p = Problem::new(ErrorCode::ValidationSchema, "4bf92f35")
            .with_detail("doc_type が不正です")
            .with_instance("/api/v1/projects/8f2c/documents")
            .with_errors(vec![FieldError::new(
                "doc_type",
                "UNKNOWN_VALUE",
                "doc_type は列挙値ではありません",
            )]);

        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(
            v["type"],
            "https://systivra.example/errors/VALIDATION_SCHEMA"
        );
        assert_eq!(v["status"], 400);
        assert_eq!(v["code"], "VALIDATION_SCHEMA");
        assert_eq!(v["trace_id"], "4bf92f35");
        assert_eq!(v["errors"][0]["field"], "doc_type");
    }

    #[test]
    fn optional_fields_are_omitted_not_null() {
        let p = Problem::new(ErrorCode::NotFound, "t");
        let v = serde_json::to_value(&p).unwrap();
        assert!(v.get("detail").is_none());
        assert!(v.get("instance").is_none());
        assert!(v.get("errors").is_none());
    }
}
