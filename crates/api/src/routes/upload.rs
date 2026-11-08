use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::StatusCode;
use axum::routing::put;
use axum::{Extension, Router};

use crate::error::ApiError;
use crate::trace::TraceId;
use crate::ApiState;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/api/v1/uploads/{token}", put(receive))
        .layer(DefaultBodyLimit::max(app::MAX_UPLOAD_BYTES as usize))
}

#[utoipa::path(
    put,
    path = "/api/v1/uploads/{token}",
    params(("token" = String, Path, description = "Signed upload token")),
    request_body(content = Vec<u8>, content_type = "application/octet-stream"),
    responses((status = 204), (status = 400, body = contracts::Problem), (status = 401, body = contracts::Problem))
)]
pub(crate) async fn receive(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Path(token): Path<String>,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    let claims = state
        .upload
        .verify(&token, chrono::Utc::now())
        .ok_or_else(|| ApiError::new(contracts::ErrorCode::AuthTokenExpired, &trace))?;
    if body.is_empty() {
        return Err(
            ApiError::new(contracts::ErrorCode::ValidationSchema, &trace)
                .detail("空のファイルはアップロードできません"),
        );
    }
    if body.len() as i64 > claims.max_size {
        return Err(
            ApiError::new(contracts::ErrorCode::ValidationSchema, &trace)
                .detail("ファイルサイズが上限を超えています"),
        );
    }
    if !body.starts_with(b"%PDF-")
        && !body.starts_with(b"PK\x03\x04")
        && !body.starts_with(b"PK\x05\x06")
    {
        return Err(
            ApiError::new(contracts::ErrorCode::ValidationSchema, &trace)
                .detail("PDF / XLSX ではありません"),
        );
    }
    state
        .app
        .put_object(&claims.object_key, &body)
        .await
        .map_err(|e| ApiError::from_app(e, &trace))?;
    Ok(StatusCode::NO_CONTENT)
}
