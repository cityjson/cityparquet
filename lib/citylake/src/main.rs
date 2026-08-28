use std::sync::Arc;

use citylake::core::db::service::DuckLakeService;
use citylake::core::interface::repository::CityLakeRepository;
use citylake::core::interface::types::CityLakeConfig;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let config = CityLakeConfig::default();
    let repo: Arc<dyn CityLakeRepository> = Arc::new(DuckLakeService::new(config.clone())?);
    citylake::app::server::serve(config, repo).await
}
