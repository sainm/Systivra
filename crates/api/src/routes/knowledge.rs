use app::Authenticated;
use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Extension, Json, Router};
use contracts::{
    ChunkResponse, ErrorCode, EvidenceResponse, RetokenizeResponse, SearchRequest, SearchResponse,
};
use domain::{ChunkId, EvidenceId};

use crate::auth::ProjectScope;
use crate::error::ApiError;
use crate::trace::TraceId;
use crate::ApiState;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route(
            "/api/v1/projects/{pid}/knowledge/search",
            axum::routing::post(search),
        )
        .route(
            "/api/v1/projects/{pid}/knowledge/retokenize",
            axum::routing::post(retokenize),
        )
        .route("/api/v1/chunks/{cid}", get(chunk))
        .route("/api/v1/evidence/{eid}", get(evidence))
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{pid}/knowledge/retokenize",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    responses(
        (status = 200, body = RetokenizeResponse),
        (status = 403, body = contracts::Problem)
    )
)]
/// 保存済み chunk の `tokenized` を現行 tokenizer で作り直す保守操作。
/// `fts` は生成列なので、これだけで日本語の語彙が索引に載る。
pub(crate) async fn retokenize(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
) -> Result<Json<RetokenizeResponse>, ApiError> {
    let response = state
        .app
        .retokenize_chunks(&auth, scope.project, Some(trace.0.clone()))
        .await
        .map_err(|e| ApiError::from_app(e, &trace))?;
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{pid}/knowledge/search",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    request_body = SearchRequest,
    responses(
        (status = 200, body = SearchResponse),
        (status = 400, body = contracts::Problem),
        (status = 409, body = contracts::Problem)
    )
)]
pub(crate) async fn search(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Json(req): Json<SearchRequest>,
) -> Result<Json<SearchResponse>, ApiError> {
    let response = state
        .app
        .search(&auth, scope.project, req)
        .await
        .map_err(|e| ApiError::from_app(e, &trace))?;
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/api/v1/chunks/{cid}",
    params(("cid" = uuid::Uuid, Path, description = "Chunk ID")),
    responses((status = 200, body = ChunkResponse), (status = 404, body = contracts::Problem))
)]
pub(crate) async fn chunk(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path(cid): Path<String>,
) -> Result<Json<ChunkResponse>, ApiError> {
    let chunk_id: ChunkId = cid
        .parse()
        .map_err(|_| ApiError::new(ErrorCode::NotFoundChunk, &trace))?;
    let response = state
        .app
        .chunk_for_user(&auth, chunk_id)
        .await
        .map_err(|e| ApiError::from_app(e, &trace))?;
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/api/v1/evidence/{eid}",
    params(("eid" = uuid::Uuid, Path, description = "Evidence ID")),
    responses((status = 200, body = EvidenceResponse), (status = 404, body = contracts::Problem))
)]
pub(crate) async fn evidence(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path(eid): Path<String>,
) -> Result<Json<EvidenceResponse>, ApiError> {
    let evidence_id: EvidenceId = eid
        .parse()
        .map_err(|_| ApiError::new(ErrorCode::NotFoundEvidence, &trace))?;
    let response = state
        .app
        .evidence_for_user(&auth, evidence_id)
        .await
        .map_err(|e| ApiError::from_app(e, &trace))?;
    Ok(Json(response))
}
