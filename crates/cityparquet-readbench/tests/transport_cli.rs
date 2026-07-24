//! `--transport`/`--base-url` flag parsing on the `--child` path: this test
//! only proves the FLAGS are accepted and threaded to a clear "not
//! implemented yet" error for a format with no HTTP branch — the real HTTP
//! behaviour is proven per-format in later tests (Tasks 11-13).

use std::process::Command;

#[test]
fn child_rejects_http_transport_for_a_format_with_no_http_branch_yet_with_a_clear_message() {
    let output = Command::new(env!("CARGO_BIN_EXE_cityparquet-readbench"))
        .args([
            "--child",
            "--format",
            "cityjsonseq",
            "--scenario",
            "count",
            "--transport",
            "http",
            "--base-url",
            "http://127.0.0.1:1/unused",
            "--input",
            "unused.city.jsonl",
        ])
        .output()
        .expect("failed to run the built binary");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("HTTP transport not implemented"),
        "expected a clear not-implemented message, got:\n{stderr}"
    );
}
