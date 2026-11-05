//! HTTP 契約。axum のルーティングとミドルウェアだけを置く。
//!
//! 境界（`02-アーキテクチャ` §3）：`api` は **sqlx / neo4j driver を直接使わない**。
//! `Cargo.toml` に `store` を入れていないのはそのため。DB に触るのは `app` の仕事。

#![forbid(unsafe_code)]

pub mod auth;
pub mod error;
pub mod routes;
pub mod trace;

use std::sync::Arc;

use app::App;
use axum::Router;

use crate::auth::SessionConfig;

/// ハンドラが共有する状態。
#[derive(Clone)]
pub struct ApiState {
    pub app: Arc<App>,
    pub session: Arc<SessionConfig>,
}

impl ApiState {
    pub fn new(app: App, session: SessionConfig) -> Self {
        Self {
            app: Arc::new(app),
            session: Arc::new(session),
        }
    }
}

/// ルータを組み立てる。認証の要否で 2 つに分ける。
///
/// `require_auth` を全体に掛けると `/healthz` が 401 になり、
/// Kubernetes の liveness probe が落ちてコンテナが再起動を繰り返す。
/// **公開ルートは明示的に外に出す。**
pub fn router(app: App, session: SessionConfig) -> Router {
    let state = ApiState::new(app, session);

    let protected = Router::new()
        .merge(routes::auth::protected_router())
        .merge(routes::project::router())
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_auth,
        ));

    Router::new()
        .merge(routes::health::router())
        .merge(routes::auth::public_router())
        .merge(protected)
        // 最も外側。全応答に `X-Trace-Id` を付ける。
        .layer(axum::middleware::from_fn(trace::with_trace_id))
        .with_state(state)
}
