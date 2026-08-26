//! The INPUT-EXTENSION CONVENTION: how a benchmark input's file name maps
//! onto the dataset name every artefact of it is then named after.
//!
//! A read-benchmark input is `<dataset><ext>`, and `<dataset>` is what the
//! coordinator resolves its artefacts under (`<dataset>.parquet`,
//! `<dataset>.fcb`, …), what `benchmark/scripts/readbench_prepare.sh` writes them as,
//! and what the justfile's per-dataset recipes name a package directory or
//! a results CSV after. One rule, four implementations — this Rust one, the
//! prepare script's, the justfile's (used by four recipes), and the
//! package-name counterpart in `benchmark/scripts/readbench_duckdb.sh` — because a
//! shell script cannot import a Rust function and `just` has no functions.
//!
//! `benchmark/readbench/tests/strip_extension.rs` extracts each of
//! the shell ones from its own source file and RUNS it over the same table
//! this one is run over, so the lockstep is enforced rather than merely
//! asserted in a comment. It lives in the library, not in the binary's
//! `coordinator` module, so that test can reach it at all.

/// The input extensions the read benchmark recognises, MOST SPECIFIC FIRST
/// (so `.city.jsonl` wins over `.jsonl`, which would otherwise leave a
/// stray `.city` behind).
///
/// The CityGML spellings are not decoration: CityGML is a measured format
/// (`Format::CityGml`), so a `.gml` file is a first-class benchmark input.
/// While this list knew only the two CityJSON pairs, a `.gml` input came
/// back unstripped and every path derived from it — `foo.gml.parquet`,
/// `foo.gml.csv` — was wrong.
pub const KNOWN_INPUT_EXTENSIONS: [&str; 7] = [
    ".city.jsonl",
    ".city.json",
    ".citygml",
    ".jsonl",
    ".json",
    ".gml",
    ".xml",
];

/// `name` with its [`KNOWN_INPUT_EXTENSIONS`] suffix removed (the longest,
/// most specific one wins); returned unchanged when it carries none, so an
/// unrecognised file name is never half-stripped.
pub fn strip_known_extension(name: &str) -> &str {
    for ext in KNOWN_INPUT_EXTENSIONS {
        if let Some(stripped) = name.strip_suffix(ext) {
            return stripped;
        }
    }
    name
}
