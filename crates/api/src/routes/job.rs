use std::convert::Infallible;
use std::time::Duration;

use crate::error::ApiError;
use crate::trace::TraceId;
use crate::ApiState;
use app::Authenticated;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use axum::{Extension, Json, Router};
use contracts::{ErrorCode, JobResponse};
use domain::JobId;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/api/v1/jobs/{jid}", get(job))
        .route("/api/v1/jobs/{jid}/events", get(events))
}

#[utoipa::path(
    get,
    path = "/api/v1/jobs/{jid}",
    params(("jid" = uuid::Uuid, Path, description = "Job ID")),
    responses((status = 200, body = JobResponse), (status = 404, body = contracts::Problem))
)]
pub(crate) async fn job(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path(jid): Path<String>,
) -> Result<Json<JobResponse>, ApiError> {
    let job_id = parse_job(&jid, &trace)?;
    let response = state
        .app
        .job_for_user(&auth, job_id)
        .await
        .map_err(|e| ApiError::from_app(e, &trace))?;
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/api/v1/jobs/{jid}/events",
    params(
        ("jid" = uuid::Uuid, Path, description = "Job ID"),
        ("Last-Event-ID" = Option<String>, Header, description = "Replay events after this id")
    ),
    responses((status = 200, description = "Server-sent events"), (status = 404, body = contracts::Problem))
)]
pub(crate) async fn events(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path(jid): Path<String>,
    headers: HeaderMap,
) -> Result<Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let job_id = parse_job(&jid, &trace)?;
    state
        .app
        .job_for_user(&auth, job_id)
        .await
        .map_err(|e| ApiError::from_app(e, &trace))?;

    let last = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);
    let app_state = state.clone();
    let auth = auth.clone();
    let stream = async_stream::stream! {
        let mut cursor = last;
        let mut ticker = tokio::time::interval(Duration::from_millis(500));
        loop {
            ticker.tick().await;
            let job = match app_state.app.job_for_user(&auth, job_id).await {
                Ok(job) => job,
                Err(_) => break,
            };
            let events = match app_state
                .app
                .job_events_since(&auth, job_id, cursor, 100)
                .await
            {
                Ok(events) => events,
                Err(_) => break,
            };
            let count = events.len();
            for event in events {
                cursor = event.id;
                let data = serde_json::to_string(&event.data).unwrap_or_else(|_| "{}".into());
                yield Ok(Event::default().id(event.id.to_string()).event(event.event).data(data));
            }
            if job.status.is_terminal() && count == 0 {
                break;
            }
        }
    };

    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(20))
            .text("ping"),
    ))
}

fn parse_job(raw: &str, trace: &TraceId) -> Result<JobId, ApiError> {
    raw.parse()
        .map_err(|_| ApiError::new(ErrorCode::NotFoundJob, trace))
}
