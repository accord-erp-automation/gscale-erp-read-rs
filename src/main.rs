use std::{sync::Arc, time::Duration};

use anyhow::Context;
use gscale_erp_read_rs::{appconfig, httpapi, store::Store};
use sqlx::{MySqlPool, mysql::MySqlPoolOptions};
use tokio::{net::TcpListener, signal};
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .init();

    let cfg = appconfig::load_from_env().context("config load failed")?;
    let pool = connect_pool(&cfg).await.context("db connect failed")?;

    let app = httpapi::router(Arc::new(Store::new(pool)));
    let listener = TcpListener::bind(&cfg.addr)
        .await
        .with_context(|| format!("bind {}", cfg.addr))?;

    info!(
        "gscale-erp-read-rs listening on {} for site {}",
        cfg.addr, cfg.site_name
    );

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("http server failed")
}

async fn connect_pool(cfg: &appconfig::Config) -> sqlx::Result<MySqlPool> {
    MySqlPoolOptions::new()
        .max_connections(10)
        .min_connections(0)
        .max_lifetime(Some(Duration::from_secs(5 * 60)))
        .connect_with(cfg.connect_options())
        .await
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c().await.expect("install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
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
