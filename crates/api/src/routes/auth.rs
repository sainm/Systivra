//! 認証エンドポイント。
//!
//! - `POST /api/v1/auth/login` … 公開
//! - `POST /api/v1/auth/logout` … 認証必須
//! - `GET  /api/v1/auth/me` … 認証必須

use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use contracts::{ErrorCode, LoginRequest, LoginResponse, MeResponse};
use domain::Role;

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
}

/// 認証必須のルート。
pub fn protected_router() -> Router<ApiState> {
    Router::new().route("/api/v1/auth/me", get(me))
}

async fn login(
    State(state): State<ApiState>,
    Extension(trace): Extension<TraceId>,
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
        return Err(ApiError::new(ErrorCode::AuthTokenExpired, &trace)
            .detail("login_id または password が違います"));
    };

    let memberships = state.app.memberships(authed.user_id).await.map_err(|e| {
        tracing::error!(error = %e, "membership lookup failed");
        ApiError::new(ErrorCode::Internal, &trace)
    })?;

    let cookie = state.session.issue(authed.user_id, chrono::Utc::now());

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

async fn logout(State(state): State<ApiState>) -> impl IntoResponse {
    // サーバー側に状態が無いので、消すのはブラウザの Cookie だけ。
    // 盗まれた Cookie は期限まで有効（`auth/session.rs` の注記を参照）。
    (
        StatusCode::NO_CONTENT,
        [(header::SET_COOKIE, state.session.clear_cookie())],
    )
}

async fn me(
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
