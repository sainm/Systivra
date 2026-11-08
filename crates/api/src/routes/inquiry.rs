//! Q-01 変更影響問合せの HTTP 境界。
//!
//! `/events` は `text/event-stream`。段ごとに SSE frame を送出し、
//! `Last-Event-ID` で再開できる。

use app::Authenticated;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use contracts::{
    CreateInquiryRequest, ErrorCode, InquiryAcceptedResponse, InquiryListResponse, InquiryResponse,
};
use std::convert::Infallible;
use std::time::Duration;
use uuid::Uuid;

use crate::auth::ProjectScope;
use crate::error::ApiError;
use crate::trace::TraceId;
use crate::ApiState;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route(
            "/api/v1/projects/{pid}/change-inquiries",
            get(list_inquiries).post(create_inquiry),
        )
        .route(
            "/api/v1/projects/{pid}/change-inquiries/{iid}",
            get(inquiry),
        )
        .route(
            "/api/v1/projects/{pid}/change-inquiries/{iid}/events",
            get(inquiry_events),
        )
        .route(
            "/api/v1/projects/{pid}/change-inquiries/{iid}/report.md",
            get(inquiry_markdown),
        )
        .route(
            "/api/v1/projects/{pid}/change-inquiries/{iid}/report.pdf",
            get(inquiry_pdf),
        )
        .route(
            "/api/v1/projects/{pid}/change-inquiries/{iid}/narrate",
            axum::routing::post(narrate_inquiry),
        )
        .route(
            "/api/v1/projects/{pid}/change-inquiries/{iid}/test-plans:derive",
            axum::routing::post(derive_inquiry_plan),
        )
}

fn parse_uuid(raw: &str, trace: &TraceId) -> Result<Uuid, ApiError> {
    raw.parse()
        .map_err(|_| ApiError::new(ErrorCode::ValidationSchema, trace))
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{pid}/change-inquiries",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    request_body = CreateInquiryRequest,
    responses(
        (status = 202, body = InquiryAcceptedResponse),
        (status = 400, body = contracts::Problem)
    )
)]
pub(crate) async fn create_inquiry(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Json(request): Json<CreateInquiryRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let response = state
        .app
        .create_inquiry(&auth, scope.project, request, Some(trace.0.clone()))
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok((StatusCode::ACCEPTED, Json(response)))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/change-inquiries",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    responses((status = 200, body = InquiryListResponse))
)]
pub(crate) async fn list_inquiries(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
) -> Result<Json<InquiryListResponse>, ApiError> {
    state
        .app
        .list_inquiries(&auth, scope.project)
        .await
        .map(Json)
        .map_err(|error| ApiError::from_app(error, &trace))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/change-inquiries/{iid}",
    params(
        ("pid" = uuid::Uuid, Path, description = "Project ID"),
        ("iid" = uuid::Uuid, Path, description = "Inquiry ID")
    ),
    responses((status = 200, body = InquiryResponse), (status = 404, body = contracts::Problem))
)]
pub(crate) async fn inquiry(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path((_pid, iid)): Path<(String, String)>,
) -> Result<Json<InquiryResponse>, ApiError> {
    let inquiry_id = parse_uuid(&iid, &trace)?;
    state
        .app
        .inquiry_view(&auth, scope.project, inquiry_id)
        .await
        .map(Json)
        .map_err(|error| ApiError::from_app(error, &trace))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/change-inquiries/{iid}/events",
    params(
        ("pid" = uuid::Uuid, Path, description = "Project ID"),
        ("iid" = uuid::Uuid, Path, description = "Inquiry ID"),
        ("Last-Event-ID" = Option<String>, Header, description = "Resume after this SSE event ID")
    ),
    responses((status = 200, description = "text/event-stream"))
)]
pub(crate) async fn inquiry_events(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path((_pid, iid)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let inquiry_id = parse_uuid(&iid, &trace)?;
    let events = state
        .app
        .inquiry_events(&auth, scope.project, inquiry_id)
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    let after = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    let stream = async_stream::stream! {
        for (index, inquiry_event) in events
            .into_iter()
            .enumerate()
            .skip(after as usize)
        {
            let id = index as u64 + 1;
            let data = serde_json::to_string(&inquiry_event.data).unwrap_or_else(|_| "null".into());
            yield Ok::<Event, Infallible>(
                Event::default()
                    .id(id.to_string())
                    .event(inquiry_event.event)
                    .data(data),
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response())
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{pid}/change-inquiries/{iid}/test-plans:derive",
    params(
        ("pid" = uuid::Uuid, Path, description = "Project ID"),
        ("iid" = uuid::Uuid, Path, description = "Inquiry ID")
    ),
    request_body = contracts::DeriveInquiryPlanRequest,
    responses(
        (status = 200, body = contracts::DeriveTestPlanResponse),
        (status = 400, body = contracts::Problem)
    )
)]
pub(crate) async fn derive_inquiry_plan(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path((_pid, iid)): Path<(String, String)>,
    Json(request): Json<contracts::DeriveInquiryPlanRequest>,
) -> Result<Json<contracts::DeriveTestPlanResponse>, ApiError> {
    let inquiry_id = parse_uuid(&iid, &trace)?;
    let response = state
        .app
        .derive_plan_from_inquiry(
            &auth,
            scope.project,
            inquiry_id,
            request,
            Some(trace.0.clone()),
        )
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/change-inquiries/{iid}/report.md",
    params(
        ("pid" = uuid::Uuid, Path, description = "Project ID"),
        ("iid" = uuid::Uuid, Path, description = "Inquiry ID")
    ),
    responses((status = 200, description = "Inquiry report as Markdown"))
)]
pub(crate) async fn inquiry_markdown(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path((_pid, iid)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let inquiry_id = parse_uuid(&iid, &trace)?;
    let markdown = state
        .app
        .inquiry_markdown(&auth, scope.project, inquiry_id)
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok((
        [
            (
                header::CONTENT_TYPE,
                "text/markdown; charset=utf-8".to_owned(),
            ),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"inquiry-{inquiry_id}.md\""),
            ),
        ],
        markdown,
    )
        .into_response())
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/change-inquiries/{iid}/report.pdf",
    params(
        ("pid" = uuid::Uuid, Path, description = "Project ID"),
        ("iid" = uuid::Uuid, Path, description = "Inquiry ID")
    ),
    responses(
        (status = 200, description = "Inquiry report as PDF"),
        (status = 409, body = contracts::Problem)
    )
)]
pub(crate) async fn inquiry_pdf(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path((_pid, iid)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let inquiry_id = parse_uuid(&iid, &trace)?;
    let pdf = state
        .app
        .inquiry_pdf(&auth, scope.project, inquiry_id)
        .await
        .map_err(|error| ApiError::from_app(error, &trace))?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/pdf".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"inquiry-{inquiry_id}.pdf\""),
            ),
        ],
        pdf,
    )
        .into_response())
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{pid}/change-inquiries/{iid}/narrate",
    params(
        ("pid" = uuid::Uuid, Path, description = "Project ID"),
        ("iid" = uuid::Uuid, Path, description = "Inquiry ID")
    ),
    responses((status = 200, body = contracts::NarrateInquiryResponse))
)]
pub(crate) async fn narrate_inquiry(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path((_pid, iid)): Path<(String, String)>,
) -> Result<Json<contracts::NarrateInquiryResponse>, ApiError> {
    let inquiry_id = parse_uuid(&iid, &trace)?;
    state
        .app
        .narrate_inquiry(&auth, scope.project, inquiry_id)
        .await
        .map(Json)
        .map_err(|error| ApiError::from_app(error, &trace))
}
