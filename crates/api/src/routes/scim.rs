//! SCIM 2.0 の最小実装（Users / Groups）。
//!
//! Bearer は `SCIM_BEARER_TOKEN`。未設定なら 404（機能を隠す）。
//! IdP からのユーザー作成・無効化、グループとロールの同期に使う。

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use serde_json::{json, Value};
use uuid::Uuid;

use contracts::ErrorCode;
use domain::{Role, UserId};

use crate::error::ApiError;
use crate::trace::TraceId;
use crate::ApiState;

const ROLE_EXTENSION: &str = "urn:systivra:params:scim:schemas:extension:role";

pub fn router() -> Router<ApiState> {
    Router::new()
        .route(
            "/scim/v2/ServiceProviderConfig",
            get(service_provider_config),
        )
        .route("/scim/v2/Users", get(list_users).post(create_user))
        .route(
            "/scim/v2/Users/{id}",
            get(get_user).patch(patch_user).delete(delete_user),
        )
        .route("/scim/v2/Groups", get(list_groups).post(create_group))
        .route(
            "/scim/v2/Groups/{id}",
            get(get_group).put(put_group).delete(delete_group),
        )
        .route_layer(axum::middleware::from_fn(require_scim))
}

async fn require_scim(
    headers: HeaderMap,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let expected = std::env::var("SCIM_BEARER_TOKEN").unwrap_or_default();
    let trace = req
        .extensions()
        .get::<TraceId>()
        .cloned()
        .unwrap_or_else(|| TraceId(String::new()));
    if expected.is_empty() {
        return ApiError::new(ErrorCode::NotFound, &trace).into_response();
    }
    let presented = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .unwrap_or_default();
    if !constant_time_eq(presented.as_bytes(), expected.as_bytes()) {
        return ApiError::new(ErrorCode::AuthTokenExpired, &trace)
            .detail("SCIM bearer token is invalid")
            .into_response();
    }
    next.run(req).await
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

fn scim_user(user: &app::ScimUserRow) -> Value {
    let emails = user
        .email
        .as_ref()
        .map(|email| vec![json!({"value": email, "primary": true})])
        .unwrap_or_default();
    json!({
        "schemas": ["urn:ietf:params:scim:schemas:core:2.0:User"],
        "id": user.id.to_string(),
        "userName": user.login_id,
        "displayName": user.display_name,
        "name": {"formatted": user.display_name},
        "emails": emails,
        "active": user.is_active,
        "meta": {"resourceType": "User", "location": format!("/scim/v2/Users/{}", user.id)}
    })
}

fn scim_group(group: &app::ScimGroupRow) -> Value {
    let members: Vec<Value> = group
        .members
        .iter()
        .map(|member| json!({"value": member.id.to_string(), "display": member.login_id}))
        .collect();
    json!({
        "schemas": [
            "urn:ietf:params:scim:schemas:core:2.0:Group",
            ROLE_EXTENSION
        ],
        "id": group.id.to_string(),
        "externalId": group.external_id,
        "displayName": group.display_name,
        "members": members,
        ROLE_EXTENSION: {"role": group.role.as_str()},
        "meta": {"resourceType": "Group", "location": format!("/scim/v2/Groups/{}", group.id)}
    })
}

fn scim_error(trace: &TraceId, status: u16, detail: &str) -> Response {
    (
        StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST),
        Json(json!({
            "schemas": ["urn:ietf:params:scim:api:messages:2.0:Error"],
            "status": status.to_string(),
            "detail": detail,
            "trace_id": trace.as_str()
        })),
    )
        .into_response()
}

fn user_id(raw: &str) -> Option<UserId> {
    raw.parse().ok().map(UserId)
}

fn group_id(raw: &str) -> Option<Uuid> {
    raw.parse().ok()
}

fn filter_value(filter: Option<&String>, key: &str) -> Option<String> {
    let filter = filter?;
    let (left, right) = filter.split_once(" eq ")?;
    if left.trim() != key {
        return None;
    }
    Some(right.trim().trim_matches('"').to_owned())
}

pub(crate) async fn service_provider_config() -> Json<Value> {
    Json(json!({
        "schemas": ["urn:ietf:params:scim:schemas:core:2.0:ServiceProviderConfig"],
        "patch": {"supported": true},
        "bulk": {"supported": false, "maxOperations": 0, "maxPayloadSize": 0},
        "filter": {"supported": true, "maxResults": 200},
        "changePassword": {"supported": false},
        "sort": {"supported": false},
        "etag": {"supported": false},
        "authenticationSchemes": [{"type": "oauthbearertoken", "name": "Bearer"}]
    }))
}

pub(crate) async fn list_users(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Value>, ApiError> {
    let mut users = state.app.list_scim_users().await.map_err(|e| {
        tracing::error!(error = %e, "scim list users failed");
        ApiError::new(ErrorCode::Internal, &trace)
    })?;
    if let Some(name) = filter_value(query.get("filter"), "userName") {
        users.retain(|user| user.login_id == name);
    }
    let resources: Vec<Value> = users.iter().map(scim_user).collect();
    Ok(Json(json!({
        "schemas": ["urn:ietf:params:scim:api:messages:2.0:ListResponse"],
        "totalResults": resources.len(),
        "Resources": resources
    })))
}

pub(crate) async fn create_user(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Json(body): Json<Value>,
) -> Response {
    let Some(login_id) = body.get("userName").and_then(Value::as_str) else {
        return scim_error(&trace, 400, "userName is required");
    };
    let login_id = login_id.trim();
    if login_id.is_empty() {
        return scim_error(&trace, 400, "userName is required");
    }
    let display_name = body
        .get("displayName")
        .and_then(Value::as_str)
        .or_else(|| {
            body.get("name")
                .and_then(|name| name.get("formatted"))
                .and_then(Value::as_str)
        })
        .unwrap_or(login_id);
    let email = body
        .get("emails")
        .and_then(Value::as_array)
        .and_then(|emails| emails.first())
        .and_then(|email| email.get("value"))
        .and_then(Value::as_str);
    let active = body.get("active").and_then(Value::as_bool).unwrap_or(true);

    match state.app.list_scim_users().await {
        Ok(users) if users.iter().any(|user| user.login_id == login_id) => {
            return scim_error(&trace, 409, "userName already exists");
        }
        Ok(_) => {}
        Err(e) => {
            tracing::error!(error = %e, "scim user lookup failed");
            return scim_error(&trace, 500, "user lookup failed");
        }
    }

    match state
        .app
        .create_scim_user(login_id, display_name, email, active)
        .await
    {
        Ok(user) => (StatusCode::CREATED, Json(scim_user(&user))).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "scim create user failed");
            scim_error(&trace, 500, "user creation failed")
        }
    }
}

pub(crate) async fn get_user(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Path(id): Path<String>,
) -> Response {
    let Some(id) = user_id(&id) else {
        return scim_error(&trace, 404, "user not found");
    };
    match state.app.find_scim_user(id).await {
        Ok(Some(user)) => Json(scim_user(&user)).into_response(),
        Ok(None) => scim_error(&trace, 404, "user not found"),
        Err(e) => {
            tracing::error!(error = %e, "scim get user failed");
            scim_error(&trace, 500, "user lookup failed")
        }
    }
}

pub(crate) async fn patch_user(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let Some(id) = user_id(&id) else {
        return scim_error(&trace, 404, "user not found");
    };
    let Some(active) = body
        .get("Operations")
        .and_then(Value::as_array)
        .and_then(|operations| {
            operations.iter().find_map(|operation| {
                let path = operation.get("path").and_then(Value::as_str).unwrap_or("");
                let op = operation.get("op").and_then(Value::as_str).unwrap_or("");
                if op.eq_ignore_ascii_case("replace") && path.eq_ignore_ascii_case("active") {
                    operation.get("value").and_then(Value::as_bool)
                } else {
                    None
                }
            })
        })
    else {
        return scim_error(&trace, 400, "unsupported patch operation");
    };
    match state.app.set_user_active(id, active).await {
        Ok(true) => match state.app.find_scim_user(id).await {
            Ok(Some(user)) => Json(scim_user(&user)).into_response(),
            _ => scim_error(&trace, 404, "user not found"),
        },
        Ok(false) => scim_error(&trace, 404, "user not found"),
        Err(e) => {
            tracing::error!(error = %e, "scim patch user failed");
            scim_error(&trace, 500, "user update failed")
        }
    }
}

pub(crate) async fn delete_user(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Path(id): Path<String>,
) -> Response {
    let Some(id) = user_id(&id) else {
        return scim_error(&trace, 404, "user not found");
    };
    match state.app.set_user_active(id, false).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => scim_error(&trace, 404, "user not found"),
        Err(e) => {
            tracing::error!(error = %e, "scim delete user failed");
            scim_error(&trace, 500, "user update failed")
        }
    }
}

pub(crate) async fn list_groups(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Value>, ApiError> {
    let mut groups = state.app.list_scim_groups().await.map_err(|e| {
        tracing::error!(error = %e, "scim list groups failed");
        ApiError::new(ErrorCode::Internal, &trace)
    })?;
    if let Some(name) = filter_value(query.get("filter"), "displayName") {
        groups.retain(|group| group.display_name == name);
    }
    let resources: Vec<Value> = groups.iter().map(scim_group).collect();
    Ok(Json(json!({
        "schemas": ["urn:ietf:params:scim:api:messages:2.0.ListResponse"],
        "totalResults": resources.len(),
        "Resources": resources
    })))
}

#[allow(clippy::result_large_err)]
fn group_payload(
    body: &Value,
    trace: &TraceId,
) -> Result<(String, String, Role, Vec<UserId>), Response> {
    let display_name = body
        .get("displayName")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| scim_error(trace, 400, "displayName is required"))?;
    let external_id = body
        .get("externalId")
        .and_then(Value::as_str)
        .unwrap_or(display_name)
        .to_owned();
    let role_value = body
        .get(ROLE_EXTENSION)
        .and_then(|extension| extension.get("role"))
        .and_then(Value::as_str)
        .unwrap_or("VIEWER");
    let role = Role::parse(role_value).ok_or_else(|| scim_error(trace, 400, "unknown role"))?;
    let mut members = Vec::new();
    if let Some(values) = body.get("members").and_then(Value::as_array) {
        for value in values {
            let Some(raw) = value.get("value").and_then(Value::as_str) else {
                return Err(scim_error(trace, 400, "member value is required"));
            };
            let Some(id) = user_id(raw) else {
                return Err(scim_error(trace, 400, "member value is not a user id"));
            };
            members.push(id);
        }
    }
    Ok((external_id, display_name.to_owned(), role, members))
}

pub(crate) async fn create_group(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Json(body): Json<Value>,
) -> Response {
    let (external_id, display_name, role, members) = match group_payload(&body, &trace) {
        Ok(payload) => payload,
        Err(response) => return response,
    };
    match state
        .app
        .upsert_scim_group("*", &external_id, &display_name, role, &members)
        .await
    {
        Ok(id) => match state.app.list_scim_groups().await {
            Ok(groups) => groups
                .iter()
                .find(|group| group.id == id)
                .map(|group| (StatusCode::CREATED, Json(scim_group(group))).into_response())
                .unwrap_or_else(|| scim_error(&trace, 500, "group not found after create")),
            Err(_) => scim_error(&trace, 500, "group lookup failed"),
        },
        Err(e) => {
            tracing::error!(error = %e, "scim create group failed");
            scim_error(&trace, 500, "group creation failed")
        }
    }
}

pub(crate) async fn get_group(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Path(id): Path<String>,
) -> Response {
    let Some(id) = group_id(&id) else {
        return scim_error(&trace, 404, "group not found");
    };
    match state.app.list_scim_groups().await {
        Ok(groups) => groups
            .iter()
            .find(|group| group.id == id)
            .map(|group| Json(scim_group(group)).into_response())
            .unwrap_or_else(|| scim_error(&trace, 404, "group not found")),
        Err(e) => {
            tracing::error!(error = %e, "scim get group failed");
            scim_error(&trace, 500, "group lookup failed")
        }
    }
}

pub(crate) async fn put_group(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let Some(id) = group_id(&id) else {
        return scim_error(&trace, 404, "group not found");
    };
    let existing = match state.app.list_scim_groups().await {
        Ok(groups) => groups.into_iter().find(|group| group.id == id),
        Err(_) => None,
    };
    let Some(existing) = existing else {
        return scim_error(&trace, 404, "group not found");
    };
    let (mut external_id, display_name, role, members) = match group_payload(&body, &trace) {
        Ok(payload) => payload,
        Err(response) => return response,
    };
    if external_id == display_name {
        external_id = existing.external_id.clone();
    }
    match state
        .app
        .upsert_scim_group("*", &external_id, &display_name, role, &members)
        .await
    {
        Ok(new_id) => match state.app.list_scim_groups().await {
            Ok(groups) => groups
                .iter()
                .find(|group| group.id == new_id)
                .map(|group| Json(scim_group(group)).into_response())
                .unwrap_or_else(|| scim_error(&trace, 500, "group not found after update")),
            Err(_) => scim_error(&trace, 500, "group lookup failed"),
        },
        Err(e) => {
            tracing::error!(error = %e, "scim update group failed");
            scim_error(&trace, 500, "group update failed")
        }
    }
}

pub(crate) async fn delete_group(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Path(id): Path<String>,
) -> Response {
    let Some(id) = group_id(&id) else {
        return scim_error(&trace, 404, "group not found");
    };
    match state.app.delete_scim_group(id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => scim_error(&trace, 404, "group not found"),
        Err(e) => {
            tracing::error!(error = %e, "scim delete group failed");
            scim_error(&trace, 500, "group delete failed")
        }
    }
}
