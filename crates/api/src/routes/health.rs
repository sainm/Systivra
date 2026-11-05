//! ヘルスチェック。
//!
//! 2 つを分ける。混ぜると「DB が落ちたからコンテナを再起動」という
//! 最悪の動きになる。
//!
//! - `/healthz` … プロセスが生きているか。**DB を見ない。** 常に 200。
//! - `/readyz` … 依存が使えるか。DB を見る。落ちていれば 503。

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::ApiState;

#[derive(Serialize)]
pub struct Health {
    status: &'static str,
    version: &'static str,
}

#[derive(Serialize)]
pub struct Ready {
    ready: bool,
    database: DependencyView,
}

#[derive(Serialize)]
#[serde(tag = "state", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DependencyView {
    Ok { latency_ms: u64 },
    Failed { reason: String },
}

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
}

/// liveness。依存を一切見ない。
pub async fn healthz() -> impl IntoResponse {
    Json(Health {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

/// readiness。DB を見る。
pub async fn readyz(State(state): State<ApiState>) -> impl IntoResponse {
    let r = state.app.readiness().await;
    let ready = r.is_ready();

    let database = match r.database {
        app::health::Dependency::Ok { latency_ms } => DependencyView::Ok { latency_ms },
        app::health::Dependency::Failed { reason } => DependencyView::Failed { reason },
    };

    let body = Json(Ready { ready, database });
    if ready {
        (StatusCode::OK, body).into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, body).into_response()
    }
}
