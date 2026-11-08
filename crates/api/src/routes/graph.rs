use app::Authenticated;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use contracts::{
    ErrorCode, GraphEntityQuery, GraphEntitySearchResponse, GraphNeighborsQuery,
    GraphNeighborsResponse, GraphPathQuery, GraphPathResponse, GraphRebuildResponse,
    GraphStatsResponse, GraphSubgraphQuery, GraphSubgraphResponse, GraphTraceabilityQuery,
    OntologyResponse, PublishOntologyRequest, PublishOntologyResponse, TraceabilityResponse,
};
use uuid::Uuid;

use crate::auth::ProjectScope;
use crate::error::ApiError;
use crate::trace::TraceId;
use crate::ApiState;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/api/v1/projects/{pid}/graph/ontology", get(ontology))
        .route(
            "/api/v1/projects/{pid}/graph/ontology/publish",
            post(publish_ontology),
        )
        .route("/api/v1/projects/{pid}/graph/entities", get(entities))
        .route(
            "/api/v1/projects/{pid}/graph/entities/{eid}/neighbors",
            get(neighbors),
        )
        .route("/api/v1/projects/{pid}/graph/subgraph", get(subgraph))
        .route("/api/v1/projects/{pid}/graph/path", get(path))
        .route(
            "/api/v1/projects/{pid}/graph/traceability/{eid}",
            get(traceability),
        )
        .route("/api/v1/projects/{pid}/graph/stats", get(stats))
        .route("/api/v1/projects/{pid}/graph/rebuild", post(rebuild))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/graph/ontology",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    responses((status = 200, body = OntologyResponse), (status = 403, body = contracts::Problem))
)]
pub(crate) async fn ontology(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
) -> Result<Json<OntologyResponse>, ApiError> {
    let response = state
        .app
        .graph_ontology(&auth, scope.project)
        .await
        .map_err(|e| ApiError::from_app(e, &trace))?;
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{pid}/graph/ontology/publish",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    request_body = PublishOntologyRequest,
    responses(
        (status = 200, body = PublishOntologyResponse),
        (status = 403, body = contracts::Problem)
    )
)]
pub(crate) async fn publish_ontology(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Json(req): Json<PublishOntologyRequest>,
) -> Result<Json<PublishOntologyResponse>, ApiError> {
    let response = state
        .app
        .graph_publish_ontology(&auth, scope.project, &req.version)
        .await
        .map_err(|e| ApiError::from_app(e, &trace))?;
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/graph/entities",
    params(
        ("pid" = uuid::Uuid, Path, description = "Project ID"),
        ("q" = Option<String>, Query, description = "Name query"),
        ("entity_type" = Option<String>, Query, description = "Entity type"),
        ("limit" = Option<i64>, Query, description = "Page size")
    ),
    responses((status = 200, body = GraphEntitySearchResponse))
)]
pub(crate) async fn entities(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Query(query): Query<GraphEntityQuery>,
) -> Result<Json<GraphEntitySearchResponse>, ApiError> {
    let response = state
        .app
        .graph_search(
            &auth,
            scope.project,
            query.q.as_deref().unwrap_or(""),
            query.entity_type.as_deref(),
            query.limit.unwrap_or(30),
        )
        .await
        .map_err(|e| ApiError::from_app(e, &trace))?;
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/graph/entities/{eid}/neighbors",
    params(
        ("pid" = uuid::Uuid, Path, description = "Project ID"),
        ("eid" = uuid::Uuid, Path, description = "Entity ID"),
        ("rel_types" = Option<String>, Query, description = "Comma separated relation types"),
        ("limit" = Option<i64>, Query, description = "Page size")
    ),
    responses((status = 200, body = GraphNeighborsResponse))
)]
pub(crate) async fn neighbors(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path((_pid, eid)): Path<(String, String)>,
    Query(query): Query<GraphNeighborsQuery>,
) -> Result<Json<GraphNeighborsResponse>, ApiError> {
    let entity = parse_uuid(&eid, &trace)?;
    let rel_types = query
        .rel_types
        .as_deref()
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let response = state
        .app
        .graph_neighbors(
            &auth,
            scope.project,
            entity,
            rel_types,
            query.limit.unwrap_or(100),
        )
        .await
        .map_err(|e| ApiError::from_app(e, &trace))?;
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/graph/subgraph",
    params(
        ("pid" = uuid::Uuid, Path, description = "Project ID"),
        ("root" = uuid::Uuid, Query, description = "Root entity ID"),
        ("depth" = Option<i32>, Query, description = "Hops (1-3)"),
        ("limit" = Option<i64>, Query, description = "Node limit")
    ),
    responses((status = 200, body = GraphSubgraphResponse))
)]
pub(crate) async fn subgraph(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Query(query): Query<GraphSubgraphQuery>,
) -> Result<Json<GraphSubgraphResponse>, ApiError> {
    let response = state
        .app
        .graph_subgraph(
            &auth,
            scope.project,
            query.root,
            query.depth.unwrap_or(2),
            query.limit.unwrap_or(200),
        )
        .await
        .map_err(|e| ApiError::from_app(e, &trace))?;
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/graph/path",
    params(
        ("pid" = uuid::Uuid, Path, description = "Project ID"),
        ("src" = uuid::Uuid, Query, description = "Source entity"),
        ("dst" = uuid::Uuid, Query, description = "Destination entity"),
        ("max_depth" = Option<i32>, Query, description = "Maximum hops")
    ),
    responses((status = 200, body = GraphPathResponse), (status = 404, body = contracts::Problem))
)]
pub(crate) async fn path(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Query(query): Query<GraphPathQuery>,
) -> Result<Json<GraphPathResponse>, ApiError> {
    let response = state
        .app
        .graph_path(
            &auth,
            scope.project,
            query.src,
            query.dst,
            query.max_depth.unwrap_or(3),
        )
        .await
        .map_err(|e| ApiError::from_app(e, &trace))?;
    response
        .map(Json)
        .ok_or_else(|| ApiError::new(ErrorCode::NotFound, &trace).detail("経路がありません"))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/graph/traceability/{eid}",
    params(
        ("pid" = uuid::Uuid, Path, description = "Project ID"),
        ("eid" = uuid::Uuid, Path, description = "Entity ID"),
        ("depth" = Option<i32>, Query, description = "Hops"),
        ("limit" = Option<i64>, Query, description = "Item limit")
    ),
    responses((status = 200, body = TraceabilityResponse))
)]
pub(crate) async fn traceability(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path((_pid, eid)): Path<(String, String)>,
    Query(query): Query<GraphTraceabilityQuery>,
) -> Result<Json<TraceabilityResponse>, ApiError> {
    let entity = parse_uuid(&eid, &trace)?;
    let response = state
        .app
        .graph_traceability(
            &auth,
            scope.project,
            entity,
            query.depth.unwrap_or(3),
            query.limit.unwrap_or(100),
        )
        .await
        .map_err(|e| ApiError::from_app(e, &trace))?;
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/graph/stats",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    responses((status = 200, body = GraphStatsResponse))
)]
pub(crate) async fn stats(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
) -> Result<impl IntoResponse, ApiError> {
    let response = state
        .app
        .graph_stats(&auth, scope.project)
        .await
        .map_err(|e| ApiError::from_app(e, &trace))?;
    Ok((StatusCode::OK, Json(response)))
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{pid}/graph/rebuild",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    responses((status = 200, body = GraphRebuildResponse), (status = 403, body = contracts::Problem))
)]
pub(crate) async fn rebuild(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
) -> Result<Json<GraphRebuildResponse>, ApiError> {
    let response = state
        .app
        .graph_rebuild(&auth, scope.project)
        .await
        .map_err(|e| ApiError::from_app(e, &trace))?;
    Ok(Json(response))
}

fn parse_uuid(raw: &str, trace: &TraceId) -> Result<Uuid, ApiError> {
    raw.parse()
        .map_err(|_| ApiError::new(ErrorCode::ValidationSchema, trace))
}
