use std::time::Duration;

use app::App;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://systivra_scheduler@localhost:5432/systivra".to_owned());
    let app = App::from_database_url(&database_url)?;

    let interval = Duration::from_secs(
        std::env::var("WORKER_TICK_SEC")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2),
    );

    tracing::info!("systivra-worker starting");
    let mut ticker = tokio::time::interval(interval);
    let mut last_session_cleanup = std::time::Instant::now();
    let mut last_artifact_cleanup = std::time::Instant::now()
        .checked_sub(Duration::from_secs(86_400))
        .unwrap_or_else(std::time::Instant::now);
    let mut last_drift_check = std::time::Instant::now()
        .checked_sub(Duration::from_secs(86_400))
        .unwrap_or_else(std::time::Instant::now);
    let neo4j_disabled = std::env::var("NEO4J_DISABLED").ok().as_deref() == Some("1");
    let retention_days = std::env::var("ARTIFACT_RETENTION_DAYS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(90);
    let trace_retention_days = std::env::var("TRACE_RETENTION_DAYS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if last_session_cleanup.elapsed() >= Duration::from_secs(3600) {
                    match app.cleanup_sessions(7).await {
                        Ok(deleted) => tracing::info!(deleted, "expired sessions cleaned"),
                        Err(error) => tracing::warn!(error = %error, "session cleanup failed"),
                    }
                    last_session_cleanup = std::time::Instant::now();
                }
                if last_artifact_cleanup.elapsed() >= Duration::from_secs(86_400) {
                    match app.expire_test_artifacts(None, retention_days, trace_retention_days).await {
                        Ok(report) => tracing::info!(
                            scanned = report.scanned,
                            expired = report.expired,
                            deleted_files = report.deleted_files,
                            bytes_reclaimed = report.bytes_reclaimed,
                            "test artifacts retention applied"
                        ),
                        Err(error) => tracing::warn!(error = %error, "artifact retention failed"),
                    }
                    last_artifact_cleanup = std::time::Instant::now();
                }
                if !neo4j_disabled && last_drift_check.elapsed() >= Duration::from_secs(86_400) {
                    match app.store().begin_unscoped().await {
                        Ok(mut tx) => {
                            let projects = app.store().list_project_ids(&mut tx).await;
                            if let Err(error) = app::commit_tx(tx).await {
                                tracing::warn!(%error, "drift list commit failed");
                            }
                            if let Ok(projects) = projects {
                                for id in projects {
                                    match app.detect_graph_drift(domain::ProjectId(id)).await {
                                        Ok(view) if view.degraded => tracing::warn!(
                                            %id,
                                            drift_rate = view.drift_rate,
                                            pg_nodes = view.pg_nodes,
                                            neo4j_nodes = view.neo4j_nodes,
                                            "graph drift detected"
                                        ),
                                        Ok(view) => tracing::info!(
                                            %id,
                                            drift_rate = view.drift_rate,
                                            "graph drift check ok"
                                        ),
                                        Err(error) => tracing::warn!(
                                            %id,
                                            %error,
                                            "graph drift check failed"
                                        ),
                                    }
                                }
                            } else if let Err(error) = projects {
                                tracing::warn!(%error, "drift project list failed");
                            }
                        }
                        Err(error) => tracing::warn!(%error, "drift check tx failed"),
                    }
                    last_drift_check = std::time::Instant::now();
                }
                if let Err(error) = worker::run(&app).await {
                    tracing::error!(error = %error, "worker tick failed");
                }
            }
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
