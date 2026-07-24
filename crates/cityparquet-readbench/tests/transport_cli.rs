//! `--transport`/`--base-url` flag parsing on the `--child` path.
//!
//! Originally this test proved the flags were plumbed through to a clear
//! "not implemented yet" error for a format with no HTTP branch (Task 10,
//! before any real HTTP runner existed). Now that every format
//! (`cityparquet`, `flatcitybuf`, `cityjsonseq`) implements
//! `--transport http` (Tasks 11-13, each proven by its own
//! `tests/*_http_runner.rs`), that premise no longer holds — so this test
//! instead checks the flag-parsing layer itself: an unrecognised
//! `--transport` value must fail clearly, not silently default to `local`
//! or panic.

use std::process::Command;

#[test]
fn child_rejects_an_unknown_transport_value_with_a_clear_message() {
    let output = Command::new(env!("CARGO_BIN_EXE_cityparquet-readbench"))
        .args([
            "--child",
            "--format",
            "cityjsonseq",
            "--scenario",
            "count",
            "--transport",
            "carrier-pigeon",
            "--input",
            "unused.city.jsonl",
        ])
        .output()
        .expect("failed to run the built binary");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown --transport"),
        "expected a clear unknown-transport message, got:\n{stderr}"
    );
}
