use app::Authenticated;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use chrono::Utc;
use contracts::{
    AuditListResponse, BulkImportRequest, BulkImportResponse, DocumentChunkListResponse,
    DocumentChunkQuery, DocumentDetail, DocumentListQuery, DocumentListResponse, ErrorCode,
    RegisterDocumentRequest, TreeQuery, TreeResponse, UploadUrlRequest, UploadUrlResponse,
};
use domain::{DocumentId, Permission, RevisionId};

use crate::auth::ProjectScope;
use crate::error::ApiError;
use crate::trace::TraceId;
use crate::ApiState;

pub fn project_router() -> Router<ApiState> {
    Router::new()
        .route(
            "/api/v1/projects/{pid}/documents:upload-url",
            post(create_upload_url),
        )
        .route("/api/v1/projects/{pid}/documents", post(register).get(list))
        .route(
            "/api/v1/projects/{pid}/documents:bulk-import",
            post(bulk_import),
        )
        .route("/api/v1/projects/{pid}/audit-logs", get(audit_logs))
}

pub fn resource_router() -> Router<ApiState> {
    Router::new()
        .route(
            "/api/v1/documents/{did}",
            get(detail).delete(delete_document),
        )
        .route("/api/v1/documents/{did}/content", get(content))
        .route("/api/v1/documents/{did}/tree", get(tree))
        .route("/api/v1/documents/{did}/chunks", get(chunks))
        .route("/api/v1/documents/{did}/revisions/{rid_parse}", post(parse))
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{pid}/documents:upload-url",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    request_body = UploadUrlRequest,
    responses(
        (status = 201, body = UploadUrlResponse),
        (status = 400, body = contracts::Problem),
        (status = 403, body = contracts::Problem)
    )
)]
pub(crate) async fn create_upload_url(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Json(req): Json<UploadUrlRequest>,
) -> Result<impl IntoResponse, ApiError> {
    require_write(scope.role, &trace)?;
    if req.filename.trim().is_empty() {
        return Err(
            ApiError::new(ErrorCode::ValidationSchema, &trace).detail("filename は必須です")
        );
    }
    if req.size_bytes < 1 || req.size_bytes as u64 > app::MAX_UPLOAD_BYTES {
        return Err(ApiError::new(ErrorCode::ValidationSchema, &trace)
            .detail("size_bytes が上限を超えています"));
    }
    if !app::document::is_supported_mime(&req.mime_type) {
        return Err(ApiError::new(ErrorCode::ValidationSchema, &trace)
            .detail("PDF / XLSX / DOCX / PPTX のいずれかを指定してください"));
    }

    let object_key = crate::upload::new_object_key(scope.project, &req.filename);
    let (token, expires_at) = state.upload.issue(
        scope.project,
        &object_key,
        app::MAX_UPLOAD_BYTES as i64,
        Utc::now(),
    );

    Ok((
        StatusCode::CREATED,
        Json(UploadUrlResponse {
            upload_url: state.upload.upload_url(&token),
            object_key,
            expires_at,
            max_size_bytes: app::MAX_UPLOAD_BYTES as i64,
        }),
    ))
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{pid}/documents:bulk-import",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    request_body = BulkImportRequest,
    responses(
        (status = 202, body = BulkImportResponse),
        (status = 400, body = contracts::Problem),
        (status = 403, body = contracts::Problem)
    )
)]
pub(crate) async fn bulk_import(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Json(req): Json<BulkImportRequest>,
) -> Result<impl IntoResponse, ApiError> {
    require_write(scope.role, &trace)?;
    let response = state
        .app
        .enqueue_bulk_import(&auth, scope.project, req, Some(trace.0.clone()))
        .await
        .map_err(|e| ApiError::from_app(e, &trace))?;
    Ok((StatusCode::ACCEPTED, Json(response)))
}

#[utoipa::path(
    post,
    path = "/api/v1/projects/{pid}/documents",
    params(("pid" = uuid::Uuid, Path, description = "Project ID")),
    request_body = RegisterDocumentRequest,
    responses(
        (status = 201, body = contracts::RegisterDocumentResponse),
        (status = 400, body = contracts::Problem),
        (status = 409, body = contracts::Problem)
    )
)]
pub(crate) async fn register(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Json(req): Json<RegisterDocumentRequest>,
) -> Result<impl IntoResponse, ApiError> {
    require_write(scope.role, &trace)?;
    let response = state
        .app
        .register_document(&auth, scope.project, req, Some(trace.0.clone()))
        .await
        .map_err(|e| ApiError::from_app(e, &trace))?;
    Ok((StatusCode::CREATED, Json(response)))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/documents",
    params(
        ("pid" = uuid::Uuid, Path, description = "Project ID"),
        ("doc_type" = Option<String>, Query, description = "Document type"),
        ("business_domain" = Option<String>, Query, description = "Business domain"),
        ("status" = Option<String>, Query, description = "Parse status"),
        ("q" = Option<String>, Query, description = "Title filter"),
        ("limit" = Option<i64>, Query, description = "Page size, max 100"),
        ("cursor" = Option<String>, Query, description = "Opaque cursor")
    ),
    responses((status = 200, body = DocumentListResponse))
)]
pub(crate) async fn list(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Query(query): Query<DocumentListQuery>,
) -> Result<Json<DocumentListResponse>, ApiError> {
    let response = state
        .app
        .list_documents(&auth, scope.project, query)
        .await
        .map_err(|e| ApiError::from_app(e, &trace))?;
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/api/v1/documents/{did}",
    params(("did" = uuid::Uuid, Path, description = "Document ID")),
    responses((status = 200, body = DocumentDetail), (status = 404, body = contracts::Problem))
)]
pub(crate) async fn detail(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path(did): Path<String>,
) -> Result<Json<DocumentDetail>, ApiError> {
    let document = parse_document(&did, &trace)?;
    let response = state
        .app
        .document_detail(&auth, document)
        .await
        .map_err(|e| ApiError::from_app(e, &trace))?;
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/api/v1/documents/{did}/content",
    params(("did" = uuid::Uuid, Path, description = "Document ID")),
    responses((status = 200, description = "PDF bytes"), (status = 404, body = contracts::Problem))
)]
pub(crate) async fn content(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path(did): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let document = parse_document(&did, &trace)?;
    let (bytes, mime_type) = state
        .app
        .document_content(&auth, document)
        .await
        .map_err(|e| ApiError::from_app(e, &trace))?;
    Ok((
        StatusCode::OK,
        [
            (axum::http::header::CONTENT_TYPE, mime_type),
            (
                axum::http::header::CACHE_CONTROL,
                "private, max-age=60".to_string(),
            ),
        ],
        bytes,
    ))
}

#[utoipa::path(
    delete,
    path = "/api/v1/documents/{did}",
    params(("did" = uuid::Uuid, Path, description = "Document ID")),
    responses((status = 204), (status = 404, body = contracts::Problem))
)]
pub(crate) async fn delete_document(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path(did): Path<String>,
) -> Result<StatusCode, ApiError> {
    let document = parse_document(&did, &trace)?;
    state
        .app
        .delete_document(&auth, document, Some(trace.0.clone()))
        .await
        .map_err(|e| ApiError::from_app(e, &trace))?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/api/v1/documents/{did}/tree",
    params(
        ("did" = uuid::Uuid, Path, description = "Document ID"),
        ("revision_id" = Option<uuid::Uuid>, Query, description = "Revision ID"),
        ("parent" = Option<String>, Query, description = "Parent node ID or ROOT"),
        ("limit" = Option<i64>, Query, description = "Page size"),
        ("cursor" = Option<String>, Query, description = "Opaque cursor")
    ),
    responses((status = 200, body = TreeResponse), (status = 404, body = contracts::Problem))
)]
pub(crate) async fn tree(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path(did): Path<String>,
    Query(query): Query<TreeQuery>,
) -> Result<Json<TreeResponse>, ApiError> {
    let document = parse_document(&did, &trace)?;
    let response = state
        .app
        .document_tree(&auth, document, query)
        .await
        .map_err(|e| ApiError::from_app(e, &trace))?;
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/api/v1/documents/{did}/chunks",
    params(
        ("did" = uuid::Uuid, Path, description = "Document ID"),
        ("revision_id" = Option<uuid::Uuid>, Query, description = "Revision ID"),
        ("limit" = Option<i64>, Query, description = "Page size"),
        ("cursor" = Option<String>, Query, description = "Opaque cursor")
    ),
    responses((status = 200, body = DocumentChunkListResponse), (status = 404, body = contracts::Problem))
)]
pub(crate) async fn chunks(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path(did): Path<String>,
    Query(query): Query<DocumentChunkQuery>,
) -> Result<Json<DocumentChunkListResponse>, ApiError> {
    let document = parse_document(&did, &trace)?;
    let response = state
        .app
        .document_chunks(&auth, document, query)
        .await
        .map_err(|e| ApiError::from_app(e, &trace))?;
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/api/v1/documents/{did}/revisions/{rid}:parse",
    params(
        ("did" = uuid::Uuid, Path, description = "Document ID"),
        ("rid" = uuid::Uuid, Path, description = "Revision ID"),
        ("Idempotency-Key" = Option<String>, Header, description = "Idempotency key")
    ),
    responses((status = 202, body = contracts::ParseAcceptedResponse), (status = 404, body = contracts::Problem))
)]
pub(crate) async fn parse(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Path((did, rid_parse)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let document = parse_document(&did, &trace)?;
    let rid = rid_parse
        .strip_suffix(":parse")
        .ok_or_else(|| ApiError::new(ErrorCode::ValidationSchema, &trace))?;
    let revision: RevisionId = rid
        .parse()
        .map_err(|_| ApiError::new(ErrorCode::NotFoundDocument, &trace))?;
    let idempotency_key = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let response = state
        .app
        .enqueue_parse(
            &auth,
            document,
            revision,
            idempotency_key,
            Some(trace.0.clone()),
        )
        .await
        .map_err(|e| ApiError::from_app(e, &trace))?;
    Ok((StatusCode::ACCEPTED, Json(response)))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{pid}/audit-logs",
    params(
        ("pid" = uuid::Uuid, Path, description = "Project ID"),
        ("limit" = Option<i64>, Query, description = "Page size, max 200")
    ),
    responses((status = 200, body = AuditListResponse), (status = 403, body = contracts::Problem))
)]
pub(crate) async fn audit_logs(
    State(state): State<ApiState>,
    scope: ProjectScope,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<Authenticated>,
    Query(query): Query<AuditQuery>,
) -> Result<Json<AuditListResponse>, ApiError> {
    let response = state
        .app
        .audit_logs(&auth, scope.project, query.limit.unwrap_or(50))
        .await
        .map_err(|e| ApiError::from_app(e, &trace))?;
    Ok(Json(response))
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct AuditQuery {
    limit: Option<i64>,
}

fn require_write(role: domain::Role, trace: &TraceId) -> Result<(), ApiError> {
    if Permission::DocumentIngest.writable_by(role) {
        Ok(())
    } else {
        Err(ApiError::new(ErrorCode::ForbiddenProjectScope, trace)
            .detail("document:write がありません"))
    }
}

fn parse_document(raw: &str, trace: &TraceId) -> Result<DocumentId, ApiError> {
    raw.parse()
        .map_err(|_| ApiError::new(ErrorCode::NotFoundDocument, trace))
}
