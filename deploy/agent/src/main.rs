use std::net::SocketAddr;
use std::sync::Arc;

use tracing::info;

use deploy_agent::{build_router, AgentStore, Config, Runtime};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "deploy_agent=info".into()),
        )
        .init();

    let config = Config::from_env()?;
    info!("starting deploy-agent on {}", config.bind_addr);
    info!("  gateway: {}", config.gateway_url);
    info!("  runner:  {}", config.runner_url);

    let agent = AgentStore::open(&config.agent_db_path).await?;
    let pk_b64 = base64::Engine::encode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        &agent.public_key().key_data,
    );
    info!("agent public key (paste into dashboard): {}", pk_b64);

    let rt = Arc::new(Runtime { config: config.clone(), agent });
    let bind_addr: SocketAddr = config.bind_addr.parse()?;
    let app = build_router(rt.clone());
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    info!("listening on http://{}", bind_addr);
    axum::serve(listener, app).await?;
    Ok(())
}
