//! プロジェクトスコープのエンドポイント。
//!
//! ここに来る時点で `ProjectScope` が通っている＝所属が確認済み。
//! ハンドラで再度 `project_id` を検証しない（二重実装は必ずずれる）。

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, patch};
use axum::{Extension, Json, Router};
use contracts::{
    ErrorCode, MemberListResponse, MemberSummary, ProjectMeResponse, UpdateMemberRoleRequest,
};
use domain::{Role, UserId};

use crate::auth::ProjectScope;
use crate::error::ApiError;
use crate::routes::auth::writable_permissions;
use crate::trace::TraceId;
use crate::ApiState;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/api/v1/projects/{pid}/me", get(project_me))
        .route("/api/v1/projects/{pid}/members", get(list_members))
        .route(
            "/api/v1/projects/{pid}/members/{uid}",
            patch(update_member_role),
        )
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/members",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    responses((status = 200, body = MemberListResponse), (status = 403, body = contracts::Problem))
)]
pub(crate) async fn list_members(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
) -> Result<Json<MemberListResponse>, ApiError> {
    if !matches!(scope.role, Role::Owner | Role::Architect) {
        return Err(ApiError::new(ErrorCode::ForbiddenProjectScope, &trace));
    }
    let members = state.app.list_members(scope.project).await.map_err(|e| {
        tracing::error!(error = %e, "member list failed");
        ApiError::new(ErrorCode::Internal, &trace)
    })?;
    Ok(Json(MemberListResponse {
        members: members
            .into_iter()
            .map(|member| MemberSummary {
                user_id: member.user_id,
                login_id: member.login_id,
                display_name: member.display_name,
                email: member.email,
                is_active: member.is_active,
                role: member.role,
            })
            .collect(),
    }))
}

#[utoipa::path(
    patch,
    path = "/api/v1/projects/{pid}/members/{uid}",
    params(
        ("pid" = uuid::Uuid, Path, description = "Project ID"),
        ("uid" = uuid::Uuid, Path, description = "User ID")
    ),
    request_body = UpdateMemberRoleRequest,
    responses((status = 204), (status = 403, body = contracts::Problem), (status = 404, body = contracts::Problem))
)]
pub(crate) async fn update_member_role(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Path((_pid, raw_uid)): Path<(String, String)>,
    Json(request): Json<UpdateMemberRoleRequest>,
) -> Result<StatusCode, ApiError> {
    if scope.role != Role::Owner {
        return Err(ApiError::new(ErrorCode::ForbiddenProjectScope, &trace));
    }
    let user: UserId = raw_uid
        .parse()
        .map_err(|_| ApiError::new(ErrorCode::NotFound, &trace))?;
    let members = state.app.list_members(scope.project).await.map_err(|e| {
        tracing::error!(error = %e, "member list failed");
        ApiError::new(ErrorCode::Internal, &trace)
    })?;
    if !members.iter().any(|member| member.user_id == user) {
        return Err(ApiError::new(ErrorCode::NotFound, &trace));
    }
    state
        .app
        .set_member_role(scope.project, user, request.role)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "member role update failed");
            ApiError::new(ErrorCode::Internal, &trace)
        })?;
    Ok(StatusCode::NO_CONTENT)
}

/// このプロジェクトでの自分の立場と、書ける操作の一覧。
#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/me",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    responses((status = 200, body = ProjectMeResponse), (status = 404, body = contracts::Problem))
)]
pub(crate) async fn project_me(
    State(_state): State<ApiState>,
    scope: ProjectScope,
) -> Json<ProjectMeResponse> {
    Json(ProjectMeResponse {
        project_id: scope.project,
        role: scope.role,
        permissions: writable_permissions(scope.role),
    })
}
