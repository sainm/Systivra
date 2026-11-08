//! Phase 6 の HTTP 境界。
//!
//! すべて project-scoped（`ProjectScope`）にした。リソース単独パスでの
//! `project_for` 逆引きを増やさず、RLS のスコープを 1 箇所に閉じる。

use app::Authenticated;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use contracts::{
    ApproveTestCaseResponse, ArtifactCleanupResponse, CoverageView, CreateDefectRequest,
    CreateTestRunRequest, CreateTestRunResponse, DefectCandidateView, DeriveTestPlanRequest,
    DeriveTestPlanResponse, ErrorCode, ExpireArtifactsRequest, ExtractRulesRequest,
    ExtractRulesResponse, ImportExecutionsRequest, ImportExecutionsResponse,
    RecomputeCoverageRequest, TestOverviewResponse,
};
use uuid::Uuid;

use crate::auth::ProjectScope;
use crate::error::ApiError;
use crate::trace::TraceId;
use crate::ApiState;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route(
            "/api/v1/projects/{pid}/test/rules:extract",
            post(extract_rules),
        )
        .route(
            "/api/v1/projects/{pid}/test/plans:derive",
            post(derive_plan),
        )
        .route("/api/v1/projects/{pid}/test/overview", get(overview))
        .route(
            "/api/v1/projects/{pid}/test/cases/{cid}/approve",
            post(approve_case),
        )
        .route("/api/v1/projects/{pid}/test/runs", post(create_run))
        .route(
            "/api/v1/projects/{pid}/test/runs/{rid}/executions:import",
            post(import_executions),
        )
        .route(
            "/api/v1/projects/{pid}/test/artifacts:expire",
            post(expire_artifacts),
        )
        .route(
            "/api/v1/projects/{pid}/test/runs/{rid}/artifacts:cleanup",
            post(cleanup_run_artifacts),
        )
        .route(
            "/api/v1/projects/{pid}/test/coverage:recompute",
            post(recompute_coverage),
        )
        .route("/api/v1/projects/{pid}/test/defects", post(create_defect))
}

fn parse_uuid(raw: &str, trace: &TraceId) -> Result<Uuid, ApiError> {
    raw.parse()
        .map_err(|_| ApiError::new(ErrorCode::ValidationSchema, trace))
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{pid}/test/rules:extract",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    request_body = ExtractRulesRequest,
    responses(
        (status = 200, body = ExtractRulesResponse),
        (status = 400, body = contracts::Problem),
        (status = 403, body = contracts::Problem)
    )
)]
pub(crate) async fn extract_rules(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Json(request): Json<ExtractRulesRequest>,
) -> Result<Json<ExtractRulesResponse>, ApiError> {
    let response = state
        .app
        .extract_test_rules(&auth, scope.project, request, Some(trace.0.clone()))
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{pid}/test/plans:derive",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    request_body = DeriveTestPlanRequest,
    responses(
        (status = 200, body = DeriveTestPlanResponse),
        (status = 404, body = contracts::Problem)
    )
)]
pub(crate) async fn derive_plan(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Json(request): Json<DeriveTestPlanRequest>,
) -> Result<Json<DeriveTestPlanResponse>, ApiError> {
    let response = state
        .app
        .derive_test_plan(&auth, scope.project, request, Some(trace.0.clone()))
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/test/overview",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    responses((status = 200, body = TestOverviewResponse))
)]
pub(crate) async fn overview(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
) -> Result<Json<TestOverviewResponse>, ApiError> {
    let response = state
        .app
        .test_overview(&auth, scope.project)
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{pid}/test/cases/{cid}/approve",
    params(
        ("pid" = uuid::Uuid, Path, description = "Project ID"),
        ("cid" = uuid::Uuid, Path, description = "Test case ID")
    ),
    responses(
        (status = 200, body = ApproveTestCaseResponse),
        (status = 422, body = contracts::Problem),
        (status = 404, body = contracts::Problem)
    )
)]
pub(crate) async fn approve_case(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path((_pid, cid)): Path<(String, String)>,
) -> Result<Json<ApproveTestCaseResponse>, ApiError> {
    let case_id = parse_uuid(&cid, &trace)?;
    let response = state
        .app
        .approve_test_case(&auth, scope.project, case_id, Some(trace.0.clone()))
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    if !response.validation_errors.is_empty() {
        return Err(ApiError::new(ErrorCode::SchemaViolation, &trace)
            .detail(response.validation_errors.join(", ")));
    }
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{pid}/test/runs",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    request_body = CreateTestRunRequest,
    responses(
        (status = 202, body = CreateTestRunResponse),
        (status = 400, body = contracts::Problem)
    )
)]
pub(crate) async fn create_run(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Json(request): Json<CreateTestRunRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let response = state
        .app
        .create_test_run(&auth, scope.project, request, Some(trace.0.clone()))
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok((StatusCode::ACCEPTED, Json(response)))
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{pid}/test/runs/{rid}/executions:import",
    params(
        ("pid" = uuid::Uuid, Path, description = "Project ID"),
        ("rid" = uuid::Uuid, Path, description = "Test run ID")
    ),
    request_body = ImportExecutionsRequest,
    responses(
        (status = 200, body = ImportExecutionsResponse),
        (status = 404, body = contracts::Problem)
    )
)]
pub(crate) async fn import_executions(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path((_pid, rid)): Path<(String, String)>,
    Json(request): Json<ImportExecutionsRequest>,
) -> Result<Json<ImportExecutionsResponse>, ApiError> {
    let run_id = parse_uuid(&rid, &trace)?;
    let response = state
        .app
        .import_test_executions(&auth, scope.project, run_id, request, Some(trace.0.clone()))
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{pid}/test/artifacts:expire",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    request_body = ExpireArtifactsRequest,
    responses(
        (status = 200, body = ArtifactCleanupResponse),
        (status = 403, body = contracts::Problem)
    )
)]
pub(crate) async fn expire_artifacts(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Json(request): Json<ExpireArtifactsRequest>,
) -> Result<Json<ArtifactCleanupResponse>, ApiError> {
    state
        .app
        .require_test_run(&auth, scope.project)
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    let response = state
        .app
        .expire_test_artifacts(
            Some(scope.project),
            request.retention_days.unwrap_or(90),
            request.trace_days.unwrap_or(30),
        )
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{pid}/test/runs/{rid}/artifacts:cleanup",
    params(
        ("pid" = uuid::Uuid, Path, description = "Project ID"),
        ("rid" = uuid::Uuid, Path, description = "Test run ID")
    ),
    responses(
        (status = 200, body = ArtifactCleanupResponse),
        (status = 404, body = contracts::Problem)
    )
)]
pub(crate) async fn cleanup_run_artifacts(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path((_pid, rid)): Path<(String, String)>,
) -> Result<Json<ArtifactCleanupResponse>, ApiError> {
    let run_id = parse_uuid(&rid, &trace)?;
    let response = state
        .app
        .cleanup_run_artifacts(&auth, scope.project, run_id, Some(trace.0.clone()))
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{pid}/test/coverage:recompute",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    request_body = RecomputeCoverageRequest,
    responses((status = 200, body = CoverageView))
)]
pub(crate) async fn recompute_coverage(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Json(request): Json<RecomputeCoverageRequest>,
) -> Result<Json<CoverageView>, ApiError> {
    let response = state
        .app
        .recompute_test_coverage(&auth, scope.project, request)
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{pid}/test/defects",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    request_body = CreateDefectRequest,
    responses(
        (status = 200, body = DefectCandidateView),
        (status = 400, body = contracts::Problem)
    )
)]
pub(crate) async fn create_defect(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Json(request): Json<CreateDefectRequest>,
) -> Result<Json<DefectCandidateView>, ApiError> {
    let response = state
        .app
        .create_test_defect(&auth, scope.project, request, Some(trace.0.clone()))
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok(Json(response))
}
