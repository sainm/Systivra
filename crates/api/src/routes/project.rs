//! プロジェクトスコープのエンドポイント。
//!
//! ここに来る時点で `ProjectScope` が通っている＝所属が確認済み。
//! ハンドラで再度 `project_id` を検証しない（二重実装は必ずずれる）。

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use contracts::ProjectMeResponse;

use crate::auth::ProjectScope;
use crate::routes::auth::writable_permissions;
use crate::ApiState;

pub fn router() -> Router<ApiState> {
    Router::new().route("/api/v1/projects/{pid}/me", get(project_me))
}

/// このプロジェクトでの自分の立場と、書ける操作の一覧。
async fn project_me(
    State(_state): State<ApiState>,
    scope: ProjectScope,
) -> Json<ProjectMeResponse> {
    Json(ProjectMeResponse {
        project_id: scope.project,
        role: scope.role,
        permissions: writable_permissions(scope.role),
    })
}
