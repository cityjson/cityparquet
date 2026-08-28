mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

fn app() -> (axum::Router, tempfile::TempDir) {
    let (service, dir) = common::test_service();
    (
        citylake::app::server::router(std::sync::Arc::new(service)),
        dir,
    )
}

async fn send(app: &axum::Router, request: Request<Body>) -> (StatusCode, serde_json::Value) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

#[tokio::test]
async fn health_reports_ok() {
    let (app, _dir) = app();
    let (status, _) = send(&app, Request::get("/health").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn a_dataset_is_created_described_and_dropped_over_http() {
    let (app, _dir) = app();
    let source = common::fixture("delft.city.jsonl");
    let body = serde_json::json!({ "source_path": source.to_str().unwrap() }).to_string();

    let (status, _) = send(
        &app,
        Request::post("/datasets/delft")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, described) = send(
        &app,
        Request::get("/datasets/delft").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(described["name"], "delft");

    let (status, _) = send(
        &app,
        Request::delete("/datasets/delft")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn an_invalid_dataset_name_is_rejected_before_it_reaches_sql() {
    let (app, _dir) = app();
    let (status, _) = send(
        &app,
        Request::get("/datasets/not%20a%20name")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn an_absent_dataset_is_a_404_not_a_500() {
    let (app, _dir) = app();
    let (status, _) = send(
        &app,
        Request::get("/datasets/absent")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn creating_the_same_dataset_twice_is_a_conflict() {
    let (app, _dir) = app();
    let source = common::fixture("delft.city.jsonl");
    let body = serde_json::json!({ "source_path": source.to_str().unwrap() }).to_string();
    let request = || {
        Request::post("/datasets/delft")
            .header("content-type", "application/json")
            .body(Body::from(body.clone()))
            .unwrap()
    };

    let (first, _) = send(&app, request()).await;
    assert_eq!(first, StatusCode::CREATED);
    let (second, _) = send(&app, request()).await;
    assert_eq!(second, StatusCode::CONFLICT);
}

#[tokio::test]
async fn objects_are_queryable_by_module() {
    let (app, _dir) = app();
    let source = common::fixture("delft.city.jsonl");
    send(
        &app,
        Request::post("/datasets/delft")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "source_path": source.to_str().unwrap() }).to_string(),
            ))
            .unwrap(),
    )
    .await;

    let (status, rows) = send(
        &app,
        Request::get("/datasets/delft/modules/building/objects?limit=2")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rows.as_array().unwrap().len(), 2);
}
