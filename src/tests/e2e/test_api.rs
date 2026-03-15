use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::{get, post, put};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

use crate::app::handlers;
use crate::core::interface::repository::CityLakeRepository;
use crate::tests::helpers;

/// Build a test router without middleware (avoids potential issues with tracing).
fn build_test_router(repo: Arc<dyn CityLakeRepository>) -> Router {
    Router::new()
        .route("/tables/:table_name", post(handlers::table::create_table))
        .route(
            "/tables/:table_name/objects",
            post(handlers::insert::insert_objects).get(handlers::query::query_objects),
        )
        .route(
            "/tables/:table_name/objects/:id",
            put(handlers::update::update_object).delete(handlers::delete::delete_object),
        )
        .route(
            "/tables/:table_name/compact",
            post(handlers::compaction::compact_table),
        )
        .route(
            "/tables/:table_name/export",
            post(handlers::export::export_table),
        )
        .route("/health", get(|| async { "ok" }))
        .with_state(repo)
}

/// Helper to parse JSON from response body.
async fn body_json(body: Body) -> Value {
    let bytes = body.collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn test_health_check() {
    let service = helpers::setup();
    let repo: Arc<dyn CityLakeRepository> = Arc::new(service);
    let app = build_test_router(repo);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&bytes[..], b"ok");
}

#[tokio::test]
async fn test_query_objects_via_api() {
    let service = helpers::setup_with_table("query_api");
    let repo: Arc<dyn CityLakeRepository> = Arc::new(service);

    let app = build_test_router(repo);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/tables/query_api/objects")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response.into_body()).await;
    assert_eq!(body["count"], 3);
    assert_eq!(body["table"], "query_api");
    assert!(body["objects"].is_array());
}

#[tokio::test]
async fn test_query_with_limit_via_api() {
    let service = helpers::setup_with_table("limit_api");
    let repo: Arc<dyn CityLakeRepository> = Arc::new(service);
    let app = build_test_router(repo);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/tables/limit_api/objects?limit=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response.into_body()).await;
    assert_eq!(body["count"], 1);
}

#[tokio::test]
async fn test_query_with_filter_via_api() {
    let service = helpers::setup_with_table("filter_api");
    let repo: Arc<dyn CityLakeRepository> = Arc::new(service);
    let app = build_test_router(repo);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/tables/filter_api/objects?filter=id%20%3D%20%27building_001%27")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response.into_body()).await;
    assert_eq!(body["count"], 1);
}

#[tokio::test]
async fn test_delete_object_via_api() {
    let service = helpers::setup_with_table("delete_api");
    let repo: Arc<dyn CityLakeRepository> = Arc::new(service);
    let app = build_test_router(repo);

    // First verify query works
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/tables/delete_api/objects")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let query_body = body_json(response.into_body()).await;
    assert_eq!(query_body["count"], 3, "Should have 3 objects before delete");

    // Delete existing object
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/tables/delete_api/objects/building_001")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Verify count decreased
    let response = app
        .oneshot(
            Request::builder()
                .uri("/tables/delete_api/objects")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(response.into_body()).await;
    assert_eq!(body["count"], 2);
}

#[tokio::test]
async fn test_delete_nonexistent_returns_404() {
    let service = helpers::setup_with_table("del_404");
    let repo: Arc<dyn CityLakeRepository> = Arc::new(service);
    let app = build_test_router(repo);

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/tables/del_404/objects/nonexistent_id")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_update_nonexistent_returns_404() {
    let service = helpers::setup_with_table("upd_404");
    let repo: Arc<dyn CityLakeRepository> = Arc::new(service);
    let app = build_test_router(repo);

    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/tables/upd_404/objects/nonexistent_id")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"cityjson_data": "{}"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_compact_via_api() {
    let service = helpers::setup_with_table("compact_api");
    let repo: Arc<dyn CityLakeRepository> = Arc::new(service);
    let app = build_test_router(repo);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/tables/compact_api/compact")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response.into_body()).await;
    assert!(body["stats"]["rows_compacted"].is_number());
    assert_eq!(body["stats"]["rows_compacted"], 3);
}

#[tokio::test]
async fn test_crud_workflow() {
    let service = helpers::setup_with_table("crud_test");
    let repo: Arc<dyn CityLakeRepository> = Arc::new(service);
    let app = build_test_router(repo);

    // 1. Query all — should have 3
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/tables/crud_test/objects")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(response.into_body()).await;
    assert_eq!(body["count"], 3);

    // 2. Delete one
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/tables/crud_test/objects/building_002")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // 3. Query again — should have 2
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/tables/crud_test/objects")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(response.into_body()).await;
    assert_eq!(body["count"], 2);

    // 4. Try deleting the same one again — should 404
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/tables/crud_test/objects/building_002")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // 5. Query with limit
    let response = app
        .oneshot(
            Request::builder()
                .uri("/tables/crud_test/objects?limit=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(response.into_body()).await;
    assert_eq!(body["count"], 1);
}
