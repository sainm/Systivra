//! HTTP 契約。axum のルーティングとミドルウェアだけを置く。
//!
//! 境界（`02-アーキテクチャ` §3）：`api` は **sqlx / neo4j driver を直接使わない**。
//! `Cargo.toml` に `store` を入れていないのはそのため。DB に触るのは `app` の仕事。

#![forbid(unsafe_code)]

pub mod error;
pub mod routes;
pub mod trace;

use std::sync::Arc;

use app::App;
use axum::Router;

/// ハンドラが共有する状態。`App` を `Arc` で包む（Clone を軽くする）。
#[derive(Clone)]
pub struct ApiState {
    pub app: Arc<App>,
}

impl ApiState {
    pub fn new(app: App) -> Self {
        Self { app: Arc::new(app) }
    }
}

/// ルータを組み立てる。ミドルウェアの順序はここで固定する。
///
/// `06-API.md` §4 の順序（trace_id → 認証 → パス照合 → SET LOCAL → 権限）のうち、
/// Phase 1 では **trace_id だけ**を実装する。残りは `P1-API-01`。
pub fn router(app: App) -> Router {
    let state = ApiState::new(app);

    Router::new()
        .merge(routes::health::router())
        // 全ての応答に `X-Trace-Id` を付ける。Problem の `trace_id` と同じ値。
        .layer(axum::middleware::from_fn(trace::with_trace_id))
        .with_state(state)
}
