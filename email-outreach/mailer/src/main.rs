use std::net::SocketAddr;
use std::sync::Arc;

use tracing::info;

use outreach_mailer::{build_router, AppState, Config};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "outreach_mailer=info".into()),
        )
        .init();

    let config = Config::from_env()?;
    info!("starting outreach-mailer");
    info!("  bind:           {}", config.bind_addr);
    info!("  gateway:        {}", config.maat_gateway_url);
    info!("  allowed domains: {:?}", config.allowed_domains);

    info!("fetching executor key from gateway…");
    let executor_key =
        maat_resource::fetch_executor_key(&config.maat_gateway_url, &config.maat_api_key).await?;
    info!("executor key cached (key_id={})", executor_key.key_id);

    let state = Arc::new(AppState::new(config, executor_key).await?);
    let bind_addr: SocketAddr = state.config.bind_addr.parse()?;
    let app = build_router(state.clone());
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    info!("listening on http://{}", bind_addr);
    axum::serve(listener, app).await?;
    Ok(())
}
