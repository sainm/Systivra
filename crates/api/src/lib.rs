//! HTTP 契約。axum のルーティングとミドルウェアだけを置く。
//!
//! 境界（`02-アーキテクチャ` §3）：`api` は **sqlx / neo4j driver を直接使わない**。
//! `Cargo.toml` に `store` を入れていないのはそのため。DB に触るのは `app` の仕事。

#![forbid(unsafe_code)]

pub mod auth;
pub mod error;
pub mod openapi;
pub mod routes;
pub mod security;
pub mod trace;
pub mod upload;

use std::sync::Arc;

use app::App;
use axum::Router;

use crate::auth::SessionConfig;
use crate::upload::UploadConfig;

/// ハンドラが共有する状態。
#[derive(Clone)]
pub struct ApiState {
    pub app: Arc<App>,
    pub session: Arc<SessionConfig>,
    pub upload: Arc<UploadConfig>,
    /// 未設定ならパスワードログインのみ。IdP 停止中でも API は起動する。
    pub oidc: Option<Arc<providers::OidcRuntime>>,
    pub rate_limiter: Arc<auth::rate_limit::RateLimiter>,
}

impl ApiState {
    pub fn new(
        app: App,
        session: SessionConfig,
        oidc: Option<Arc<providers::OidcRuntime>>,
    ) -> Self {
        let upload = UploadConfig::new(
            session.signing_key().to_vec(),
            chrono::Duration::minutes(15),
        );
        Self {
            app: Arc::new(app),
            session: Arc::new(session),
            upload: Arc::new(upload),
            oidc,
            rate_limiter: Arc::new(auth::rate_limit::RateLimiter::from_env()),
        }
    }
}

/// ルータを組み立てる。認証の要否で 2 つに分ける。
///
/// `require_auth` を全体に掛けると `/healthz` が 401 になり、
/// Kubernetes の liveness probe が落ちてコンテナが再起動を繰り返す。
/// **公開ルートは明示的に外に出す。**
pub fn router(app: App, session: SessionConfig) -> Router {
    router_with_oidc(app, session, None)
}

pub fn router_with_oidc(
    app: App,
    session: SessionConfig,
    oidc: Option<Arc<providers::OidcRuntime>>,
) -> Router {
    let state = ApiState::new(app, session, oidc);

    let public_auth = routes::auth::public_router().route_layer(
        axum::middleware::from_fn_with_state(state.clone(), auth::rate_limit::limit_auth),
    );

    let protected = Router::new()
        .merge(routes::auth::protected_router())
        .merge(routes::project::router())
        .merge(routes::document::project_router())
        .merge(routes::document::resource_router())
        .merge(routes::code::router())
        .merge(routes::index_build::router())
        .merge(routes::graph::router())
        .merge(routes::impact::router())
        .merge(routes::test::router())
        .merge(routes::ai::router())
        .merge(routes::inquiry::router())
        .merge(routes::knowledge::router())
        .merge(routes::job::router())
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_auth,
        ));

    Router::new()
        .merge(routes::health::router())
        .merge(public_auth)
        .merge(routes::upload::router())
        .merge(routes::scim::router())
        .merge(protected)
        // 内側から trace → security の順。全応答に `X-Trace-Id` と
        // セキュリティヘッダを付ける。
        .layer(axum::middleware::from_fn(trace::with_trace_id))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            security::with_security_headers,
        ))
        .with_state(state)
}
