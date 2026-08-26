use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post, put};
use axum::Router;
use std::sync::Arc;

use crate::core::interface::repository::CityLakeRepository;
use crate::core::interface::types::CityLakeConfig;

use super::handlers;
use super::middleware;

/// Upper bound on multipart upload size. Axum's default is 2 MiB which is
/// instantly blown by realistic CityJSON files; 256 MiB is generous enough for
/// typical municipal datasets without inviting accidental DoS.
const UPLOAD_BODY_LIMIT: usize = 256 * 1024 * 1024;

/// Build the axum router with all routes and middleware.
pub fn build_router(repo: Arc<dyn CityLakeRepository>) -> Router {
    Router::new()
        // Catalog
        .route("/tables", get(handlers::list::list_tables))
        // Table operations
        .route("/tables/{table_name}", post(handlers::table::create_table))
        .route(
            "/tables/{table_name}/upload",
            post(handlers::table::create_table_upload)
                .layer(DefaultBodyLimit::max(UPLOAD_BODY_LIMIT)),
        )
        // Object CRUD
        .route(
            "/tables/{table_name}/objects",
            post(handlers::insert::insert_objects).get(handlers::query::query_objects),
        )
        .route(
            "/tables/{table_name}/objects/upload",
            post(handlers::insert::insert_objects_upload)
                .layer(DefaultBodyLimit::max(UPLOAD_BODY_LIMIT)),
        )
        .route(
            "/tables/{table_name}/objects/{id}",
            put(handlers::update::update_object).delete(handlers::delete::delete_object),
        )
        // Compaction
        .route(
            "/tables/{table_name}/compact",
            post(handlers::compaction::compact_table),
        )
        // Export
        .route(
            "/tables/{table_name}/export",
            post(handlers::export::export_table),
        )
        // Health check
        .route("/health", get(health_check))
        // Middleware
        .layer(middleware::trace_layer())
        .layer(middleware::cors_layer())
        // State
        .with_state(repo)
}

/// Start the HTTP server.
pub async fn start_server(
    repo: Arc<dyn CityLakeRepository>,
    config: &CityLakeConfig,
) -> anyhow::Result<()> {
    let app = build_router(repo);
    let addr = format!("{}:{}", config.host, config.port);

    tracing::info!("Starting CityLake server on {addr}");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn health_check() -> &'static str {
    "ok"
}
