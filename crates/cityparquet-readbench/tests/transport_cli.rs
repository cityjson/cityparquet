//! The `--child` path's own flag-parsing layer: a bad `--transport` or
//! `--format` value must fail clearly rather than silently defaulting or
//! panicking.
//!
//! Originally this test proved `--transport`/`--base-url` were plumbed
//! through to a clear "not implemented yet" error for a format with no HTTP
//! branch (Task 10, before any real HTTP runner existed). Now that every
//! format (`cityparquet`, `flatcitybuf`, `cityjsonseq`) implements
//! `--transport http` (Tasks 11-13, each proven by its own
//! `tests/*_http_runner.rs`), that premise no longer holds — so these tests
//! instead check the parsing layer itself.

use std::process::Command;

use cityparquet_readbench::format::Format;

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

/// An invalid `--format` is rejected by CLAP, at parse time, and the error
/// enumerates every valid name from `Format::ALL` itself.
///
/// This locks in a DELIBERATE change of failure mode. Before `--format`
/// became a `Format`, an unknown name travelled all the way into
/// `formats::resolve`, which raised an `anyhow` error carrying a
/// hand-maintained list of names — reported as a run failure (exit 1).
/// Now `Format`'s own `FromStr` rejects it during argument parsing, so it is
/// a clap USAGE error (exit 2) and the list can no longer drift. The exit
/// code is asserted because this harness is driven from shell scripts, where
/// "you invoked me wrongly" and "the measurement failed" are worth telling
/// apart — and because asserting only the message would not have caught the
/// change at all.
#[test]
fn child_rejects_an_unknown_format_at_parse_time_and_names_the_valid_ones() {
    let output = Command::new(env!("CARGO_BIN_EXE_cityparquet-readbench"))
        .args([
            "--child",
            "--format",
            "carrier-pigeon",
            "--scenario",
            "count",
            "--input",
            "unused.city.jsonl",
        ])
        .output()
        .expect("failed to run the built binary");

    assert_eq!(
        output.status.code(),
        Some(2),
        "clap reports a usage error as exit 2; a resolve-time failure would be exit 1"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("carrier-pigeon"),
        "the error must quote the rejected value, got:\n{stderr}"
    );
    for format in Format::ALL {
        assert!(
            stderr.contains(format.as_str()),
            "the error must list every valid format; '{format}' is missing from:\n{stderr}"
        );
    }
}
