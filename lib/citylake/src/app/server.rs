//! The axum router: every route the API exposes, wired to its handler, and
//! the server that binds and serves it.

use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::core::interface::repository::CityLakeRepository;
use crate::core::interface::types::CityLakeConfig;

use super::handlers::{dataset, maintenance, objects, package};

/// Upper bound on a multipart upload. Axum's default body limit (2 MiB) is
/// blown instantly by a realistic CityJSON source; this is generous enough
/// for a municipal dataset without leaving the limit unbounded.
const UPLOAD_BODY_LIMIT: usize = 256 * 1024 * 1024;

/// Build the router. The API has no authentication — CORS is permissive and
/// every route is open to whoever can reach the port.
pub fn router(repo: Arc<dyn CityLakeRepository>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/datasets", get(dataset::list))
        .route(
            "/datasets/{ds}",
            post(dataset::create)
                .get(dataset::describe)
                .delete(dataset::drop_dataset),
        )
        .route(
            "/datasets/{ds}/upload",
            post(dataset::create_upload).layer(DefaultBodyLimit::max(UPLOAD_BODY_LIMIT)),
        )
        .route(
            "/datasets/{ds}/objects",
            post(objects::ingest).delete(objects::delete_where),
        )
        .route(
            "/datasets/{ds}/objects/upload",
            post(objects::ingest_upload).layer(DefaultBodyLimit::max(UPLOAD_BODY_LIMIT)),
        )
        .route(
            "/datasets/{ds}/modules/{module}/objects",
            get(objects::query),
        )
        .route(
            "/datasets/{ds}/objects/{id}",
            put(objects::update).delete(objects::delete),
        )
        .route("/datasets/{ds}/export", post(package::export))
        .route("/datasets/{ds}/package", post(package::write_package))
        .route("/datasets/{ds}/merge", post(package::merge))
        .route("/datasets/{ds}/validate", post(maintenance::validate))
        .route("/datasets/{ds}/reconcile", post(maintenance::reconcile))
        .route("/datasets/{ds}/vacuum", post(maintenance::vacuum))
        .route("/datasets/{ds}/compact", post(maintenance::compact))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(repo)
}

async fn health() -> StatusCode {
    StatusCode::OK
}

/// Build the router and serve it on `config.host:config.port`.
pub async fn serve(
    config: CityLakeConfig,
    repo: Arc<dyn CityLakeRepository>,
) -> anyhow::Result<()> {
    let app = router(repo);
    let addr = format!("{}:{}", config.host, config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(%addr, "CityLake listening");
    axum::serve(listener, app).await?;
    Ok(())
}
