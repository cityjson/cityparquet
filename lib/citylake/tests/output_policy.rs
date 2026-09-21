mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};

#[tokio::test]
async fn writing_a_package_outside_the_root_is_refused() {
    // The API's whole write surface, in one assertion: a caller naming a path
    // the operator did not sanction gets 400, not a written directory.
    let (app, _dir) = common::app_with_output_root();
    let (status, _) = common::send(
        &app,
        Request::post("/datasets/any/package")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"output_dir":"/tmp/escaped"}"#))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn writing_a_package_without_a_configured_root_is_refused() {
    let (app, _dir) = common::app_without_output_root();
    let (status, body) = common::send(
        &app,
        Request::post("/datasets/any/package")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"output_dir":"pkg"}"#))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // The operator meets a sentence naming what to set, not a mystery.
    assert!(
        format!("{body}").contains("CITYLAKE_OUTPUT_ROOT"),
        "the refusal must name the variable: {body}"
    );
}

#[tokio::test]
async fn exporting_outside_the_root_is_refused() {
    let (app, _dir) = common::app_with_output_root();
    let (status, _) = common::send(
        &app,
        Request::post("/datasets/any/export")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"module":"building","output_path":"../escaped.city.jsonl","format":"cityjsonseq"}"#,
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn writing_a_package_inside_the_root_lands_at_the_resolved_path() {
    // Every other test here checks a refusal, which every one of them would
    // pass even if the handler resolved the path correctly and then wrote
    // to the caller's raw string, or to the root, instead of the resolved
    // path it just approved. Only a request that is allowed to succeed, and
    // then checking the file landed exactly where `resolve_output_path`
    // said it would, catches that.
    let (app, dir) = common::app_with_output_root();
    let source = common::fixture("delft.city.jsonl");
    let body = serde_json::json!({ "source_path": source.to_str().unwrap() }).to_string();
    let (status, _) = common::send(
        &app,
        Request::post("/datasets/delft")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = common::send(
        &app,
        Request::post("/datasets/delft/package")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"output_dir":"pkg"}"#))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert!(
        dir.path()
            .join("output-root")
            .join("pkg")
            .join("metadata.json")
            .exists(),
        "the package must land under the configured root at the resolved path, not somewhere else"
    );
}

#[tokio::test]
async fn exporting_inside_the_root_lands_at_the_resolved_path() {
    // The export sibling of the package test above, and needed for the same
    // reason: every other export case here is a refusal, and a refusal is
    // just as green when the handler resolves the path correctly and then
    // hands the repository `body.output_path` instead of what it resolved.
    //
    // The requested path is one directory deep, and that directory is
    // created inside the root and nowhere else. So the raw string names
    // nothing the server process can write relative to its own working
    // directory: substituting it fails the export outright rather than
    // quietly writing somewhere off to the side.
    let (app, dir) = common::app_with_output_root();
    let source = common::fixture("delft.city.jsonl");
    let body = serde_json::json!({ "source_path": source.to_str().unwrap() }).to_string();
    let (status, _) = common::send(
        &app,
        Request::post("/datasets/delft")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let exports = dir.path().join("output-root").join("exports");
    std::fs::create_dir_all(&exports).expect("create the export directory");

    let (status, _) = common::send(
        &app,
        Request::post("/datasets/delft/export")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"module":"building","output_path":"exports/delft-building.city.jsonl","format":"cityjsonseq"}"#,
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let written = exports.join("delft-building.city.jsonl");
    assert!(
        written.exists(),
        "the export must land under the configured root at the resolved path, not somewhere else"
    );
    assert!(
        std::fs::metadata(&written).unwrap().len() > 0,
        "the exported file must carry the module, not be an empty placeholder"
    );
}

#[tokio::test]
async fn exporting_to_the_root_itself_is_refused() {
    // `resolve_output_path` treats a requested path of "" or "." as the root
    // itself — coherent for `package`, which is about to create a directory
    // there, but not for `export`, which writes a single file: without this
    // guard the request would sail past the path check and only fail later,
    // deep inside the extension, on a dataset that (in this test) does not
    // even exist — so a wrong implementation here surfaces as 404, not 400.
    let (app, _dir) = common::app_with_output_root();
    let (status, _) = common::send(
        &app,
        Request::post("/datasets/any/export")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"module":"building","output_path":".","format":"cityjsonseq"}"#,
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn exporting_to_the_root_itself_via_an_empty_path_is_refused() {
    // Same guard as above, exercised through the other string
    // `resolve_output_path` treats as "the root itself" — a regression that
    // narrowed the handler's check to a literal `"."` match would let this
    // one back through while the "." test above kept passing.
    let (app, _dir) = common::app_with_output_root();
    let (status, _) = common::send(
        &app,
        Request::post("/datasets/any/export")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"module":"building","output_path":"","format":"cityjsonseq"}"#,
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
