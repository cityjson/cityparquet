//! `write-cityjsonseq --input <seq> --output <path>`: the CityJSONSeq WRITER
//! the cross-format write benchmark measures, a streaming parse-and-serialise
//! of a canonical `.city.jsonl` stream.
//!
//! What these tests pin down is that it really is a re-serialisation — the
//! same kind of work `cjseq collect`, `fcb ser` and `cityparquet convert` do
//! for their own formats — and not a copy: every line goes through cjseq's
//! typed model and comes back out semantically unchanged, and a line that
//! does not parse is an error rather than bytes passed through. The row this
//! subcommand produces divides every write ratio in the plots, so "it parsed
//! nothing" is exactly the failure mode worth a test.

use std::path::PathBuf;
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib/cityparquet-rs/tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// JSON equality that ignores how a number was SPELLED. serde_json compares
/// `1` and `1.0` as different `Value::Number`s, but cjseq's typed model holds
/// coordinates and extents as `f64`, so an integer-spelled `0` in the source
/// legitimately comes back as `0.0`. Everything else — keys, strings, nesting,
/// array order — is compared exactly.
fn canonical(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Number(n) => {
            serde_json::json!(n.as_f64().expect("a finite JSON number"))
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(canonical).collect())
        }
        serde_json::Value::Object(map) => {
            serde_json::Value::Object(map.iter().map(|(k, v)| (k.clone(), canonical(v))).collect())
        }
        other => other.clone(),
    }
}

fn write_seq(input: &std::path::Path, output: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cityparquet-readbench"))
        .args([
            "write-cityjsonseq",
            "--input",
            input.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
        ])
        .output()
        .unwrap()
}

#[test]
fn it_reserialises_every_line_of_the_stream_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("delft.out.city.jsonl");
    let input = fixture("delft.city.jsonl");
    let output = write_seq(&input, &out);
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stdout.is_empty(),
        "the writer is silent on success; got {:?}",
        String::from_utf8_lossy(&output.stdout)
    );

    let source = std::fs::read_to_string(&input).unwrap();
    let written = std::fs::read_to_string(&out).unwrap();
    let source_lines: Vec<&str> = source.lines().filter(|l| !l.trim().is_empty()).collect();
    let written_lines: Vec<&str> = written.lines().collect();
    assert_eq!(
        written_lines.len(),
        source_lines.len(),
        "line for line: a header plus one line per feature"
    );
    assert!(
        source_lines.len() > 1000,
        "the delft fixture is a real multi-feature stream, got {} lines",
        source_lines.len()
    );

    // Every FEATURE line — lines 2..N, all but one of the stream — survives
    // exactly: geometry, attributes, vertices, appearance, and any unmodelled
    // key on a CityObject (cjseq's `CityObject` carries `#[serde(flatten)]`).
    for (index, (before, after)) in source_lines.iter().zip(&written_lines).enumerate().skip(1) {
        let before: serde_json::Value = serde_json::from_str(before).unwrap();
        let after: serde_json::Value = serde_json::from_str(after)
            .unwrap_or_else(|e| panic!("line {} of the output is not JSON: {e}", index + 1));
        assert_eq!(
            canonical(&after),
            canonical(&before),
            "line {} changed meaning",
            index + 1
        );
    }

    // Line 1 is the HEADER, and it is the one line cjseq's typed model does
    // not carry losslessly: `Metadata` has no `#[serde(flatten)]` catch-all,
    // so a key it does not name is dropped. `cjseq collect` — the writer
    // behind the benchmark's neighbouring `cityjson` row — parses through the
    // very same struct and drops the very same keys, so the two rows are
    // equally faithful by construction. This test pins the gap to exactly
    // that: unnamed keys inside `metadata`, and nothing else anywhere.
    let before: serde_json::Value = serde_json::from_str(source_lines[0]).unwrap();
    let after: serde_json::Value = serde_json::from_str(written_lines[0]).unwrap();
    assert_eq!(after["type"], "CityJSON");
    let dropped: Vec<&str> = before["metadata"]
        .as_object()
        .unwrap()
        .keys()
        .filter(|key| after["metadata"].get(key.as_str()).is_none())
        .map(String::as_str)
        .collect();
    assert_eq!(
        dropped,
        ["fullMetadataUrl", "version"],
        "the header gap is cjseq's unmodelled `metadata` keys, and only those"
    );
    let mut expected = before.clone();
    for key in &dropped {
        expected["metadata"].as_object_mut().unwrap().remove(*key);
    }
    assert_eq!(
        canonical(&after),
        canonical(&expected),
        "the header is otherwise unchanged"
    );
}

#[test]
fn it_rewrites_rather_than_copies_the_bytes() {
    // The defect this subcommand replaces was `cat`: on a reflink-capable
    // filesystem that is a metadata-only clone, so the output is byte-identical
    // to the input and nothing is parsed. cjseq's typed model does not preserve
    // JSON object key order, so a genuine re-serialisation of the delft fixture
    // differs byte-wise while (per the test above) meaning the same thing.
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("delft.out.city.jsonl");
    let input = fixture("delft.city.jsonl");
    assert!(write_seq(&input, &out).status.success());
    assert_ne!(
        std::fs::read(&input).unwrap(),
        std::fs::read(&out).unwrap(),
        "identical bytes would mean the stream was copied, not re-serialised"
    );
}

#[test]
fn a_malformed_feature_line_is_an_error_naming_the_line() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("broken.city.jsonl");
    std::fs::write(
        &input,
        concat!(
            r#"{"type":"CityJSON","version":"2.0","transform":{"scale":[0.001,0.001,0.001],"translate":[0.0,0.0,0.0]},"CityObjects":{},"vertices":[]}"#,
            "\n",
            r#"{"type":"CityJSONFeature","id":"a","CityObjects":{},"vertices":[]}"#,
            "\n",
            "{ this is not json\n",
        ),
    )
    .unwrap();
    let out = dir.path().join("out.city.jsonl");
    let output = write_seq(&input, &out);
    assert!(
        !output.status.success(),
        "a malformed line must fail, not be passed through"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("line 3"), "{stderr}");
    assert!(stderr.contains("CityJSONFeature"), "{stderr}");
}

#[test]
fn a_malformed_header_line_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("broken.city.jsonl");
    std::fs::write(&input, "{\"type\":\"CityJSONFeature\"}\n").unwrap();
    let out = dir.path().join("out.city.jsonl");
    let output = write_seq(&input, &out);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("header"), "{stderr}");
}
