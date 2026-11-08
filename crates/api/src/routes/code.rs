use app::Authenticated;
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use contracts::{
    CodeAnalysisAcceptedResponse, CodeAnalysisRunListResponse, CodeAnalysisRunResponse,
    CodeFactListResponse, CodeFactQuery, CodeRepositoryListResponse, CodeRepositorySummary,
    CodeRevisionListResponse, CodeRevisionQuery, CodeSourceFileListResponse,
    CodeSymbolListResponse, CodeSymbolQuery, CreateCodeAnalysisRequest,
    CreateCodeRepositoryRequest, ErrorCode, SyncCodeRevisionRequest, SyncCodeRevisionResponse,
};
use uuid::Uuid;

use crate::auth::ProjectScope;
use crate::error::ApiError;
use crate::trace::TraceId;
use crate::ApiState;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route(
            "/api/v1/projects/{pid}/code/repositories",
            get(list_repositories).post(create_repository),
        )
        .route(
            "/api/v1/projects/{pid}/code/revisions:sync",
            post(sync_revision),
        )
        .route("/api/v1/projects/{pid}/code/revisions", get(list_revisions))
        .route(
            "/api/v1/projects/{pid}/code/revisions/{rid}/files",
            get(list_source_files),
        )
        .route(
            "/api/v1/projects/{pid}/code/analysis-runs",
            get(list_analysis_runs).post(create_analysis_run),
        )
        .route("/api/v1/projects/{pid}/code/symbols", get(list_symbols))
        .route("/api/v1/projects/{pid}/code/facts", get(list_facts))
        .route("/api/v1/code/analysis-runs/{rid}", get(analysis_run))
        .route("/api/v1/code/files/{fid}/content", get(source_content))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/code/repositories",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    responses((status = 200, body = CodeRepositoryListResponse), (status = 403, body = contracts::Problem))
)]
pub(crate) async fn list_repositories(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
) -> Result<Json<CodeRepositoryListResponse>, ApiError> {
    let response = state
        .app
        .code_repositories(&auth, scope.project)
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{pid}/code/repositories",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    request_body = CreateCodeRepositoryRequest,
    responses(
        (status = 201, body = CodeRepositorySummary),
        (status = 400, body = contracts::Problem),
        (status = 403, body = contracts::Problem)
    )
)]
pub(crate) async fn create_repository(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Json(request): Json<CreateCodeRepositoryRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let response = state
        .app
        .create_code_repository(&auth, scope.project, request, Some(trace.0.clone()))
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok((StatusCode::CREATED, Json(response)))
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{pid}/code/revisions:sync",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    request_body = SyncCodeRevisionRequest,
    responses(
        (status = 202, body = SyncCodeRevisionResponse),
        (status = 400, body = contracts::Problem),
        (status = 403, body = contracts::Problem)
    )
)]
pub(crate) async fn sync_revision(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Json(request): Json<SyncCodeRevisionRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let response = state
        .app
        .enqueue_code_sync(&auth, scope.project, request, Some(trace.0.clone()))
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok((StatusCode::ACCEPTED, Json(response)))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/code/revisions",
    params(
        ("pid" = uuid::Uuid, Path, description = "Project ID"),
        ("repository_id" = Option<uuid::Uuid>, Query, description = "Repository ID"),
        ("limit" = Option<i64>, Query, description = "Page size")
    ),
    responses((status = 200, body = CodeRevisionListResponse))
)]
pub(crate) async fn list_revisions(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Query(query): Query<CodeRevisionQuery>,
) -> Result<Json<CodeRevisionListResponse>, ApiError> {
    let response = state
        .app
        .code_revisions(
            &auth,
            scope.project,
            query.repository_id,
            query.limit.unwrap_or(50),
        )
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/code/revisions/{rid}/files",
    params(
        ("pid" = uuid::Uuid, Path, description = "Project ID"),
        ("rid" = uuid::Uuid, Path, description = "Code revision ID")
    ),
    responses((status = 200, body = CodeSourceFileListResponse))
)]
pub(crate) async fn list_source_files(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path((_pid, rid)): Path<(String, String)>,
) -> Result<Json<CodeSourceFileListResponse>, ApiError> {
    let revision_id = parse_uuid(&rid, &trace)?;
    let response = state
        .app
        .code_source_files(&auth, scope.project, revision_id)
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/code/analysis-runs",
    params(
        ("pid" = uuid::Uuid, Path, description = "Project ID"),
        ("limit" = Option<i64>, Query, description = "Page size")
    ),
    responses((status = 200, body = CodeAnalysisRunListResponse))
)]
pub(crate) async fn list_analysis_runs(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Query(query): Query<AnalysisRunListQuery>,
) -> Result<Json<CodeAnalysisRunListResponse>, ApiError> {
    let response = state
        .app
        .code_analysis_runs(&auth, scope.project, query.limit.unwrap_or(20))
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok(Json(response))
}

#[derive(Debug, Clone, serde::Deserialize, utoipa::ToSchema)]
pub struct AnalysisRunListQuery {
    pub limit: Option<i64>,
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{pid}/code/analysis-runs",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    request_body = CreateCodeAnalysisRequest,
    responses(
        (status = 202, body = CodeAnalysisAcceptedResponse),
        (status = 400, body = contracts::Problem),
        (status = 403, body = contracts::Problem)
    )
)]
pub(crate) async fn create_analysis_run(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Json(request): Json<CreateCodeAnalysisRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let response = state
        .app
        .enqueue_code_analysis(&auth, scope.project, request, Some(trace.0.clone()))
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok((StatusCode::ACCEPTED, Json(response)))
}

#[utoipa::path(
    get,
    path = "/api/v1/code/analysis-runs/{rid}",
    params(("rid" = uuid::Uuid, Path, description = "Analysis run ID")),
    responses((status = 200, body = CodeAnalysisRunResponse), (status = 404, body = contracts::Problem))
)]
pub(crate) async fn analysis_run(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path(rid): Path<String>,
) -> Result<Json<CodeAnalysisRunResponse>, ApiError> {
    let run_id = parse_uuid(&rid, &trace)?;
    let project = state
        .app
        .project_for(auth.user_id, "code_analysis", run_id)
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    let response = state
        .app
        .code_analysis_run(&auth, project, run_id)
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/code/symbols",
    params(
        ("pid" = uuid::Uuid, Path, description = "Project ID"),
        ("revision_id" = Option<uuid::Uuid>, Query, description = "Code revision ID"),
        ("fqn" = Option<String>, Query, description = "FQN fragment"),
        ("limit" = Option<i64>, Query, description = "Page size")
    ),
    responses((status = 200, body = CodeSymbolListResponse))
)]
pub(crate) async fn list_symbols(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Query(query): Query<CodeSymbolQuery>,
) -> Result<Json<CodeSymbolListResponse>, ApiError> {
    let response = state
        .app
        .code_symbols(
            &auth,
            scope.project,
            query.revision_id,
            query.fqn.as_deref().unwrap_or(""),
            query.limit.unwrap_or(100),
        )
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/code/facts",
    params(
        ("pid" = uuid::Uuid, Path, description = "Project ID"),
        ("revision_id" = Option<uuid::Uuid>, Query, description = "Code revision ID"),
        ("subject" = Option<String>, Query, description = "Subject or object fragment"),
        ("relation" = Option<String>, Query, description = "Ontology relation"),
        ("limit" = Option<i64>, Query, description = "Page size")
    ),
    responses((status = 200, body = CodeFactListResponse))
)]
pub(crate) async fn list_facts(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Query(query): Query<CodeFactQuery>,
) -> Result<Json<CodeFactListResponse>, ApiError> {
    let response = state
        .app
        .code_facts(
            &auth,
            scope.project,
            query.revision_id,
            query.subject.as_deref().unwrap_or(""),
            query.relation.as_deref().unwrap_or(""),
            query.limit.unwrap_or(100),
        )
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/api/v1/code/files/{fid}/content",
    params(("fid" = uuid::Uuid, Path, description = "Source file ID")),
    responses((status = 200, description = "Original source bytes"), (status = 404, body = contracts::Problem))
)]
pub(crate) async fn source_content(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path(fid): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let file_id = parse_uuid(&fid, &trace)?;
    let project = state
        .app
        .project_for(auth.user_id, "code_source_file", file_id)
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    let bytes = state
        .app
        .code_source_file_content(&auth, project, file_id)
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_string()),
            (header::CACHE_CONTROL, "private, max-age=60".to_string()),
        ],
        bytes,
    ))
}

fn parse_uuid(raw: &str, trace: &TraceId) -> Result<Uuid, ApiError> {
    raw.parse()
        .map_err(|_| ApiError::new(ErrorCode::ValidationSchema, trace))
}
