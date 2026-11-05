//! 外部と交わす形（DTO / Schema）。HTTP とジョブの境界はここで固定する。
//!
//! 方針：`serde` の rename を 1 箇所に集め、フィールド名の揺れを構造的に防ぐ。
//! 契約の正本は `detail/04-api-contract.md`。

pub mod auth;
pub mod doc_type;
pub mod error_code;
pub mod problem;

pub use auth::{LoginRequest, LoginResponse, MeResponse, MembershipSummary, ProjectMeResponse};
pub use doc_type::DocType;
pub use error_code::ErrorCode;
pub use problem::{FieldError, Problem};
