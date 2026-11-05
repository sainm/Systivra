//! worker プロセスの起動。Phase 1 では骨格のみ。
//!
//! API とは別プロセス・別 DB ロールで動かす（`detail/02` §0.1）。
//! 同じロールにすると、API がジョブ行を書き換えられてしまう。

use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();
    tracing::info!("systivra-worker starting (skeleton)");

    let interval = Duration::from_secs(
        std::env::var("WORKER_TICK_SEC")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5),
    );

    let mut ticker = tokio::time::interval(interval);
    loop {
        tokio::select! {
            _ = ticker.tick() => worker::tick().await,
            _ = shutdown_signal() => {
                tracing::info!("worker shutting down");
                break;
            }
        }
    }

    Ok(())
}

fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,systivra_worker=debug"));
    fmt().with_env_filter(filter).with_target(true).init();
}

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
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
