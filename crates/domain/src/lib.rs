//! 領域型と純ロジック。
//!
//! この crate は **IO を持たない**（`02-アーキテクチャ` §3）。DB も HTTP も知らない。
//! ここに置けるのは、値の定義・不変条件・決定的な計算だけ。

pub mod ids;
pub mod permission;
pub mod project;

pub use ids::{DocumentId, JobId, ProjectId, UserId, WorkspaceId};
pub use permission::{Access, Permission};
pub use project::{Project, Role, SourceKind};
