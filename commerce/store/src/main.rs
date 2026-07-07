//! commerce-store daemon.

use std::net::SocketAddr;
use std::sync::Arc;

use tracing::info;

use commerce_store::{build_router, AppState, Config};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "commerce_store=info".into()),
        )
        .init();

    let config = Config::from_env()?;
    info!("Starting commerce-store");
    info!("  Bind:           {}", config.bind_addr);
    info!("  Postgres:       {}", redact_url(&config.database_url));
    info!("  Maat gateway:   {}", config.maat_gateway_url);
    info!("  Stripe key:     {}…", &config.stripe_secret_key[..8.min(config.stripe_secret_key.len())]);

    info!("Fetching gateway executor key…");
    let executor_key =
        maat_resource::fetch_executor_key(&config.maat_gateway_url, &config.maat_api_key).await?;
    info!(
        "Executor key cached (key_id={}, {} bytes)",
        executor_key.key_id,
        executor_key.key.key_data.len()
    );

    let state = Arc::new(AppState::new(config, executor_key).await?);
    let app = build_router(state.clone());

    let addr: SocketAddr = state.config.bind_addr.parse()?;
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("commerce-store listening on http://{}", addr);

    axum::serve(listener, app).await?;
    Ok(())
}

fn redact_url(url: &str) -> String {
    match url.split_once("://") {
        Some((scheme, rest)) => match rest.split_once('@') {
            Some((_, host)) => format!("{}://<redacted>@{}", scheme, host),
            None => url.to_string(),
        },
        None => url.to_string(),
    }
}
