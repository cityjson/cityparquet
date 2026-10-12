//! The in-process variant builder and the prepare script stamp a package with
//! the same STAC `datetime`, or the two would never be the same bytes.

#[test]
fn prepare_script_uses_the_crate_corpus_datetime() {
    let script = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/readbench_prepare.sh"),
    )
    .unwrap();
    let expected = format!(
        "CORPUS_DATETIME=\"{}\"",
        cityparquet_readbench::CORPUS_DATETIME
    );
    assert!(
        script.lines().any(|l| l == expected),
        "missing `{expected}`"
    );
}
