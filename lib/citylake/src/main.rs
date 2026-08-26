use std::sync::Arc;

use citylake::app::server;
use citylake::core::interface::types::CityLakeConfig;
use citylake::DuckLakeService;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    // Load config (from env/defaults for now)
    let config = CityLakeConfig::default();

    // Initialize the DuckLake service
    let service = DuckLakeService::new(config.clone())
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let repo: Arc<dyn citylake::CityLakeRepository> = Arc::new(service);

    // Start the HTTP server
    server::start_server(repo, &config).await?;

    Ok(())
}
