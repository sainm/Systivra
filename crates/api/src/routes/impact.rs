use app::Authenticated;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use contracts::{
    CreateImpactAssessmentRequest, ErrorCode, ImpactAssessmentAcceptedResponse,
    ImpactAssessmentListQuery, ImpactAssessmentListResponse, ImpactAssessmentResponse,
};
use uuid::Uuid;

use crate::auth::ProjectScope;
use crate::error::ApiError;
use crate::trace::TraceId;
use crate::ApiState;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route(
            "/api/v1/projects/{pid}/impacts",
            get(list_impacts).post(create_impact),
        )
        .route("/api/v1/impacts/{iid}", get(impact))
        .route("/api/v1/impacts/{iid}/report.md", get(impact_markdown))
        .route("/api/v1/impacts/{iid}/narrate", post(narrate_impact))
        .route("/api/v1/impacts/{iid}/report.pdf", get(impact_pdf))
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{pid}/impacts",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    request_body = CreateImpactAssessmentRequest,
    responses(
        (status = 202, body = ImpactAssessmentAcceptedResponse),
        (status = 400, body = contracts::Problem),
        (status = 403, body = contracts::Problem)
    )
)]
pub(crate) async fn create_impact(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Json(request): Json<CreateImpactAssessmentRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let response = state
        .app
        .enqueue_impact_assessment(&auth, scope.project, request, Some(trace.0.clone()))
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok((StatusCode::ACCEPTED, Json(response)))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/impacts",
    params(
        ("pid" = uuid::Uuid, Path, description = "Project ID"),
        ("limit" = Option<i64>, Query, description = "Page size")
    ),
    responses((status = 200, body = ImpactAssessmentListResponse))
)]
pub(crate) async fn list_impacts(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Query(query): Query<ImpactAssessmentListQuery>,
) -> Result<Json<ImpactAssessmentListResponse>, ApiError> {
    let response = state
        .app
        .impact_assessments(&auth, scope.project, query.limit.unwrap_or(20))
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/api/v1/impacts/{iid}",
    params(("iid" = uuid::Uuid, Path, description = "Impact assessment ID")),
    responses(
        (status = 200, body = ImpactAssessmentResponse),
        (status = 404, body = contracts::Problem)
    )
)]
pub(crate) async fn impact(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path(iid): Path<String>,
) -> Result<Json<ImpactAssessmentResponse>, ApiError> {
    let impact_id = parse_uuid(&iid, &trace)?;
    let project = state
        .app
        .project_for(auth.user_id, "impact_assessment", impact_id)
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    let response = state
        .app
        .impact_assessment(&auth, project, impact_id)
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok(Json(response))
}

fn parse_uuid(raw: &str, trace: &TraceId) -> Result<Uuid, ApiError> {
    raw.parse()
        .map_err(|_| ApiError::new(ErrorCode::ValidationSchema, trace))
}

#[utoipa::path(
    get,
    path = "/api/v1/impacts/{iid}/report.md",
    params(("iid" = uuid::Uuid, Path, description = "Impact assessment ID")),
    responses((status = 200, description = "Impact report as Markdown"))
)]
pub(crate) async fn impact_markdown(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path(iid): Path<String>,
) -> Result<axum::response::Response, ApiError> {
    use axum::response::IntoResponse;
    let impact_id = parse_uuid(&iid, &trace)?;
    let project = state
        .app
        .project_for(auth.user_id, "impact_assessment", impact_id)
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    let markdown = state
        .app
        .impact_markdown(&auth, project, impact_id)
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok((
        [
            (
                axum::http::header::CONTENT_TYPE,
                "text/markdown; charset=utf-8".to_owned(),
            ),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"impact-{impact_id}.md\""),
            ),
        ],
        markdown,
    )
        .into_response())
}

#[utoipa::path(
    get,
    path = "/api/v1/impacts/{iid}/report.pdf",
    params(("iid" = uuid::Uuid, Path, description = "Impact assessment ID")),
    responses(
        (status = 200, description = "Impact report as PDF"),
        (status = 409, body = contracts::Problem)
    )
)]
pub(crate) async fn impact_pdf(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path(iid): Path<String>,
) -> Result<axum::response::Response, ApiError> {
    let impact_id = parse_uuid(&iid, &trace)?;
    let project = state
        .app
        .project_for(auth.user_id, "impact_assessment", impact_id)
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    let pdf = state
        .app
        .impact_pdf(&auth, project, impact_id)
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok((
        [
            (
                axum::http::header::CONTENT_TYPE,
                "application/pdf".to_owned(),
            ),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"impact-{impact_id}.pdf\""),
            ),
        ],
        pdf,
    )
        .into_response())
}

#[utoipa::path(
    post,
    path = "/api/v1/impacts/{iid}/narrate",
    params(("iid" = uuid::Uuid, Path, description = "Impact assessment ID")),
    responses((status = 200, body = contracts::NarrateImpactResponse))
)]
pub(crate) async fn narrate_impact(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path(iid): Path<String>,
) -> Result<Json<contracts::NarrateImpactResponse>, ApiError> {
    let impact_id = parse_uuid(&iid, &trace)?;
    let project = state
        .app
        .project_for(auth.user_id, "impact_assessment", impact_id)
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    state
        .app
        .narrate_impact(&auth, project, impact_id)
        .await
        .map(Json)
        .map_err(|error| ApiError::from_app(error, &trace))
}
