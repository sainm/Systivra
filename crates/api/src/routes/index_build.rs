use app::Authenticated;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use contracts::{
    CreateIndexBuildRequest, CreateIndexBuildResponse, ErrorCode, IndexBuildListResponse,
    IndexBuildSummary,
};
use uuid::Uuid;

use crate::auth::ProjectScope;
use crate::error::ApiError;
use crate::trace::TraceId;
use crate::ApiState;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route(
            "/api/v1/projects/{pid}/index-builds",
            get(list_index_builds).post(create_index_build),
        )
        .route(
            "/api/v1/projects/{pid}/index-builds/{bid_activate}",
            post(activate_index_build),
        )
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/index-builds",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    responses((status = 200, body = IndexBuildListResponse), (status = 403, body = contracts::Problem))
)]
pub(crate) async fn list_index_builds(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
) -> Result<Json<IndexBuildListResponse>, ApiError> {
    let response = state
        .app
        .list_index_builds(&auth, scope.project)
        .await
        .map_err(|e| ApiError::from_app(e, &trace))?;
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{pid}/index-builds",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    request_body = CreateIndexBuildRequest,
    responses(
        (status = 202, body = CreateIndexBuildResponse),
        (status = 400, body = contracts::Problem),
        (status = 403, body = contracts::Problem)
    )
)]
pub(crate) async fn create_index_build(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Json(req): Json<CreateIndexBuildRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let response = state
        .app
        .create_index_build(&auth, scope.project, req, Some(trace.0.clone()))
        .await
        .map_err(|e| ApiError::from_app(e, &trace))?;
    Ok((StatusCode::ACCEPTED, Json(response)))
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{pid}/index-builds/{bid_activate}",
    params(
        ("pid" = uuid::Uuid, Path, description = "Project ID"),
        ("bid_activate" = String, Path, description = "Index build ID with :activate suffix")
    ),
    responses(
        (status = 200, body = IndexBuildSummary),
        (status = 400, body = contracts::Problem),
        (status = 403, body = contracts::Problem)
    )
)]
pub(crate) async fn activate_index_build(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path((_pid, bid)): Path<(String, String)>,
) -> Result<Json<IndexBuildSummary>, ApiError> {
    let raw = bid
        .strip_suffix(":activate")
        .ok_or_else(|| ApiError::new(ErrorCode::ValidationSchema, &trace))?;
    let build_id = raw
        .parse::<Uuid>()
        .map_err(|_| ApiError::new(ErrorCode::ValidationSchema, &trace))?;
    let response = state
        .app
        .activate_index_build(&auth, scope.project, build_id, Some(trace.0.clone()))
        .await
        .map_err(|e| ApiError::from_app(e, &trace))?;
    Ok(Json(response))
}
