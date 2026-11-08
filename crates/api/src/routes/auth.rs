//! 認証エンドポイント。
//!
//! - `POST /api/v1/auth/login` … 公開
//! - `POST /api/v1/auth/logout` … 認証必須
//! - `GET  /api/v1/auth/me` … 認証必須

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{delete, get, post};
use axum::{Extension, Form, Json, Router};
use chrono::Utc;
use contracts::{
    AuthConfigResponse, ErrorCode, LoginRequest, LoginResponse, MeResponse, OidcProviderSummary,
    RevokeOtherSessionsResponse, SessionListResponse, SessionSummary,
};
use domain::{Role, SessionId};
use serde::{Deserialize, Serialize};
use utoipa::IntoParams;
use uuid::Uuid;

use crate::auth::rate_limit::ClientInfo;
use crate::auth::session::{cookie_value, FLOW_COOKIE_NAME};
use crate::error::ApiError;
use crate::trace::TraceId;
use crate::ApiState;

/// 認証不要のルート。
///
/// `logout` を含めるのは意図的。セッションが切れた状態でも Cookie を
/// 消せないと、利用者は「ログアウトできない」状態で詰まる。
/// Cookie を消すのに権限は要らない。
pub fn public_router() -> Router<ApiState> {
    Router::new()
        .route("/api/v1/auth/login", post(login))
        .route("/api/v1/auth/logout", post(logout))
        .route(
            "/api/v1/auth/oidc/backchannel-logout",
            post(oidc_backchannel_logout),
        )
        .route(
            "/api/v1/auth/oidc/frontchannel-logout",
            get(oidc_frontchannel_logout).post(oidc_frontchannel_logout_post),
        )
        .route("/api/v1/auth/config", get(auth_config))
        .route("/api/v1/auth/oidc/start", get(oidc_start))
        .route("/api/v1/auth/oidc/callback", get(oidc_callback))
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct OidcStartQuery {
    pub return_to: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct OidcCallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OidcFlow {
    state: String,
    nonce: String,
    verifier: String,
    return_to: String,
    exp: i64,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct BackchannelLogoutForm {
    pub logout_token: String,
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/oidc/backchannel-logout",
    responses(
        (status = 200, description = "logout token を受け付け、該当セッションを失効"),
        (status = 400, body = contracts::Problem)
    )
)]
pub(crate) async fn oidc_backchannel_logout(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Form(form): Form<BackchannelLogoutForm>,
) -> Result<StatusCode, ApiError> {
    let runtime = state.oidc.as_ref().ok_or_else(|| {
        ApiError::new(ErrorCode::AuthSsoUnavailable, &trace).detail("SSO is not configured")
    })?;
    let client = runtime.client().await.map_err(|e| {
        tracing::warn!(error = %e, "oidc discovery failed");
        ApiError::new(ErrorCode::AuthSsoUnavailable, &trace).detail("SSO provider is unavailable")
    })?;

    let logout = client
        .verify_logout_token(&form.logout_token)
        .map_err(|e| {
            tracing::warn!(error = %e, "logout token verification failed");
            ApiError::new(ErrorCode::AuthLogoutTokenInvalid, &trace)
                .detail("logout token is invalid")
        })?;

    if !runtime.remember_logout_jti(&logout.jti).await {
        return Err(ApiError::new(ErrorCode::AuthLogoutTokenInvalid, &trace)
            .detail("logout token was already used"));
    }

    let provider = runtime.config().issuer.clone();
    let revoked = state
        .app
        .revoke_identity_sessions(&provider, logout.subject.as_deref(), logout.sid.as_deref())
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "session revoke failed");
            ApiError::new(ErrorCode::Internal, &trace)
        })?;
    if let Err(e) = state
        .app
        .record_auth_audit(&app::AuthAudit {
            project: None,
            actor: None,
            action: "auth.sso.logout",
            result: "SUCCESS",
            provider: &provider,
            subject: logout.subject.as_deref(),
            reason: None,
            trace_id: Some(trace.as_str()),
        })
        .await
    {
        tracing::error!(error = %e, "sso logout audit failed");
    }
    tracing::info!(revoked, "back-channel logout applied");
    Ok(StatusCode::OK)
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct FrontChannelLogoutQuery {
    pub iss: Option<String>,
    pub sid: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct FrontChannelLogoutForm {
    pub iss: Option<String>,
    pub sid: Option<String>,
}

async fn frontchannel_logout(
    state: &ApiState,
    trace: &TraceId,
    iss: Option<String>,
    sid: Option<String>,
) -> Result<StatusCode, ApiError> {
    let expected = std::env::var("OIDC_ISSUER").unwrap_or_default();
    if expected.is_empty() {
        return Err(
            ApiError::new(ErrorCode::AuthSsoUnavailable, trace).detail("SSO is not configured")
        );
    }
    let (Some(iss), Some(sid)) = (iss, sid.filter(|value| !value.is_empty())) else {
        return Err(
            ApiError::new(ErrorCode::ValidationSchema, trace).detail("iss and sid are required")
        );
    };
    if iss != expected {
        return Err(ApiError::new(ErrorCode::ValidationSchema, trace).detail("issuer mismatch"));
    }
    let revoked = state
        .app
        .revoke_identity_sessions(&expected, None, Some(&sid))
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "front-channel logout failed");
            ApiError::new(ErrorCode::Internal, trace)
        })?;
    if let Err(e) = state
        .app
        .record_auth_audit(&app::AuthAudit {
            project: None,
            actor: None,
            action: "auth.sso.frontchannel_logout",
            result: "SUCCESS",
            provider: &expected,
            subject: None,
            reason: Some(&sid),
            trace_id: Some(trace.as_str()),
        })
        .await
    {
        tracing::error!(error = %e, "front-channel logout audit failed");
    }
    tracing::info!(revoked, "front-channel logout applied");
    Ok(StatusCode::OK)
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/oidc/frontchannel-logout",
    params(
        ("iss" = Option<String>, Query, description = "Issuer"),
        ("sid" = Option<String>, Query, description = "IdP session ID")
    ),
    responses(
        (status = 200, description = "sid 一致セッションを失効"),
        (status = 400, body = contracts::Problem)
    )
)]
pub(crate) async fn oidc_frontchannel_logout(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Query(query): Query<FrontChannelLogoutQuery>,
) -> Result<StatusCode, ApiError> {
    frontchannel_logout(&state, &trace, query.iss, query.sid).await
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/oidc/frontchannel-logout",
    request_body = FrontChannelLogoutForm,
    responses(
        (status = 200, description = "sid 一致セッションを失効"),
        (status = 400, body = contracts::Problem)
    )
)]
pub(crate) async fn oidc_frontchannel_logout_post(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Form(form): Form<FrontChannelLogoutForm>,
) -> Result<StatusCode, ApiError> {
    frontchannel_logout(&state, &trace, form.iss, form.sid).await
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/sessions",
    responses((status = 200, body = SessionListResponse))
)]
pub(crate) async fn list_sessions(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<app::Authenticated>,
) -> Result<Json<SessionListResponse>, ApiError> {
    let rows = state.app.list_sessions(auth.user_id).await.map_err(|e| {
        tracing::error!(error = %e, "session list failed");
        ApiError::new(ErrorCode::Internal, &trace)
    })?;
    let sessions = rows
        .into_iter()
        .map(|row| SessionSummary {
            id: SessionId(row.id),
            created_at: row.created_at,
            last_seen_at: row.last_seen_at,
            expires_at: row.expires_at,
            user_agent: row.user_agent,
            provider: row.provider,
            current: auth.session_id == Some(row.id),
        })
        .collect();
    Ok(Json(SessionListResponse { sessions }))
}

#[utoipa::path(
    delete,
    path = "/api/v1/auth/sessions/{sid}",
    params(("sid" = uuid::Uuid, Path, description = "Session ID")),
    responses((status = 204), (status = 404, body = contracts::Problem))
)]
pub(crate) async fn revoke_session(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<app::Authenticated>,
    Path(sid): Path<String>,
) -> Result<StatusCode, ApiError> {
    let session: Uuid = sid
        .parse()
        .map_err(|_| ApiError::new(ErrorCode::NotFound, &trace))?;
    let revoked = state
        .app
        .revoke_user_session(auth.user_id, session)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "session revoke failed");
            ApiError::new(ErrorCode::Internal, &trace)
        })?;
    if !revoked {
        return Err(ApiError::new(ErrorCode::NotFound, &trace));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/logout-all",
    responses((status = 200, body = RevokeOtherSessionsResponse))
)]
pub(crate) async fn logout_all(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<app::Authenticated>,
) -> Result<Json<RevokeOtherSessionsResponse>, ApiError> {
    let keep = auth.session_id.unwrap_or_else(Uuid::nil);
    let revoked = state
        .app
        .revoke_other_sessions(auth.user_id, keep)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "logout-all failed");
            ApiError::new(ErrorCode::Internal, &trace)
        })?;
    Ok(Json(RevokeOtherSessionsResponse { revoked }))
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/config",
    responses((status = 200, body = AuthConfigResponse))
)]
pub(crate) async fn auth_config(State(state): State<ApiState>) -> Json<AuthConfigResponse> {
    Json(AuthConfigResponse {
        password_login: true,
        oidc: state.oidc.as_ref().map(|oidc| OidcProviderSummary {
            provider_label: oidc.config().provider_label.clone(),
            start_url: "/api/v1/auth/oidc/start".to_owned(),
        }),
    })
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/oidc/start",
    params(
        ("return_to" = Option<String>, Query, description = "ログイン後の戻り先（相対パスのみ）")
    ),
    responses(
        (status = 303, description = "IdP の認可エンドポイントへリダイレクト"),
        (status = 503, body = contracts::Problem)
    )
)]
pub(crate) async fn oidc_start(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Query(query): Query<OidcStartQuery>,
) -> Result<Response, ApiError> {
    let runtime = state.oidc.as_ref().ok_or_else(|| {
        ApiError::new(ErrorCode::AuthSsoUnavailable, &trace).detail("SSO is not configured")
    })?;

    let client = runtime.client().await.map_err(|e| {
        tracing::warn!(error = %e, "oidc discovery failed");
        ApiError::new(ErrorCode::AuthSsoUnavailable, &trace).detail("SSO provider is unavailable")
    })?;

    let flow = OidcFlow {
        state: random_token(2),
        nonce: random_token(2),
        verifier: random_token(3),
        return_to: safe_return_to(query.return_to.as_deref()),
        exp: Utc::now().timestamp() + state.session.flow_ttl_minutes() * 60,
    };

    let url = client
        .authorization_url(&flow.state, &flow.nonce, &flow.verifier)
        .map_err(|e| {
            tracing::warn!(error = %e, "oidc authorization url failed");
            ApiError::new(ErrorCode::AuthSsoUnavailable, &trace)
                .detail("SSO authorization is unavailable")
        })?;

    let payload = serde_json::to_vec(&flow).map_err(|e| {
        tracing::error!(error = %e, "oidc flow serialization failed");
        ApiError::new(ErrorCode::Internal, &trace)
    })?;
    let sealed = state.session.seal("oidc-flow", &payload);

    let mut response = Redirect::to(&url).into_response();
    let cookie = HeaderValue::from_str(&state.session.set_flow_cookie(&sealed)).map_err(|e| {
        tracing::error!(error = %e, "oidc flow cookie is invalid");
        ApiError::new(ErrorCode::Internal, &trace)
    })?;
    response.headers_mut().append(header::SET_COOKIE, cookie);
    Ok(response)
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/oidc/callback",
    params(
        ("code" = Option<String>, Query, description = "認可コード"),
        ("state" = Option<String>, Query, description = "CSRF state"),
        ("error" = Option<String>, Query, description = "IdP が返したエラー"),
        ("error_description" = Option<String>, Query, description = "IdP のエラー詳細")
    ),
    responses((status = 303, description = "ログイン画面または元の画面へリダイレクト"))
)]
pub(crate) async fn oidc_callback(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    headers: HeaderMap,
    client_info: ClientInfo,
    Query(query): Query<OidcCallbackQuery>,
) -> Response {
    let Some(runtime) = state.oidc.as_ref() else {
        return error_redirect("unavailable");
    };
    let provider = runtime.config().issuer.clone();

    let Some(raw) = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|c| cookie_value(c, FLOW_COOKIE_NAME))
    else {
        audit_failure(&state, &trace, &provider, None, "flow_cookie_missing").await;
        return error_redirect("state_invalid");
    };

    let Some(payload) = state.session.open("oidc-flow", raw) else {
        audit_failure(&state, &trace, &provider, None, "flow_cookie_invalid").await;
        return error_redirect("state_invalid");
    };
    let Ok(flow) = serde_json::from_slice::<OidcFlow>(&payload) else {
        audit_failure(&state, &trace, &provider, None, "flow_cookie_invalid").await;
        return error_redirect("state_invalid");
    };
    if flow.exp <= Utc::now().timestamp() {
        audit_failure(&state, &trace, &provider, None, "flow_expired").await;
        return error_redirect("state_expired");
    }

    if query.error.is_some() {
        audit_failure(&state, &trace, &provider, None, "idp_error").await;
        return error_redirect("provider_error");
    }
    if query.state.as_deref() != Some(flow.state.as_str()) {
        audit_failure(&state, &trace, &provider, None, "state_mismatch").await;
        return error_redirect("state_invalid");
    }
    let Some(code) = query.code.as_deref().filter(|c| !c.is_empty()) else {
        audit_failure(&state, &trace, &provider, None, "code_missing").await;
        return error_redirect("exchange_failed");
    };

    let Some(client) = runtime.client().await.ok() else {
        audit_failure(&state, &trace, &provider, None, "discovery_failed").await;
        return error_redirect("unavailable");
    };
    let identity = match client.exchange(code, &flow.verifier, &flow.nonce).await {
        Ok(identity) => identity,
        Err(e) => {
            tracing::warn!(error = %e, "oidc token exchange failed");
            runtime.invalidate().await;
            audit_failure(&state, &trace, &provider, None, "token_exchange_failed").await;
            return error_redirect("exchange_failed");
        }
    };

    let Some(authed) = state
        .app
        .authenticate_external_identity(&provider, &identity.subject)
        .await
        .ok()
        .flatten()
    else {
        audit_failure(
            &state,
            &trace,
            &provider,
            Some(&identity.subject),
            "identity_unlinked",
        )
        .await;
        return error_redirect("not_linked");
    };

    if let Err(e) = state
        .app
        .sync_group_roles(authed.user_id, &provider, &identity.groups)
        .await
    {
        tracing::warn!(error = %e, "group role sync failed");
    }

    let memberships = match state.app.memberships(authed.user_id).await {
        Ok(m) => m,
        Err(e) => {
            tracing::error!(error = %e, "sso membership lookup failed");
            audit_failure(
                &state,
                &trace,
                &provider,
                Some(&identity.subject),
                "membership_lookup_failed",
            )
            .await;
            return error_redirect("internal");
        }
    };

    let now = Utc::now();
    let ip_hash = state.session.fingerprint(&client_info.ip);
    let session_id = match state
        .app
        .create_session(
            authed.user_id,
            now + state.session.ttl(),
            Some(&ip_hash),
            client_info.user_agent.as_deref(),
            &app::SessionIdentity {
                provider: Some(&provider),
                subject: Some(&identity.subject),
                idp_sid: identity.sid.as_deref(),
            },
        )
        .await
    {
        Ok(id) => id,
        Err(e) => {
            tracing::error!(error = %e, "sso session creation failed");
            audit_failure(
                &state,
                &trace,
                &provider,
                Some(&identity.subject),
                "session_creation_failed",
            )
            .await;
            return error_redirect("internal");
        }
    };

    if let Err(e) = state
        .app
        .record_auth_audit(&app::AuthAudit {
            project: memberships.first().map(|m| m.project_id),
            actor: Some(authed.user_id),
            action: "auth.sso.login",
            result: "SUCCESS",
            provider: &provider,
            subject: Some(&identity.subject),
            reason: None,
            trace_id: Some(trace.as_str()),
        })
        .await
    {
        tracing::error!(error = %e, "sso success audit failed");
    }

    let cookie = state.session.issue(session_id, authed.user_id, now);
    let mut response = Redirect::to(&flow.return_to).into_response();
    for cookie in [
        state.session.set_cookie(&cookie),
        state.session.clear_flow_cookie(),
    ] {
        if let Ok(value) = HeaderValue::from_str(&cookie) {
            response.headers_mut().append(header::SET_COOKIE, value);
        }
    }
    response
}

fn error_redirect(reason: &str) -> Response {
    Redirect::to(&format!("/login?sso_error={reason}")).into_response()
}

async fn audit_failure(
    state: &ApiState,
    trace: &TraceId,
    provider: &str,
    subject: Option<&str>,
    reason: &str,
) {
    if let Err(e) = state
        .app
        .record_auth_audit(&app::AuthAudit {
            project: None,
            actor: None,
            action: "auth.sso.login",
            result: "FAILED",
            provider,
            subject,
            reason: Some(reason),
            trace_id: Some(trace.as_str()),
        })
        .await
    {
        tracing::error!(error = %e, "sso failure audit failed");
    }
}

fn random_token(uuids: usize) -> String {
    (0..uuids)
        .map(|_| Uuid::new_v4().simple().to_string())
        .collect::<Vec<_>>()
        .join("")
}

fn safe_return_to(raw: Option<&str>) -> String {
    let candidate = raw.unwrap_or("/");
    if candidate.starts_with('/')
        && !candidate.starts_with("//")
        && !candidate.contains('\\')
        && !candidate.chars().any(char::is_control)
    {
        candidate.to_owned()
    } else {
        "/".to_owned()
    }
}

/// 認証必須のルート。
pub fn protected_router() -> Router<ApiState> {
    Router::new()
        .route("/api/v1/auth/me", get(me))
        .route("/api/v1/auth/sessions", get(list_sessions))
        .route("/api/v1/auth/sessions/{sid}", delete(revoke_session))
        .route("/api/v1/auth/logout-all", post(logout_all))
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/login",
    request_body = LoginRequest,
    responses(
        (status = 200, body = LoginResponse),
        (status = 400, body = contracts::Problem),
        (status = 401, body = contracts::Problem)
    )
)]
pub(crate) async fn login(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    client: ClientInfo,
    Json(req): Json<LoginRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if req.login_id.trim().is_empty() || req.password.is_empty() {
        return Err(ApiError::new(ErrorCode::ValidationSchema, &trace)
            .detail("login_id と password は必須です"));
    }

    let authed = state
        .app
        .authenticate_password(&req.login_id, &req.password)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "login failed due to backend error");
            ApiError::new(ErrorCode::Internal, &trace)
        })?;

    let Some(authed) = authed else {
        // ユーザー不在・パスワード誤り・無効化を**区別しない**。
        // 区別すると、ログイン画面がアカウントの存在確認に使われる。
        if let Err(e) = state
            .app
            .record_auth_audit(&app::AuthAudit {
                project: None,
                actor: None,
                action: "auth.password.login",
                result: "FAILED",
                provider: "password",
                subject: Some(&req.login_id),
                reason: Some("invalid_credentials"),
                trace_id: Some(trace.as_str()),
            })
            .await
        {
            tracing::error!(error = %e, "password failure audit failed");
        }
        return Err(ApiError::new(ErrorCode::AuthTokenExpired, &trace)
            .detail("login_id または password が違います"));
    };

    let memberships = state.app.memberships(authed.user_id).await.map_err(|e| {
        tracing::error!(error = %e, "membership lookup failed");
        ApiError::new(ErrorCode::Internal, &trace)
    })?;

    let now = Utc::now();
    let ip_hash = state.session.fingerprint(&client.ip);
    let session = state
        .app
        .create_session(
            authed.user_id,
            now + state.session.ttl(),
            Some(&ip_hash),
            client.user_agent.as_deref(),
            &app::SessionIdentity::default(),
        )
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "session creation failed");
            ApiError::new(ErrorCode::Internal, &trace)
        })?;
    let cookie = state.session.issue(session, authed.user_id, now);

    if let Err(e) = state
        .app
        .record_auth_audit(&app::AuthAudit {
            project: memberships.first().map(|m| m.project_id),
            actor: Some(authed.user_id),
            action: "auth.password.login",
            result: "SUCCESS",
            provider: "password",
            subject: Some(&req.login_id),
            reason: None,
            trace_id: Some(trace.as_str()),
        })
        .await
    {
        tracing::error!(error = %e, "password success audit failed");
    }

    let body = LoginResponse {
        user_id: authed.user_id,
        display_name: authed.display_name,
        memberships,
    };

    Ok((
        StatusCode::OK,
        [(header::SET_COOKIE, state.session.set_cookie(&cookie))],
        Json(body),
    ))
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/logout",
    responses((status = 204))
)]
pub(crate) async fn logout(State(state): State<ApiState>, headers: HeaderMap) -> impl IntoResponse {
    // Cookie が無い・壊れている・既に失効していても 204 で消す。
    let raw = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|c| cookie_value(c, crate::auth::session::COOKIE_NAME));
    if let Some(claims) = raw.and_then(|r| state.session.verify(r, Utc::now())) {
        if let Err(e) = state.app.revoke_session(claims.session_id).await {
            tracing::error!(error = %e, "session revoke failed");
        }
    }
    (
        StatusCode::NO_CONTENT,
        [(header::SET_COOKIE, state.session.clear_cookie())],
    )
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/me",
    responses((status = 200, body = MeResponse), (status = 401, body = contracts::Problem))
)]
pub(crate) async fn me(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
    Extension(auth): Extension<app::Authenticated>,
) -> Result<Json<MeResponse>, ApiError> {
    let memberships = state.app.memberships(auth.user_id).await.map_err(|e| {
        tracing::error!(error = %e, "membership lookup failed");
        ApiError::new(ErrorCode::Internal, &trace)
    })?;

    Ok(Json(MeResponse {
        user_id: auth.user_id,
        display_name: auth.display_name,
        memberships,
    }))
}

/// 画面が「この操作を出してよいか」を判断するための一覧。
/// 判定ロジックをフロントに複製させない。
pub fn writable_permissions(role: Role) -> Vec<String> {
    domain::Permission::ALL
        .iter()
        .filter(|p| p.writable_by(role))
        .map(|p| p.as_str().to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn return_to_accepts_relative_paths_only() {
        assert_eq!(safe_return_to(Some("/projects?x=1")), "/projects?x=1");
        assert_eq!(safe_return_to(None), "/");
        assert_eq!(safe_return_to(Some("//evil.example")), "/");
        assert_eq!(safe_return_to(Some("https://evil.example")), "/");
        assert_eq!(safe_return_to(Some("/a\\b")), "/");
        assert_eq!(safe_return_to(Some("/a\nb")), "/");
    }

    #[test]
    fn random_token_has_expected_length() {
        assert_eq!(random_token(2).len(), 64);
        assert_eq!(random_token(3).len(), 96);
        assert_ne!(random_token(2), random_token(2));
    }
}
