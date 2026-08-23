pub mod core;

#[cfg(feature = "server")]
pub mod app;

#[cfg(test)]
mod tests;

// Re-export key types for library consumers
pub use core::db::service::DuckLakeService;
pub use core::interface::repository::{CityLakeRepository, RepositoryResult};
pub use core::interface::types::{CityJsonMetadata, CityLakeConfig, CompactionStats, ExportFormat};
