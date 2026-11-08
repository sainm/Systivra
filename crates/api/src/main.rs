//! API プロセスの起動。
//!
//! 起動は DB に依存させない（`connect_lazy`）。DB が落ちていても
//! `/healthz` は 200 を返し、`/readyz` だけが 503 になる。
//! こうしないと、DB の再起動中に API が CrashLoopBackOff に入る。

use std::net::SocketAddr;

use api::{auth::SessionConfig, router_with_oidc};
use app::App;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();

    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        // 開発用の既定値。本番では必ず環境変数で上書きする。
        "postgres://systivra_app@localhost:5432/systivra".to_owned()
    });

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);

    let app = App::from_database_url(&database_url)?;

    // 署名鍵が無いまま起動しない。空鍵で動かすと Cookie を誰でも偽造できる。
    let session = SessionConfig::from_env()?;

    // 一部だけ設定された OIDC 設定は起動時に失敗させる（SSO 有効のつもりで
    // 無効のまま動く事故を防ぐ）。未設定ならパスワードログインのみ。
    let oidc = providers::OidcConfig::from_env()?
        .map(|config| std::sync::Arc::new(providers::OidcRuntime::new(config)));

    let http = router_with_oidc(app, session, oidc);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;

    tracing::info!(%addr, "systivra-api listening");

    axum::serve(
        listener,
        http.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    Ok(())
}

fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,systivra_api=debug"));
    fmt().with_env_filter(filter).with_target(true).init();
}

/// SIGTERM / Ctrl-C で受付を止める。処理中のリクエストは完了させる。
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("ctrl-c received"),
        _ = terminate => tracing::info!("sigterm received"),
    }
}
