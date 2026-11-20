//! Retrieval Engine の HTTP 入口。
//!
//! **ここには検索ロジックを書かない。** 受け取って `app::retrieval` に渡し、
//! 結果をそのまま返すだけ。MCP Adapter も同じ方針で、この API を呼ぶ。

use app::Authenticated;
use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Extension, Json, Router};
use contracts::{RetrievalRequest, RetrievalResponse, RetrievalRunListResponse};
use serde::Deserialize;

use crate::auth::ProjectScope;
use crate::error::ApiError;
use crate::trace::TraceId;
use crate::ApiState;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route(
            "/api/v1/projects/{pid}/retrieval:query",
            axum::routing::post(retrieval_query),
        )
        .route("/api/v1/projects/{pid}/retrieval-runs", get(list_runs))
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct RunListQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    20
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{pid}/retrieval:query",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    request_body = RetrievalRequest,
    responses(
        (status = 200, body = RetrievalResponse),
        (status = 400, body = contracts::Problem),
        (status = 403, body = contracts::Problem)
    )
)]
/// 統一検索。戦略は指定も自動選択もできる。
pub(crate) async fn retrieval_query(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Json(request): Json<RetrievalRequest>,
) -> Result<Json<RetrievalResponse>, ApiError> {
    let response = state
        .app
        .retrieval_query(&auth, scope.project, request)
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/retrieval-runs",
    params(("pid" = uuid::Uuid, Path, description = "Project ID"), RunListQuery),
    responses(
        (status = 200, body = RetrievalRunListResponse),
        (status = 403, body = contracts::Problem)
    )
)]
/// 実行記録の一覧（trace を見る入口）。
pub(crate) async fn list_runs(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Query(query): Query<RunListQuery>,
) -> Result<Json<RetrievalRunListResponse>, ApiError> {
    let response = state
        .app
        .retrieval_runs(&auth, scope.project, query.limit)
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok(Json(response))
}
