//! The INPUT-EXTENSION CONVENTION, enforced across every implementation of
//! it.
//!
//! A benchmark input is named `<dataset><ext>`, and every tool in the read
//! benchmark has to recover the same `<dataset>` from it: the coordinator
//! resolves `--prepared-dir/<dataset>.parquet` and friends, the prepare
//! script writes those very artefacts, and the justfile's per-dataset
//! recipes name a CSV/package directory after it. The rule is therefore
//! implemented four times over — once in Rust
//! ([`strip_known_extension`]), once in `scripts/readbench_prepare.sh`,
//! four identical times in the MONOREPO's root `justfile` (which is where
//! the per-dataset recipes live: they reach both this crate and the corpora
//! under `benchmark/`), and once (as its composable package-name
//! counterpart) in `scripts/readbench_duckdb.sh` — because a shell script
//! cannot import a Rust function and `just` has no functions of its own.
//!
//! Every one of them used to know only `.json`/`.jsonl`. A `.gml` input was
//! therefore invisible to every `find` pattern in the justfile, and a
//! `foo.gml` that reached a stripper anyway came back as `foo.gml`, so
//! every artefact path derived from it (`foo.gml.parquet`, `foo.gml.csv`)
//! was wrong. The Rust side's own doc comment already CLAIMED lockstep with
//! the others; nothing checked it. This file is that check.
//!
//! It is deliberately executable rather than textual: each shell
//! implementation is extracted from its own source file and RUN, over the
//! same [`CASES`] table the Rust one is run over. Comparing extension
//! *lists* alone would pass a copy that listed the right extensions and
//! applied them wrongly.

use std::path::{Path, PathBuf};
use std::process::Command;

use cityparquet_readbench::naming::{KNOWN_INPUT_EXTENSIONS, strip_known_extension};

/// `(input basename, dataset name)` — the shared table every implementation
/// is run over.
///
/// It covers the compound CityJSON extensions (where the longest suffix has
/// to win), the plain ones, all three CityGML/XML spellings, a name with
/// dots of its own, an extension nothing here knows (which must be left
/// alone rather than half-stripped), and a name with no extension at all.
const CASES: &[(&str, &str)] = &[
    ("delft.city.jsonl", "delft"),
    ("lod3_railway.city.json", "lod3_railway"),
    ("9-196-328.city.json", "9-196-328"),
    ("tile.jsonl", "tile"),
    ("tile.json", "tile"),
    ("b1_lod2_s.gml", "b1_lod2_s"),
    ("berlin.citygml", "berlin"),
    ("export.xml", "export"),
    ("dotted.name.city.jsonl", "dotted.name"),
    ("archive.tar.gz", "archive.tar.gz"),
    ("no_extension", "no_extension"),
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("the workspace root is two levels above this crate")
}

/// The MONOREPO root, two levels above this crate's workspace
/// (`lib/cityparquet-rs`).
///
/// The four per-dataset recipes checked below live in the monorepo's own
/// `justfile`, not this workspace's: they reach both the harness code (here)
/// and its corpora and results (`benchmark/`), so neither tree can own them.
/// The shell implementations they are compared against are still local, which
/// is why both roots exist.
fn mono_root() -> PathBuf {
    repo_root()
        .join("..")
        .join("..")
        .canonicalize()
        .expect("the monorepo root is two levels above the cityparquet-rs workspace")
}

fn read(relative: &str) -> String {
    read_from(repo_root(), relative)
}

fn read_mono(relative: &str) -> String {
    read_from(mono_root(), relative)
}

fn read_from(root: PathBuf, relative: &str) -> String {
    let path = root.join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

/// Runs `program` under `bash -euo pipefail` once per case, passing the case
/// input as `$1`, and returns what it printed each time.
///
/// `bash` rather than `sh`: every implementation here is bash (the justfile's
/// recipes carry a `#!/usr/bin/env bash` shebang, both scripts declare one),
/// and `[[ ... ]]`/`${var%"$suffix"}` are bash syntax.
fn run_bash_over_cases(what: &str, program: &str) -> Vec<String> {
    CASES
        .iter()
        .map(|(input, _)| {
            let out = Command::new("bash")
                .arg("-c")
                .arg(program)
                .arg("bash")
                .arg(input)
                .output()
                .unwrap_or_else(|e| panic!("running the {what} implementation under bash: {e}"));
            assert!(
                out.status.success(),
                "the {what} implementation failed on '{input}': {}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8(out.stdout)
                .expect("a dataset name is UTF-8")
                .trim_end_matches('\n')
                .to_string()
        })
        .collect()
}

fn assert_matches_the_table(what: &str, produced: &[String]) {
    for ((input, expected), got) in CASES.iter().zip(produced) {
        assert_eq!(
            got, expected,
            "{what} stripped '{input}' to '{got}', not '{expected}'"
        );
    }
}

/// The single line of `scripts/readbench_prepare.sh` that owns its copy of
/// the extension list, and the single line of the `justfile` that owns its
/// own — extracted rather than restated, so the test cannot pass against a
/// list that has drifted.
fn shell_array_line(source: &str, name: &str) -> String {
    let prefix = format!("{name}=(");
    source
        .lines()
        .find(|l| l.trim_start().starts_with(&prefix))
        .unwrap_or_else(|| panic!("no `{name}=(...)` line found"))
        .to_string()
}

fn just_variable(source: &str, name: &str) -> String {
    let prefix = format!("{name} := \"");
    let line = source
        .lines()
        .find(|l| l.starts_with(&prefix))
        .unwrap_or_else(|| panic!("no `{name} := \"...\"` assignment found in the justfile"));
    line[prefix.len()..]
        .strip_suffix('"')
        .expect("a just string assignment ends in a quote")
        .to_string()
}

// ---------------------------------------------------------------------------
// 1. Rust
// ---------------------------------------------------------------------------

#[test]
fn the_rust_stripper_recovers_every_dataset_name() {
    for (input, expected) in CASES {
        assert_eq!(
            strip_known_extension(input),
            *expected,
            "strip_known_extension('{input}')"
        );
    }
}

#[test]
fn the_most_specific_extension_wins() {
    // `.city.jsonl` must be tried before `.jsonl`, and `.citygml` before
    // `.gml` - otherwise the leading component of the name survives as a
    // stray `.city`/`.city` fragment.
    let ordered: Vec<&str> = KNOWN_INPUT_EXTENSIONS.to_vec();
    for (i, ext) in ordered.iter().enumerate() {
        for longer in &ordered[i + 1..] {
            assert!(
                !longer.ends_with(ext),
                "'{longer}' is more specific than '{ext}' but is tried after it"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 2. scripts/readbench_prepare.sh
// ---------------------------------------------------------------------------

#[test]
fn the_prepare_script_strips_identically() {
    let script = read("scripts/readbench_prepare.sh");
    let array = shell_array_line(&script, "KNOWN_INPUT_EXTENSIONS");
    let function = extract_shell_function(&script, "strip_known_extension");
    let program = format!("set -euo pipefail\n{array}\n{function}\nstrip_known_extension \"$1\"\n");
    assert_matches_the_table(
        "scripts/readbench_prepare.sh",
        &run_bash_over_cases("prepare-script", &program),
    );
}

#[test]
fn the_prepare_script_carries_the_same_extension_list() {
    let script = read("scripts/readbench_prepare.sh");
    let array = shell_array_line(&script, "KNOWN_INPUT_EXTENSIONS");
    let inner = array
        .split_once('(')
        .and_then(|(_, rest)| rest.rsplit_once(')'))
        .expect("a bash array literal")
        .0;
    let listed: Vec<&str> = inner.split_whitespace().collect();
    assert_eq!(
        listed,
        KNOWN_INPUT_EXTENSIONS.to_vec(),
        "scripts/readbench_prepare.sh's KNOWN_INPUT_EXTENSIONS has drifted from the Rust one"
    );
}

/// `name() { ... }` from `source`, up to the first line that is a bare `}`.
fn extract_shell_function(source: &str, name: &str) -> String {
    let opener = format!("{name}() {{");
    let mut lines = source.lines().skip_while(|l| !l.starts_with(&opener));
    let mut out = String::new();
    for line in lines.by_ref() {
        out.push_str(line);
        out.push('\n');
        if line == "}" {
            return out;
        }
    }
    panic!("no `{name}()` shell function found");
}

// ---------------------------------------------------------------------------
// 3. the justfile (four recipes, one shared extension list)
// ---------------------------------------------------------------------------

/// Every `for ext in {{KNOWN_INPUT_EXTENSIONS}} ... done` block in the
/// justfile, dedented.
fn justfile_stripper_blocks(justfile: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<Vec<String>> = None;
    for line in justfile.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("for ext in {{KNOWN_INPUT_EXTENSIONS}}") {
            current = Some(Vec::new());
        }
        if let Some(block) = current.as_mut() {
            block.push(trimmed.to_string());
            if trimmed == "done" {
                blocks.push(current.take().expect("in a block").join("\n"));
            }
        }
    }
    blocks
}

#[test]
fn the_justfile_has_exactly_one_stripper_repeated_verbatim() {
    let justfile = read_mono("justfile");
    let blocks = justfile_stripper_blocks(&justfile);
    // convert-all, bench, write-bench, compression-bench.
    assert_eq!(
        blocks.len(),
        4,
        "expected four per-dataset recipes to strip the input extension, found {}",
        blocks.len()
    );
    for block in &blocks[1..] {
        assert_eq!(
            block, &blocks[0],
            "a justfile recipe's stripper has drifted from the others"
        );
    }
    assert!(
        !justfile.contains("%.city.jsonl}"),
        "a justfile recipe still carries its own inline extension list"
    );
    // The line the block operates on is part of the convention too: a recipe
    // that fed the stripper a full PATH rather than a basename would produce
    // a `<dir>/<dataset>` "name" and every artefact path built from it would
    // be wrong in a way the block itself cannot catch.
    assert_eq!(
        justfile.matches(r#"name="$(basename "$f")""#).count(),
        4,
        "every per-dataset recipe must strip the basename, not the path"
    );
}

#[test]
fn the_justfile_strips_identically() {
    let justfile = read_mono("justfile");
    let extensions = just_variable(&justfile, "KNOWN_INPUT_EXTENSIONS");
    let block = justfile_stripper_blocks(&justfile)
        .into_iter()
        .next()
        .expect("at least one stripper block");
    let program = format!(
        "set -euo pipefail\nname=\"$(basename \"$1\")\"\n{}\nprintf '%s' \"$name\"\n",
        block.replace("{{KNOWN_INPUT_EXTENSIONS}}", &extensions)
    );
    assert_matches_the_table("the justfile", &run_bash_over_cases("justfile", &program));
}

#[test]
fn the_justfile_carries_the_same_extension_list() {
    let justfile = read_mono("justfile");
    let listed: Vec<String> = just_variable(&justfile, "KNOWN_INPUT_EXTENSIONS")
        .split_whitespace()
        .map(str::to_string)
        .collect();
    assert_eq!(
        listed,
        KNOWN_INPUT_EXTENSIONS
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>(),
        "the justfile's KNOWN_INPUT_EXTENSIONS has drifted from the Rust one"
    );
}

// ---------------------------------------------------------------------------
// 4. the justfile's input-discovery `find` pattern
// ---------------------------------------------------------------------------

/// A stripper that knows an extension the discovery `find` never matches is
/// dead code: the input is not found at all, so nothing is ever stripped.
/// This is the half of the bug that made a `.gml` input invisible to every
/// recipe.
#[test]
fn every_known_extension_is_discoverable() {
    let justfile = read_mono("justfile");
    let pattern = just_variable(&justfile, "KNOWN_INPUT_FIND");
    let globs: Vec<String> = pattern
        .split_whitespace()
        .filter(|t| t.starts_with("'*") && t.ends_with('\''))
        .map(|t| t.trim_matches('\'').trim_start_matches('*').to_string())
        .collect();
    assert!(!globs.is_empty(), "no `-name '*.ext'` globs in {pattern}");
    for ext in KNOWN_INPUT_EXTENSIONS {
        assert!(
            globs.iter().any(|g| ext.ends_with(g.as_str())),
            "no `find` glob in KNOWN_INPUT_FIND matches a '{ext}' input"
        );
    }
}

#[test]
fn every_recipe_discovers_inputs_through_the_shared_pattern() {
    let justfile = read_mono("justfile");
    assert_eq!(
        justfile.matches("{{KNOWN_INPUT_FIND}}").count(),
        4,
        "expected four per-dataset recipes to discover inputs through KNOWN_INPUT_FIND"
    );
    let inline = justfile
        .lines()
        .filter(|l| !l.starts_with("KNOWN_INPUT_FIND :="))
        .filter(|l| l.contains("-name '*."))
        .count();
    assert_eq!(
        inline, 0,
        "a justfile recipe still carries its own inline `find` name pattern"
    );
}

// ---------------------------------------------------------------------------
// 5. scripts/readbench_duckdb.sh — the composable package-name counterpart
// ---------------------------------------------------------------------------

/// `readbench_duckdb.sh` is handed a `<dataset>.parquet` PACKAGE directory,
/// never the original input, so it strips `.parquet`/`-hilbert` rather than
/// an input extension — a different convention, which is exactly why it is
/// checked by COMPOSITION rather than by running it over [`CASES`]
/// directly. What must hold is that the dataset name the justfile derives
/// (by stripping the input extension) survives the round trip through the
/// package name this script is given back, for every input the benchmark
/// now accepts — including the CityGML ones.
#[test]
fn the_duckdb_baseline_recovers_the_justfile_dataset_name() {
    let script = read("scripts/readbench_duckdb.sh");
    let stripper: String = script
        .lines()
        .filter(|l| l.starts_with("PKG_BASE=\"${PKG_BASE%"))
        .map(|l| format!("{l}\n"))
        .collect();
    assert!(
        stripper.lines().count() >= 2,
        "expected readbench_duckdb.sh to strip both `.parquet` and `-hilbert`"
    );
    for (input, dataset) in CASES {
        for suffix in [".parquet", "-hilbert.parquet"] {
            let program = format!(
                "set -euo pipefail\nPKG_BASE=\"$(basename \"$1\")\"\n{stripper}printf '%s' \"$PKG_BASE\"\n"
            );
            let package = format!("benchmark/formats/data/readbench/{dataset}{suffix}");
            let out = Command::new("bash")
                .arg("-c")
                .arg(&program)
                .arg("bash")
                .arg(&package)
                .output()
                .expect("running readbench_duckdb.sh's package-name stripper");
            assert!(out.status.success());
            assert_eq!(
                String::from_utf8_lossy(&out.stdout),
                *dataset,
                "the package the justfile derives from '{input}' ({package}) does not strip \
                 back to '{dataset}'"
            );
        }
    }
}
