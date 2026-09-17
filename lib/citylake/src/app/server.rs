//! The axum router: every route the API exposes, wired to its handler, and
//! the server that binds and serves it.

use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, FromRef};
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

/// Router state: the repository every handler calls through, plus the root
/// the two write endpoints (`package::export`, `package::write_package`)
/// confine an API-supplied output path to.
///
/// `dataset`, `maintenance` and `objects` extract `State<Arc<dyn
/// CityLakeRepository>>` directly, unaware this struct exists at all — the
/// `FromRef` impl below is what makes that keep compiling: axum derives
/// their narrower state from this one. Only `package`'s `export` and
/// `write_package` extract `State<AppState>`, because they are the only
/// handlers that need `output_root`.
#[derive(Clone)]
pub struct AppState {
    pub(crate) repo: Arc<dyn CityLakeRepository>,
    pub(crate) output_root: Option<String>,
}

impl FromRef<AppState> for Arc<dyn CityLakeRepository> {
    fn from_ref(state: &AppState) -> Self {
        state.repo.clone()
    }
}

/// Build the router. The API has no authentication — CORS is permissive and
/// every route is open to whoever can reach the port. `output_root` is
/// `CITYLAKE_OUTPUT_ROOT`; when `None`, `export` and `write_package` refuse
/// every request rather than writing wherever the caller names.
pub fn router(repo: Arc<dyn CityLakeRepository>, output_root: Option<String>) -> Router {
    let state = AppState { repo, output_root };
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
        .with_state(state)
}

async fn health() -> StatusCode {
    StatusCode::OK
}

/// Build the router and serve it on `config.host:config.port`.
pub async fn serve(
    config: CityLakeConfig,
    repo: Arc<dyn CityLakeRepository>,
) -> anyhow::Result<()> {
    let output_root = config.output_root.clone();
    let app = router(repo, output_root);
    let addr = format!("{}:{}", config.host, config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(%addr, "CityLake listening");
    axum::serve(listener, app).await?;
    Ok(())
}
