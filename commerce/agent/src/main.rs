//! commerce-agent binary.

use std::net::SocketAddr;
use std::sync::Arc;

use tracing::info;

use commerce_agent::{build_router, AgentStore, Config, Runtime};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "commerce_agent=info".into()),
        )
        .init();

    let config = Config::from_env()?;
    let api_key = std::env::var("MAAT_API_KEY")
        .map_err(|_| anyhow::anyhow!("MAAT_API_KEY is required"))?;
    let commerce_customer_id = std::env::var("COMMERCE_AGENT_CUSTOMER_ID")
        .map_err(|_| anyhow::anyhow!("COMMERCE_AGENT_CUSTOMER_ID is required"))?
        .parse()
        .map_err(|e| anyhow::anyhow!("COMMERCE_AGENT_CUSTOMER_ID is not a valid UUID: {}", e))?;

    info!("Starting commerce-agent");
    info!("  Bind:     {}", config.bind_addr);
    info!("  Gateway:  {}", config.gateway_url);
    info!("  Store:    {}", config.store_url);
    info!("  State DB: {}", config.state_db);

    let agent = AgentStore::open(&config.state_db).await?;
    info!(
        "Agent identity ready (pubkey {} bytes)",
        agent.public_key().key_data.len()
    );

    let rt = Arc::new(Runtime {
        config: config.clone(),
        agent,
        api_key,
        commerce_customer_id,
    });

    let app = build_router(rt.clone());
    let addr: SocketAddr = config.bind_addr.parse()?;
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("commerce-agent listening on http://{}", addr);

    axum::serve(listener, app).await?;
    Ok(())
}
