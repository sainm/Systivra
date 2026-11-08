//! SCIM 2.0 の最小実装（Users / Groups）。
//!
//! Bearer は `SCIM_BEARER_TOKEN`。未設定なら 404（機能を隠す）。
//! IdP からのユーザー作成・無効化、グループとロールの同期に使う。

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
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
            get(get_group)
                .put(put_group)
                .patch(patch_group)
                .delete(delete_group),
        )
        .route("/scim/v2/Bulk", axum::routing::post(bulk))
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

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn etag(value: &Value) -> String {
    format!("W/\"{:016x}\"", fnv1a(value.to_string().as_bytes()))
}

fn if_match_ok(headers: &HeaderMap, current: &str) -> bool {
    headers
        .get(header::IF_MATCH)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            let value = value.trim();
            value == "*" || value == current
        })
        .unwrap_or(true)
}

fn not_modified(headers: &HeaderMap, current: &str) -> bool {
    headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim() == current)
        .unwrap_or(false)
}

fn with_etag(mut response: Response, tag: &str) -> Response {
    if let Ok(value) = HeaderValue::from_str(tag) {
        response.headers_mut().insert(header::ETAG, value);
    }
    response
}

fn scim_error_value(status: u16, detail: &str) -> Value {
    json!({
        "schemas": ["urn:ietf:params:scim:api:messages:2.0:Error"],
        "status": status.to_string(),
        "detail": detail
    })
}

fn resource_tag(value: &Value) -> String {
    value["meta"]["version"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn scim_user(user: &app::ScimUserRow) -> Value {
    let emails = user
        .email
        .as_ref()
        .map(|email| vec![json!({"value": email, "primary": true})])
        .unwrap_or_default();
    let mut value = json!({
        "schemas": ["urn:ietf:params:scim:schemas:core:2.0:User"],
        "id": user.id.to_string(),
        "userName": user.login_id,
        "displayName": user.display_name,
        "name": {"formatted": user.display_name},
        "emails": emails,
        "active": user.is_active,
        "meta": {"resourceType": "User", "location": format!("/scim/v2/Users/{}", user.id)}
    });
    let version = etag(&value);
    value["meta"]["version"] = json!(version);
    value
}

fn scim_group(group: &app::ScimGroupRow) -> Value {
    let members: Vec<Value> = group
        .members
        .iter()
        .map(|member| json!({"value": member.id.to_string(), "display": member.login_id}))
        .collect();
    let mut value = json!({
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
    });
    let version = etag(&value);
    value["meta"]["version"] = json!(version);
    value
}

fn apply_group_patch(current: &mut Value, body: &Value) -> Result<(), String> {
    let operations = body
        .get("Operations")
        .and_then(Value::as_array)
        .ok_or_else(|| "Operations is required".to_owned())?;
    for operation in operations {
        let op = operation
            .get("op")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        let path = operation.get("path").and_then(Value::as_str).unwrap_or("");
        let value = operation.get("value");
        if path.is_empty() {
            let Some(object) = value.and_then(Value::as_object) else {
                return Err("patch value must be an object".into());
            };
            for (key, item) in object {
                if key == "members" {
                    current["members"] = item.clone();
                } else if key == "displayName" {
                    current["displayName"] = item.clone();
                } else if key == ROLE_EXTENSION {
                    if let Some(role) = item.get("role") {
                        current[ROLE_EXTENSION]["role"] = role.clone();
                    }
                }
            }
            continue;
        }
        if path.eq_ignore_ascii_case("displayName") && matches!(op.as_str(), "replace" | "add") {
            current["displayName"] = value.cloned().ok_or("displayName value is required")?;
        } else if path.eq_ignore_ascii_case("members") {
            let incoming: Vec<Value> = value
                .and_then(Value::as_array)
                .cloned()
                .ok_or("members value is required")?;
            let mut members = current["members"].as_array().cloned().unwrap_or_default();
            match op.as_str() {
                "add" => {
                    for member in incoming {
                        if !members
                            .iter()
                            .any(|existing| existing.get("value") == member.get("value"))
                        {
                            members.push(member);
                        }
                    }
                }
                "remove" => {
                    for member in &incoming {
                        let id = member.get("value").and_then(Value::as_str);
                        members
                            .retain(|existing| existing.get("value").and_then(Value::as_str) != id);
                    }
                }
                "replace" => members = incoming,
                _ => return Err("unsupported members operation".into()),
            }
            current["members"] = Value::Array(members);
        } else if path.eq_ignore_ascii_case(ROLE_EXTENSION)
            || path.eq_ignore_ascii_case(&format!("{ROLE_EXTENSION}.role"))
        {
            let role = match value {
                Some(Value::String(role)) => json!(role),
                Some(other) => other.get("role").cloned().ok_or("role value is required")?,
                None => return Err("role value is required".into()),
            };
            current[ROLE_EXTENSION]["role"] = role;
        } else {
            return Err(format!("unsupported patch path: {path}"));
        }
    }
    Ok(())
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
        "bulk": {"supported": true, "maxOperations": 50, "maxPayloadSize": 1048576},
        "filter": {"supported": true, "maxResults": 200},
        "changePassword": {"supported": false},
        "sort": {"supported": false},
        "etag": {"supported": true},
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
    headers: HeaderMap,
) -> Response {
    let Some(id) = user_id(&id) else {
        return scim_error(&trace, 404, "user not found");
    };
    match state.app.find_scim_user(id).await {
        Ok(Some(user)) => {
            let value = scim_user(&user);
            let tag = resource_tag(&value);
            if not_modified(&headers, &tag) {
                return StatusCode::NOT_MODIFIED.into_response();
            }
            with_etag(Json(value).into_response(), &tag)
        }
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
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let Some(id) = user_id(&id) else {
        return scim_error(&trace, 404, "user not found");
    };
    let current = match state.app.find_scim_user(id).await {
        Ok(Some(user)) => user,
        Ok(None) => return scim_error(&trace, 404, "user not found"),
        Err(e) => {
            tracing::error!(error = %e, "scim get user failed");
            return scim_error(&trace, 500, "user lookup failed");
        }
    };
    if !if_match_ok(&headers, &resource_tag(&scim_user(&current))) {
        return scim_error(&trace, 412, "If-Match does not match");
    }
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
            Ok(Some(user)) => {
                let value = scim_user(&user);
                let tag = resource_tag(&value);
                with_etag(Json(value).into_response(), &tag)
            }
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
    headers: HeaderMap,
) -> Response {
    let Some(id) = group_id(&id) else {
        return scim_error(&trace, 404, "group not found");
    };
    match state.app.list_scim_groups().await {
        Ok(groups) => match groups.iter().find(|group| group.id == id) {
            Some(group) => {
                let value = scim_group(group);
                let tag = resource_tag(&value);
                if not_modified(&headers, &tag) {
                    return StatusCode::NOT_MODIFIED.into_response();
                }
                with_etag(Json(value).into_response(), &tag)
            }
            None => scim_error(&trace, 404, "group not found"),
        },
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
    headers: HeaderMap,
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
    if !if_match_ok(&headers, &resource_tag(&scim_group(&existing))) {
        return scim_error(&trace, 412, "If-Match does not match");
    }
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
                .map(|group| {
                    let value = scim_group(group);
                    let tag = resource_tag(&value);
                    with_etag(Json(value).into_response(), &tag)
                })
                .unwrap_or_else(|| scim_error(&trace, 500, "group not found after update")),
            Err(_) => scim_error(&trace, 500, "group lookup failed"),
        },
        Err(e) => {
            tracing::error!(error = %e, "scim update group failed");
            scim_error(&trace, 500, "group update failed")
        }
    }
}

pub(crate) async fn patch_group(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let Some(id) = group_id(&id) else {
        return scim_error(&trace, 404, "group not found");
    };
    let existing = match state.app.list_scim_groups().await {
        Ok(groups) => groups.into_iter().find(|group| group.id == id),
        Err(e) => {
            tracing::error!(error = %e, "scim get group failed");
            return scim_error(&trace, 500, "group lookup failed");
        }
    };
    let Some(existing) = existing else {
        return scim_error(&trace, 404, "group not found");
    };
    let mut patched = scim_group(&existing);
    if !if_match_ok(&headers, &resource_tag(&patched)) {
        return scim_error(&trace, 412, "If-Match does not match");
    }
    if let Err(message) = apply_group_patch(&mut patched, &body) {
        return scim_error(&trace, 400, &message);
    }
    let (external_id, display_name, role, members) = match group_payload(&patched, &trace) {
        Ok(payload) => payload,
        Err(response) => return response,
    };
    match state
        .app
        .upsert_scim_group("*", &external_id, &display_name, role, &members)
        .await
    {
        Ok(new_id) => match state.app.list_scim_groups().await {
            Ok(groups) => groups
                .iter()
                .find(|group| group.id == new_id)
                .map(|group| {
                    let value = scim_group(group);
                    let tag = resource_tag(&value);
                    with_etag(Json(value).into_response(), &tag)
                })
                .unwrap_or_else(|| scim_error(&trace, 500, "group not found after patch")),
            Err(_) => scim_error(&trace, 500, "group lookup failed"),
        },
        Err(e) => {
            tracing::error!(error = %e, "scim patch group failed");
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

async fn dispatch_bulk(
    state: &ApiState,
    trace: &TraceId,
    method: &str,
    path: &str,
    data: Value,
) -> (u16, Option<Value>, bool) {
    let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
    match (method, segments.as_slice()) {
        ("POST", ["Users"]) => {
            let Some(login_id) = data.get("userName").and_then(Value::as_str) else {
                return (
                    400,
                    Some(scim_error_value(400, "userName is required")),
                    false,
                );
            };
            let display_name = data
                .get("displayName")
                .and_then(Value::as_str)
                .unwrap_or(login_id);
            let email = data
                .get("emails")
                .and_then(Value::as_array)
                .and_then(|emails| emails.first())
                .and_then(|email| email.get("value"))
                .and_then(Value::as_str);
            let active = data.get("active").and_then(Value::as_bool).unwrap_or(true);
            if let Ok(users) = state.app.list_scim_users().await {
                if users.iter().any(|user| user.login_id == login_id) {
                    return (
                        409,
                        Some(scim_error_value(409, "userName already exists")),
                        false,
                    );
                }
            }
            match state
                .app
                .create_scim_user(login_id, display_name, email, active)
                .await
            {
                Ok(user) => (201, Some(scim_user(&user)), true),
                Err(e) => {
                    tracing::error!(error = %e, "bulk create user failed");
                    (
                        500,
                        Some(scim_error_value(500, "user creation failed")),
                        false,
                    )
                }
            }
        }
        ("PATCH", ["Users", id]) => {
            let Some(id) = user_id(id) else {
                return (404, Some(scim_error_value(404, "user not found")), false);
            };
            let Some(active) =
                data.get("Operations")
                    .and_then(Value::as_array)
                    .and_then(|operations| {
                        operations.iter().find_map(|operation| {
                            let path = operation.get("path").and_then(Value::as_str).unwrap_or("");
                            if path.eq_ignore_ascii_case("active") {
                                operation.get("value").and_then(Value::as_bool)
                            } else {
                                None
                            }
                        })
                    })
            else {
                return (400, Some(scim_error_value(400, "unsupported patch")), false);
            };
            match state.app.set_user_active(id, active).await {
                Ok(true) => match state.app.find_scim_user(id).await {
                    Ok(Some(user)) => (200, Some(scim_user(&user)), true),
                    _ => (404, Some(scim_error_value(404, "user not found")), false),
                },
                Ok(false) => (404, Some(scim_error_value(404, "user not found")), false),
                Err(e) => {
                    tracing::error!(error = %e, "bulk patch user failed");
                    (
                        500,
                        Some(scim_error_value(500, "user update failed")),
                        false,
                    )
                }
            }
        }
        ("DELETE", ["Users", id]) => {
            let Some(id) = user_id(id) else {
                return (404, Some(scim_error_value(404, "user not found")), false);
            };
            match state.app.set_user_active(id, false).await {
                Ok(true) => (204, None, true),
                Ok(false) => (404, Some(scim_error_value(404, "user not found")), false),
                Err(e) => {
                    tracing::error!(error = %e, "bulk delete user failed");
                    (
                        500,
                        Some(scim_error_value(500, "user update failed")),
                        false,
                    )
                }
            }
        }
        ("POST", ["Groups"]) => {
            let (external_id, display_name, role, members) = match group_payload(&data, trace) {
                Ok(payload) => payload,
                Err(_) => {
                    return (
                        400,
                        Some(scim_error_value(400, "invalid group payload")),
                        false,
                    )
                }
            };
            match state
                .app
                .upsert_scim_group("*", &external_id, &display_name, role, &members)
                .await
            {
                Ok(id) => match state.app.list_scim_groups().await {
                    Ok(groups) => match groups.iter().find(|group| group.id == id) {
                        Some(group) => (201, Some(scim_group(group)), true),
                        None => (500, Some(scim_error_value(500, "group not found")), false),
                    },
                    Err(_) => (
                        500,
                        Some(scim_error_value(500, "group lookup failed")),
                        false,
                    ),
                },
                Err(e) => {
                    tracing::error!(error = %e, "bulk create group failed");
                    (
                        500,
                        Some(scim_error_value(500, "group creation failed")),
                        false,
                    )
                }
            }
        }
        ("PATCH", ["Groups", id]) => {
            let Some(id) = group_id(id) else {
                return (404, Some(scim_error_value(404, "group not found")), false);
            };
            let existing = match state.app.list_scim_groups().await {
                Ok(groups) => groups.into_iter().find(|group| group.id == id),
                Err(_) => None,
            };
            let Some(existing) = existing else {
                return (404, Some(scim_error_value(404, "group not found")), false);
            };
            let mut patched = scim_group(&existing);
            if let Err(message) = apply_group_patch(&mut patched, &data) {
                return (400, Some(scim_error_value(400, &message)), false);
            }
            let (external_id, display_name, role, members) = match group_payload(&patched, trace) {
                Ok(payload) => payload,
                Err(_) => {
                    return (
                        400,
                        Some(scim_error_value(400, "invalid group payload")),
                        false,
                    )
                }
            };
            match state
                .app
                .upsert_scim_group("*", &external_id, &display_name, role, &members)
                .await
            {
                Ok(new_id) => match state.app.list_scim_groups().await {
                    Ok(groups) => match groups.iter().find(|group| group.id == new_id) {
                        Some(group) => (200, Some(scim_group(group)), true),
                        None => (500, Some(scim_error_value(500, "group not found")), false),
                    },
                    Err(_) => (
                        500,
                        Some(scim_error_value(500, "group lookup failed")),
                        false,
                    ),
                },
                Err(e) => {
                    tracing::error!(error = %e, "bulk patch group failed");
                    (
                        500,
                        Some(scim_error_value(500, "group update failed")),
                        false,
                    )
                }
            }
        }
        ("DELETE", ["Groups", id]) => {
            let Some(id) = group_id(id) else {
                return (404, Some(scim_error_value(404, "group not found")), false);
            };
            match state.app.delete_scim_group(id).await {
                Ok(true) => (204, None, true),
                Ok(false) => (404, Some(scim_error_value(404, "group not found")), false),
                Err(e) => {
                    tracing::error!(error = %e, "bulk delete group failed");
                    (
                        500,
                        Some(scim_error_value(500, "group delete failed")),
                        false,
                    )
                }
            }
        }
        _ => (
            501,
            Some(scim_error_value(501, "unsupported bulk operation")),
            false,
        ),
    }
}

pub(crate) async fn bulk(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Json(body): Json<Value>,
) -> Response {
    let Some(operations) = body.get("Operations").and_then(Value::as_array) else {
        return scim_error(&trace, 400, "Operations is required");
    };
    if operations.len() > 50 {
        return scim_error(&trace, 413, "too many operations");
    }
    let fail_on = body
        .get("failOnErrors")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let mut results = Vec::new();
    let mut errors = 0usize;
    for operation in operations {
        if fail_on > 0 && errors >= fail_on {
            break;
        }
        let method = operation
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_uppercase();
        let path = operation.get("path").and_then(Value::as_str).unwrap_or("");
        let data = operation.get("data").cloned().unwrap_or(Value::Null);
        let (status, response, ok) = dispatch_bulk(&state, &trace, &method, path, data).await;
        if !ok {
            errors += 1;
        }
        let mut item = json!({"method": method, "path": path, "status": status.to_string()});
        if let Some(bulk_id) = operation.get("bulkId") {
            item["bulkId"] = bulk_id.clone();
        }
        if let Some(response) = response {
            item["response"] = response;
        }
        results.push(item);
    }
    Json(json!({
        "schemas": ["urn:ietf:params:scim:api:messages:2.0:BulkResponse"],
        "Operations": results
    }))
    .into_response()
}
