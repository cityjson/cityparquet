# STAC Catalog → CityParquet Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Convert every item in the published City3D STAC catalog into a CityParquet package and reassemble the results into a mirror STAC catalog, skipping and recording failures rather than aborting.

**Architecture:** A Python/uv driver (`tools/catalog2cityparquet`) orchestrates: enumerate items → download → decompress → shell out to `cityparquet convert` → stamp the emitted `metadata.json` → aggregate with `city3dstac`. All format logic stays in Rust; the driver only moves bytes and tallies outcomes. Three small Rust reader fixes and one additive stac-tool flag precede it.

**Tech Stack:** Rust (cityparquet-rs, city3d-stac-tool), Python 3.11+ with uv (httpx, duckdb, PyYAML, pytest), DuckDB for remote stac-geoparquet reads.

**Design spec:** `docs/superpowers/specs/2026-08-11-stac-catalog-to-cityparquet-design.md`

## Global Constraints

- **British English** in all prose, comments and docs.
- **Strict red-green TDD**: failing test first, smallest change to pass, then refactor. Never write implementation before a failing test.
- **Rust tests read real CityJSON/CityGML fixtures** from `tests/fixtures/` — never inline hand-written CityJSON/CityGML. New fixtures are downloaded by `just fixtures`.
- **`cityparquet-schema` must stay free of `arrow-array`/`parquet`** — verified by `just isolation`.
- **`just check` must be green** (clippy `-D warnings` + tests + isolation + `fmt --check`) before any Rust task is considered done.
- **`city3d-stac-tool` changes must be backward compatible** — the registry repo and its CI depend on the existing CLI surface. New flags are additive and optional only.
- Catalog base URL: `https://storage.googleapis.com/city3d-stac`
- GCS listing API: `https://storage.googleapis.com/storage/v1/b/city3d-stac/o?prefix=<prefix>&maxResults=1000` (paginate via `nextPageToken`).
- Converter binary: `target/release/cityparquet` (build with `cargo build --release -p cityparquet-cli`).
- Closed ledger reason vocabulary: `download_failed`, `unsupported_archive`, `unsupported_citygml_version`, `unsupported_cityjson_version`, `no_crs`, `geographic_crs`, `convert_failed`, `empty_collection`, `duplicate_bundle`, `stale_item_index`.

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/cityparquet/src/citygml/sniff.rs` | *(modify)* detect `CityModel` in any CityGML namespace; report version |
| `crates/cityparquet/src/citygml/xml.rs` | *(modify)* add `NS_CITYGML_FAMILY` constant |
| `crates/cityparquet/src/source.rs` | *(modify)* route a non-2.0 CityGML to a clear error |
| `crates/cityparquet/src/citygml/header.rs` | *(modify)* `srsName` fallback past the preamble |
| `crates/cityparquet/src/scan.rs` | *(modify)* `crs_override` honoured; `crs_source` provenance in `city.other` |
| `crates/cityparquet/src/package.rs` | *(modify)* `ConvertOptions.crs_override` |
| `crates/cityparquet-cli/src/main.rs` | *(modify)* `--crs` flag |
| `vendor/city3d-stac-tool/src/cli/mod.rs` | *(modify)* `--items-dir` on `update-collection` |
| `tools/catalog2cityparquet/src/catalog2cityparquet/ledger.py` | outcome records, summary.csv, histogram |
| `…/discover.py` | catalog → collections → items, with index reconciliation |
| `…/fetch.py` | download, magic-byte sniff, decompress, extract, skip rules |
| `…/convert.py` | invoke `cityparquet convert`, stamp `metadata.json` |
| `…/aggregate.py` | emit config YAML, invoke `city3dstac` |
| `…/__main__.py` | CLI, orchestration, failure isolation, summary |
| `justfile` | *(modify)* `catalog-convert*` recipes |

---

### Task 1: `--items-dir` on `city3d-stac-tool update-collection`

**Files:**
- Create: `vendor/city3d-stac-tool` (git submodule)
- Modify: `vendor/city3d-stac-tool/src/cli/mod.rs` (the `UpdateCollection` variant near line 165, and its match arm near line 437)
- Test: `vendor/city3d-stac-tool/tests/items_dir.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `city3dstac update-collection --items-dir <DIR>` — walks `DIR` recursively, accepting any `.json` whose parsed body has `"type": "Feature"` and a `stac_version`, skipping files named `collection.json` or `catalog.json`. Unioned with positional `items` and `--items-from-file`.

- [ ] **Step 1: Add the submodule and branch**

```bash
cd /data2/hideba/cityparquet-paper/cityparquet-rs
git submodule add git@github.com:cityjson/city3d-stac-tool.git vendor/city3d-stac-tool
cd vendor/city3d-stac-tool && git checkout -b feat/items-dir
```

- [ ] **Step 2: Write the failing test**

Create `vendor/city3d-stac-tool/tests/items_dir.rs`:

```rust
use std::fs;
use std::process::Command;

/// `--items-dir` must find STAC Items by CONTENT, not by filename: CityParquet
/// names its Item `metadata.json`, while this tool's own convention is
/// `*_item.json`. Both must be picked up, nested directories included, and
/// `collection.json` must be ignored.
#[test]
fn items_dir_collects_items_by_content_recursively() {
    let tmp = tempfile::tempdir().unwrap();
    let items = tmp.path().join("items");
    fs::create_dir_all(items.join("a")).unwrap();
    fs::create_dir_all(items.join("b")).unwrap();

    let item = |id: &str| {
        format!(
            r#"{{"type":"Feature","stac_version":"1.1.0","id":"{id}",
                 "geometry":null,"bbox":null,"properties":{{"datetime":null}},
                 "links":[],"assets":{{}}}}"#
        )
    };
    fs::write(items.join("a/metadata.json"), item("a")).unwrap();
    fs::write(items.join("b/b_item.json"), item("b")).unwrap();
    // Must be ignored: a Collection, not an Item.
    fs::write(
        items.join("collection.json"),
        r#"{"type":"Collection","stac_version":"1.1.0","id":"c",
            "description":"x","license":"other","extent":{},"links":[]}"#,
    )
    .unwrap();

    let out = tmp.path().join("collection.json");
    let status = Command::new(env!("CARGO_BIN_EXE_city3dstac"))
        .args(["update-collection", "--items-dir"])
        .arg(&items)
        .args(["--id", "test-collection", "--description", "d", "-o"])
        .arg(&out)
        .status()
        .unwrap();
    assert!(status.success(), "update-collection must succeed");

    let written: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&out).unwrap()).unwrap();
    let item_links: Vec<_> = written["links"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|l| l["rel"] == "item")
        .collect();
    assert_eq!(item_links.len(), 2, "both items must be aggregated: {written}");
}
```

- [ ] **Step 3: Run it to verify it fails**

Run: `cd vendor/city3d-stac-tool && cargo test --test items_dir`
Expected: FAIL — `error: unexpected argument '--items-dir' found`

- [ ] **Step 4: Add the flag to the CLI enum**

In `src/cli/mod.rs`, inside the `UpdateCollection` variant, immediately after the `items_from_file` field:

```rust
        /// Recursively collect STAC item files from a directory.
        ///
        /// Any `.json` file that parses as a STAC Item (`type: "Feature"` with a
        /// `stac_version`) is collected; `collection.json` and `catalog.json` are
        /// skipped. Content sniffing rather than a filename glob, because
        /// CityParquet names its Item `metadata.json` while this tool's own
        /// convention is `*_item.json`.
        ///
        /// Composes with the positional `items` and `--items-from-file`: the
        /// union of all three is aggregated.
        #[arg(long)]
        items_dir: Option<PathBuf>,
```

Relax the existing guard on the positional argument so a directory alone suffices:

```rust
        #[arg(required_unless_present_any = ["items_from_file", "items_dir"])]
        items: Vec<PathBuf>,
```

- [ ] **Step 5: Collect items from the directory in the match arm**

In the `Commands::UpdateCollection { .. }` match arm, add `items_dir,` to the destructured field list, then after the existing `items_from_file` block:

```rust
            if let Some(dir) = items_dir {
                all_items.extend(collect_item_files(&dir)?);
            }
```

Add this free function at the end of `src/cli/mod.rs`:

```rust
/// Recursively collect files under `dir` that parse as STAC Items.
///
/// Selection is by CONTENT (`type == "Feature"` plus a `stac_version`), never by
/// filename, so both `metadata.json` (CityParquet) and `*_item.json` (this
/// tool's own convention) are found. `collection.json` and `catalog.json` are
/// skipped by name — they are valid JSON but never Items.
fn collect_item_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = std::fs::read_dir(&current).map_err(CityJsonStacError::IoError)?;
        for entry in entries {
            let path = entry.map_err(CityJsonStacError::IoError)?.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name == "collection.json" || name == "catalog.json" {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            if value.get("type").and_then(|t| t.as_str()) == Some("Feature")
                && value.get("stac_version").is_some()
            {
                found.push(path);
            }
        }
    }
    found.sort();
    Ok(found)
}
```

- [ ] **Step 6: Run the test to verify it passes**

Run: `cd vendor/city3d-stac-tool && cargo test --test items_dir`
Expected: PASS

- [ ] **Step 7: Verify backward compatibility**

Run: `cd vendor/city3d-stac-tool && cargo test --all-features && cargo clippy --all-targets --all-features -- -D warnings`
Expected: PASS — no existing test may change behaviour.

- [ ] **Step 8: Commit both repos**

```bash
cd vendor/city3d-stac-tool
git add src/cli/mod.rs tests/items_dir.rs
git commit -m "feat(update-collection): add --items-dir for recursive Item discovery

Collects STAC Items by content rather than filename so CityParquet's
metadata.json and this tool's *_item.json are both found. Additive and
optional: existing invocations are unaffected."
cd ../..
git add .gitmodules vendor/city3d-stac-tool
git commit -m "build: vendor city3d-stac-tool as a submodule"
```

---

### Task 2: Clear error for unsupported CityGML versions

**Files:**
- Modify: `crates/cityparquet/src/citygml/xml.rs` (add constant after `NS_CORE`, line 20)
- Modify: `crates/cityparquet/src/citygml/sniff.rs` (whole file)
- Modify: `crates/cityparquet/src/source.rs` (`Source::open`, the `is_citygml` branch at line 53)
- Modify: `justfile` (`fixtures` recipe)
- Test: `crates/cityparquet/tests/citygml_version_error.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `citygml::sniff_citygml(path: &Path) -> Option<CityGmlVersion>` where `pub enum CityGmlVersion { V2_0, Other(String) }`. `citygml::is_citygml(path) -> bool` is retained, returning `true` only for `V2_0`, so existing callers are unaffected.

- [ ] **Step 1: Add a real CityGML 1.0 fixture to `just fixtures`**

Append to the `fixtures` recipe in `justfile`:

```
    # Real CityGML 1.0 fixture (jklimke/libcitygml, same pinned commit as the
    # 2.0 fixtures above): a Berlin open-data sample whose root <CityModel>
    # binds the default namespace to .../citygml/1.0. Used by
    # `citygml_version_error` to prove a non-2.0 document fails with a clear
    # version message instead of a bogus "invalid CityJSON" JSON parse error.
    curl -sSfo tests/fixtures/berlin_citygml1.gml https://raw.githubusercontent.com/jklimke/libcitygml/141ed719c0ccdf8691e1dc98aa4f915438292b6b/data/berlin_open_data_sample_data.citygml
```

Run: `just fixtures`
Then confirm the ROOT element (not just some child namespace) is CityGML 1.0:
Run: `grep -o 'xmlns="http://www.opengis.net/citygml/1.0"' tests/fixtures/berlin_citygml1.gml | head -1`
Expected: prints `xmlns="http://www.opengis.net/citygml/1.0"` (verified 2026-08-11: HTTP 200, 932 KB, root `<CityModel>` carries exactly this default namespace).

- [ ] **Step 2: Write the failing test**

Create `crates/cityparquet/tests/citygml_version_error.rs`:

```rust
//! A non-2.0 CityGML document must fail with a CityGML *version* error.
//!
//! Before this, `is_citygml` returned false for any non-2.0 namespace, so the
//! file fell through to the CityJSON branch and reported
//! `invalid CityJSON: expected value at line 1 column 1` — a JSON parse error
//! for an XML file. That message sent every reader hunting the wrong problem,
//! and it is what 11 collections of the City3D catalog hit.

use std::path::Path;

use cityparquet::source::Source;

#[test]
fn citygml_1_0_reports_a_version_error_not_a_json_error() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/berlin_citygml1.gml");
    let err = Source::open(&path).expect_err("CityGML 1.0 must not open as a source");
    let msg = err.to_string();
    assert!(
        msg.contains("unsupported CityGML version"),
        "error must name the CityGML version problem, got: {msg}"
    );
    assert!(
        msg.contains("1.0"),
        "error must name the detected version, got: {msg}"
    );
    assert!(
        !msg.contains("invalid CityJSON"),
        "must NOT report a JSON parse error for an XML file, got: {msg}"
    );
}
```

- [ ] **Step 3: Run it to verify it fails**

Run: `cargo test -p cityparquet --test citygml_version_error`
Expected: FAIL — the message contains `invalid CityJSON: expected value at line 1 column 1`.

- [ ] **Step 4: Add the family namespace constant**

In `crates/cityparquet/src/citygml/xml.rs`, after `NS_CORE` (line 20):

```rust
/// The CityGML core namespace *family* — every version shares this prefix
/// (`.../citygml/1.0`, `.../citygml/2.0`, `.../citygml/3.0`). Matched to
/// recognise a CityGML document of ANY version so an unsupported one can be
/// reported as such, rather than falling through to the CityJSON sniff and
/// failing as malformed JSON.
pub const NS_CITYGML_FAMILY: &str = "http://www.opengis.net/citygml";
```

- [ ] **Step 5: Rewrite the sniffer to report the version**

Replace the body of `crates/cityparquet/src/citygml/sniff.rs` below the imports:

```rust
use super::xml::{NS_CITYGML_FAMILY, NS_CORE, ns_is};

/// Which CityGML version a document declares on its `CityModel` root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CityGmlVersion {
    /// CityGML 2.0 — the only version this reader supports.
    V2_0,
    /// Any other CityGML version, as it appears in the namespace (e.g. "1.0").
    Other(String),
}

/// Detect a CityGML document of any version by its root element.
///
/// `None` means "not CityGML at all" (so the caller falls back to CityJSON);
/// `Some(Other(v))` means "CityGML, but not a version we read" — which the
/// caller must report as a version error rather than a JSON parse failure.
pub fn sniff_citygml(path: &Path) -> Option<CityGmlVersion> {
    let file = File::open(path).ok()?;
    let mut reader = NsReader::from_reader(BufReader::new(file));
    let mut buf = Vec::new();
    // Bound the scan: the root element appears within the first handful of
    // events (declaration, comments, then the root Start).
    for _ in 0..64 {
        buf.clear();
        match reader.read_resolved_event_into(&mut buf) {
            Ok((rr, Event::Start(e))) => {
                if e.local_name().as_ref() != b"CityModel" {
                    return None;
                }
                if ns_is(&rr, NS_CORE) {
                    return Some(CityGmlVersion::V2_0);
                }
                if ns_is(&rr, NS_CITYGML_FAMILY) {
                    return Some(CityGmlVersion::Other(version_from_ns(&rr)));
                }
                return None;
            }
            Ok((_, Event::Eof)) | Err(_) => return None,
            Ok(_) => {}
        }
    }
    None
}

/// The trailing version segment of a CityGML core namespace
/// (`http://www.opengis.net/citygml/1.0` -> `"1.0"`), or `"unknown"`.
fn version_from_ns(rr: &quick_xml::name::ResolveResult) -> String {
    let quick_xml::name::ResolveResult::Bound(ns) = rr else {
        return "unknown".to_string();
    };
    std::str::from_utf8(ns.as_ref())
        .ok()
        .and_then(|s| s.rsplit('/').next())
        .filter(|s| !s.is_empty())
        .unwrap_or("unknown")
        .to_string()
}

/// True when `path` is a CityGML **2.0** document. Retained for callers that
/// only need the yes/no answer; [`sniff_citygml`] carries the version.
pub fn is_citygml(path: &Path) -> bool {
    matches!(sniff_citygml(path), Some(CityGmlVersion::V2_0))
}
```

Update the module doc comment at the top of the file to describe the new behaviour, and export the new items from `crates/cityparquet/src/citygml/mod.rs` alongside the existing `is_citygml` re-export:

```rust
pub use sniff::{CityGmlVersion, is_citygml, sniff_citygml};
```

- [ ] **Step 6: Route the unsupported version to a clear error**

In `crates/cityparquet/src/source.rs`, replace the `if crate::citygml::is_citygml(path) { … }` block at the top of `Source::open` with:

```rust
        // CityGML is XML, not JSON — detect it by its root element before the
        // CityJSON/Seq sniff below. A CityGML document of an unsupported
        // version is reported as such: letting it fall through to the JSON
        // branch produced "invalid CityJSON: expected value at line 1 column 1"
        // for an XML file, which is actively misleading.
        match crate::citygml::sniff_citygml(path) {
            Some(crate::citygml::CityGmlVersion::V2_0) => {
                let header = crate::citygml::parse_header(path)?;
                return Ok(Self {
                    path: path.to_path_buf(),
                    format: SourceFormat::CityGml,
                    header,
                    doc: None,
                    buffered: None,
                });
            }
            Some(crate::citygml::CityGmlVersion::Other(version)) => {
                return Err(err(format!(
                    "unsupported CityGML version {version} (only CityGML 2.0 is supported)"
                )));
            }
            None => {}
        }
```

- [ ] **Step 7: Run the test to verify it passes**

Run: `cargo test -p cityparquet --test citygml_version_error`
Expected: PASS

- [ ] **Step 8: Verify nothing else regressed, then commit**

Run: `just check`
Expected: PASS

```bash
git add crates/cityparquet/src/citygml/sniff.rs crates/cityparquet/src/citygml/xml.rs \
        crates/cityparquet/src/citygml/mod.rs crates/cityparquet/src/source.rs \
        crates/cityparquet/tests/citygml_version_error.rs justfile
git commit -m "fix(citygml): report unsupported CityGML versions clearly

A 1.0/3.0 document fell through to the CityJSON branch and failed with
'invalid CityJSON: expected value at line 1 column 1' for an XML file.
Sniff the CityModel root in any citygml namespace and name the version."
```

---

### Task 3: `srsName` fallback beyond the preamble

**Files:**
- Modify: `crates/cityparquet/src/citygml/header.rs` (`scan_envelope`, lines 61-105)
- Modify: `justfile` (`fixtures` recipe)
- Test: `crates/cityparquet/tests/citygml_srsname_fallback.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: no signature change — `scan_envelope` keeps its `(Option<String>, Option<[f64; 6]>)` return. Behaviour change only.

**Critical detail:** the fallback scan must collect **only** `srsName`, never `lowerCorner`/`upperCorner`. Per-object `gml:boundedBy` envelopes exist in these files, and adopting one as the dataset envelope would set `geographical_extent` to a single building's extent and skew the quantisation origin. Vertices are `i64`, so a `[0,0,0]` translate cannot overflow even for large projected coordinates — leaving the envelope absent is safe.

- [ ] **Step 1: Add the real preamble-less CityGML fixture**

This shape is genuinely rare: a survey of German AdV state exports, Luxembourg, PLATEAU and the OGC samples found that every one emits a root `gml:Envelope` carrying `srsName`. The one confirmed real example is Freiburg's own 1.86 GiB export — the very file this fix exists for — so the fixture is a **byte-prefix of that real file**, fetched with an HTTP range request. The bytes are unaltered published data; only the tail is absent, and `parse_header` stops at the first `srsName` (byte ~2,600) so it never reads that far.

Append to the `fixtures` recipe in `justfile`:

```
    # First 400 kB of Freiburg's real LoD2 CityGML 2.0 export: a file that
    # declares its CRS ONLY inside city objects (srsName appears 60,108 times
    # in the full file, never before the first cityObjectMember at byte 2534).
    # A range request, not the whole 1.86 GiB - `parse_header` stops at the
    # first srsName, so the truncated tail is never read. Used by
    # `citygml_srsname_fallback`; this exact shape could not be found in any
    # smaller published CityGML 2.0 file.
    curl -sSf -r 0-399999 -o tests/fixtures/freiburg_no_preamble_srs.gml https://geoportal.freiburg.de/stadtmodell/20240426_Freiburg_LoD2.gml
```

Run: `just fixtures`
Then confirm the shape is what both tests need:
```bash
python3 -c "
t=open('tests/fixtures/freiburg_no_preamble_srs.gml',encoding='utf8',errors='replace').read()
k=t.find('cityObjectMember')
print('root 2.0      :', 'xmlns=\"http://www.opengis.net/citygml/2.0\"' in t[:3000])
print('first COM byte:', k)
print('srsName before:', 'srsName' in t[:k])
print('srsName after :', 'srsName' in t[k:])
print('corners before:', 'lowerCorner' in t[:k])
print('corners after :', 'lowerCorner' in t[k:])
"
```
Expected (verified 2026-08-11): `root 2.0: True`, `first COM byte: 2534`, `srsName before: False`, `srsName after: True`, `corners before: False`, `corners after: True`. The last two are what make the second test meaningful — there are per-object envelopes to be wrongly adopted.

- [ ] **Step 2: Write the failing test**

Create `crates/cityparquet/tests/citygml_srsname_fallback.rs`:

```rust
//! A CityGML file may declare its CRS only inside city objects.
//!
//! `scan_envelope` stopped at the first `cityObjectMember`, assuming an
//! envelope always precedes it. Freiburg's export declares
//! `urn:ogc:def:crs:EPSG::25832` 60,108 times — once per building — and never
//! in the preamble, so the scanner saw no CRS and the writer's CRS rule
//! hard-failed a file that plainly declares one.

use std::path::Path;

use cityparquet::citygml::parse_header;

#[test]
fn srs_name_is_found_when_declared_only_inside_city_objects() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/freiburg_no_preamble_srs.gml");
    let header = parse_header(&path).expect("header must parse");
    let metadata = header
        .metadata
        .expect("a file declaring a CRS must produce metadata");
    let rs = metadata
        .reference_system
        .expect("srsName inside a city object must still yield a reference system");
    let rs = serde_json::to_value(&rs).unwrap();
    assert!(
        rs.as_str().unwrap().contains("25832"),
        "expected EPSG:25832, got {rs}"
    );
}

#[test]
fn a_per_object_envelope_does_not_become_the_dataset_extent() {
    // The fallback must collect ONLY srsName. A per-object `gml:boundedBy`
    // adopted as the dataset envelope would set geographical_extent to one
    // building's extent and skew the quantisation origin.
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/freiburg_no_preamble_srs.gml");
    let header = parse_header(&path).expect("header must parse");
    assert_eq!(
        header.transform.translate,
        vec![0.0, 0.0, 0.0],
        "no preamble envelope means a zero translate, not a per-object corner"
    );
    let extent = header.metadata.and_then(|m| m.geographical_extent);
    assert!(
        extent.is_none(),
        "a per-object envelope must not become the dataset extent, got {extent:?}"
    );
}
```

- [ ] **Step 3: Run it to verify it fails**

Run: `cargo test -p cityparquet --test citygml_srsname_fallback`
Expected: FAIL on the first test — `srsName inside a city object must still yield a reference system`.

- [ ] **Step 4: Implement the fallback**

In `crates/cityparquet/src/citygml/header.rs`, replace the `loop { … }` inside `scan_envelope` with:

```rust
    // Once past the preamble we keep scanning for an `srsName` ONLY — never
    // for corners. Real exports (e.g. Freiburg's 1.86 GiB file) declare the CRS
    // on every object's own `gml:boundedBy` and never in the preamble; adopting
    // one of those envelopes as the DATASET extent would report a single
    // building's extent and skew the quantisation origin, so corners stay
    // preamble-only.
    let mut past_preamble = false;
    // Bound the fallback: a real file declares its CRS on the first object, so
    // this stops almost immediately. The cap keeps a pathological CRS-less file
    // from being read end-to-end just to fail.
    let mut fallback_events = 0usize;
    const MAX_FALLBACK_EVENTS: usize = 100_000;

    loop {
        buf.clear();
        let (rr, ev) = reader.read_resolved_event_into(&mut buf).map_err(xml_err)?;
        match ev {
            Event::Start(e) => {
                let local = e.local_name();
                let name = local.as_ref().to_vec();
                // The dataset CRS can sit on the CityModel root or the Envelope.
                if srs_name.is_none()
                    && let Some(s) = get_attr(&e, b"srsName")
                {
                    srs_name = Some(s);
                    if past_preamble {
                        break; // fallback satisfied
                    }
                }
                if past_preamble {
                    fallback_events += 1;
                    if fallback_events >= MAX_FALLBACK_EVENTS {
                        break;
                    }
                    continue;
                }
                if ns_is(&rr, NS_GML) && name == b"lowerCorner" {
                    lower = parse_corner(&read_text(&mut reader, &mut buf)?);
                } else if ns_is(&rr, NS_GML) && name == b"upperCorner" {
                    upper = parse_corner(&read_text(&mut reader, &mut buf)?);
                } else if name == b"cityObjectMember" {
                    if srs_name.is_some() {
                        break; // preamble gave us everything
                    }
                    past_preamble = true;
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
```

Update `scan_envelope`'s doc comment:

```rust
/// Read the preamble and return `(srsName, [minx,miny,minz,maxx,maxy,maxz])`.
///
/// Corners are read from the preamble only. When the preamble declares no
/// `srsName`, the scan continues into city objects looking for one — and for
/// nothing else — because real exports declare the CRS per object and never
/// ahead of the first `cityObjectMember`.
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p cityparquet --test citygml_srsname_fallback`
Expected: PASS (both tests)

- [ ] **Step 6: Verify the whole suite, then commit**

Run: `just check`
Expected: PASS

```bash
git add crates/cityparquet/src/citygml/header.rs \
        crates/cityparquet/tests/citygml_srsname_fallback.rs justfile
git commit -m "fix(citygml): find srsName declared only inside city objects

scan_envelope stopped at the first cityObjectMember, so a file declaring its
CRS per object (never in the preamble) was rejected as having none. Scan on
for srsName only - corners stay preamble-only so a per-object envelope never
becomes the dataset extent."
```

---

### Task 4: `--crs` override

**Files:**
- Modify: `crates/cityparquet/src/scan.rs` (`ScanResult`/`CityParquetSchema` carry `crs_source`; `base_city_metadata` at line 596; the CRS error at line 400)
- Modify: `crates/cityparquet/src/source.rs` (add `Source::set_reference_system`)
- Modify: `crates/cityparquet/src/package.rs` (`ConvertOptions.crs_override`, applied in `convert_source`)
- Modify: `crates/cityparquet-cli/src/main.rs` (`--crs` flag on `Convert`)
- Test: `crates/cityparquet/tests/crs_override.rs`

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `ConvertOptions.crs_override: Option<String>` — defaults to `None` in `ConvertOptions::new`.
  - `Source::set_reference_system(&mut self, epsg_code: &str)` — sets `header.metadata.reference_system` when absent; no-op when the source already declares one.
  - CLI: `cityparquet convert … --crs EPSG:25832`.
  - Footer: `city.other.crs_source == "operator-supplied"` whenever the override was actually applied.

**Spec position (design §8.2):** the rule forbids writing with `crs` absent and forbids guessing. An explicit operator-supplied CRS is neither. The provenance stamp keeps the output honest about where the CRS came from. Absent `--crs`, behaviour is unchanged.

- [ ] **Step 1: Write the failing test**

Create `crates/cityparquet/tests/crs_override.rs`:

```rust
//! `--crs` lets an operator supply a CRS a source does not declare.
//!
//! Without it, a CRS-less source is a hard conversion error (spec
//! "CRS rules"). The override is not a guess and not an absent CRS: it makes
//! the CRS resolvable before the writer runs, and is stamped as
//! operator-supplied in `city.other` so the output never implies the SOURCE
//! declared it.

use std::fs;
use std::path::{Path, PathBuf};

use cityparquet::package::{ConvertOptions, convert_source};
use cityparquet::reader::CityParquetReaderBuilder;
use cityparquet::source::Source;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

/// A real CityJSON fixture with its `referenceSystem` removed — the shape of
/// the four catalog collections whose CityJSON carries no CRS at all.
fn crs_less_fixture(dir: &Path) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/delft.city.jsonl");
    let text = fs::read_to_string(src).expect("fixture must exist; run `just fixtures`");
    let mut lines = text.lines();
    let mut header: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    header["metadata"]
        .as_object_mut()
        .expect("delft header has metadata")
        .remove("referenceSystem");
    let dest = dir.join("no_crs.city.jsonl");
    let mut out = serde_json::to_string(&header).unwrap();
    for line in lines {
        out.push('\n');
        out.push_str(line);
    }
    fs::write(&dest, out).unwrap();
    dest
}

#[test]
fn a_crs_less_source_still_fails_without_the_override() {
    let tmp = tempfile::tempdir().unwrap();
    let input = crs_less_fixture(tmp.path());
    let source = Source::open(&input).unwrap();
    let opts = ConvertOptions::new(input.clone(), tmp.path().join("out"));
    let err = convert_source(&source, &opts).expect_err("no CRS and no override must fail");
    assert!(
        err.to_string().contains("declares no CRS"),
        "got: {err}"
    );
}

#[test]
fn the_override_supplies_the_crs_and_records_its_provenance() {
    let tmp = tempfile::tempdir().unwrap();
    let input = crs_less_fixture(tmp.path());
    let out = tmp.path().join("out");
    let mut source = Source::open(&input).unwrap();
    let mut opts = ConvertOptions::new(input.clone(), out.clone());
    opts.crs_override = Some("EPSG:7415".to_string());
    if let Some(code) = &opts.crs_override {
        source.set_reference_system(code);
    }
    convert_source(&source, &opts).expect("the override must make conversion succeed");

    let table = out.join("building.parquet");
    // The footer accessor idiom used throughout this crate's tests (see
    // `crates/cityparquet/tests/footer_encoding_dispatch.rs`).
    let builder =
        ParquetRecordBatchReaderBuilder::try_new(fs::File::open(&table).unwrap()).unwrap();
    let meta = builder.cityparquet_metadata().expect("footer must parse");
    assert!(meta.crs.is_some(), "city.crs must be populated from the override");
    let other = meta.other.expect("city.other must exist");
    assert_eq!(
        other.get("crs_source").and_then(|v| v.as_str()),
        Some("operator-supplied"),
        "provenance must record that an operator supplied the CRS: {other}"
    );
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p cityparquet --test crs_override`
Expected: FAIL — `no field `crs_override` on type `ConvertOptions``

- [ ] **Step 3: Add the Source mutator**

In `crates/cityparquet/src/source.rs`, inside `impl Source`:

```rust
    /// Declare `epsg_code` (e.g. `"EPSG:7415"`) as this source's reference
    /// system when it has none.
    ///
    /// An operator-supplied CRS for a source that declares none — see
    /// `ConvertOptions::crs_override`. Deliberately a no-op when the source
    /// already declares a CRS: an override must never silently reproject or
    /// relabel data that came with its own, correct CRS.
    pub fn set_reference_system(&mut self, epsg_code: &str) {
        let code = epsg_code.trim_start_matches("EPSG:").to_string();
        let rs = cjseq::ReferenceSystem::new(None, "EPSG".to_string(), "0".to_string(), code);
        let metadata = self.header.metadata.get_or_insert_with(|| cjseq::Metadata {
            geographical_extent: None,
            identifier: None,
            point_of_contact: None,
            reference_date: None,
            reference_system: None,
            title: None,
        });
        if metadata.reference_system.is_none() {
            metadata.reference_system = Some(rs);
        }
    }
```

- [ ] **Step 4: Thread the option and the provenance flag**

In `crates/cityparquet/src/package.rs`, add to `ConvertOptions` (after `lod0`):

```rust
    /// An operator-supplied CRS (e.g. `"EPSG:25832"`) used ONLY when the source
    /// declares none. The spec's CRS rules forbid writing `city.crs` absent and
    /// forbid guessing; an explicit operator declaration is neither, so the
    /// conversion proceeds and `city.other.crs_source` records where the CRS
    /// came from. `None` (the default) leaves the hard failure in place.
    pub crs_override: Option<String>,
```

and `crs_override: None,` to `ConvertOptions::new`.

The scan needs **no** new field. The override is applied to the source header *before* scanning (via `set_reference_system`), so `scan` legitimately sees an ordinary CRS and resolves it the usual way. Only the provenance stamp has to be added, and `opts` is already in scope at the single place the footer metadata is built.

In `crates/cityparquet/src/package.rs`, replace line 950 — `scan_result.base_city_metadata()?,` — with `city_metadata(scan_result, opts)?,` and add this free function to the same module:

```rust
/// The footer's `city` object, plus the CRS-provenance stamp.
///
/// When the CRS came from `--crs` rather than the source, `city.other`
/// records `crs_source: "operator-supplied"`. `city.other` is free-form and
/// explicitly informational per the spec, so this cannot mislead a decoder —
/// but it does stop the output implying the SOURCE declared a CRS it never
/// carried.
fn city_metadata(scan_result: &ScanResult, opts: &ConvertOptions) -> Result<CityMetadata> {
    let mut meta = scan_result.base_city_metadata()?;
    if opts.crs_override.is_some() {
        let mut other = match meta.other.take() {
            Some(serde_json::Value::Object(map)) => map,
            _ => serde_json::Map::new(),
        };
        other.insert(
            "crs_source".to_string(),
            serde_json::Value::String("operator-supplied".to_string()),
        );
        meta.other = Some(serde_json::Value::Object(other));
    }
    Ok(meta)
}
```

Add `use cityparquet_schema::CityMetadata;` to `package.rs`'s imports if it is not already there.

- [ ] **Step 5: Add the CLI flag**

In `crates/cityparquet-cli/src/main.rs`, add to the `Convert` variant:

```rust
        /// Operator-supplied CRS (e.g. EPSG:25832) used ONLY when the source
        /// declares none. Without it, a CRS-bearing source with no resolvable
        /// CRS is a hard conversion error. The output records
        /// `city.other.crs_source = "operator-supplied"`.
        #[arg(long, value_name = "EPSG")]
        crs: Option<String>,
```

and in the `Convert` match arm, after opening the source:

```rust
            if let Some(code) = &crs {
                source.set_reference_system(code);
                options.crs_override = Some(code.clone());
            }
```

(the `source` binding must become `let mut source`).

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p cityparquet --test crs_override`
Expected: PASS (both tests)

- [ ] **Step 7: Prove it end-to-end on the real blocked dataset**

```bash
cargo build --release -p cityparquet-cli
curl -sSLo /tmp/vienna.city.json "$(duckdb -noheader -csv -c "LOAD httpfs; SELECT json_extract_string(to_json(assets),'\$.data.href') FROM read_parquet('https://storage.googleapis.com/city3d-stac/vienna-3d/items.parquet') LIMIT 1;")"
./target/release/cityparquet convert /tmp/vienna.city.json -o /tmp/vienna-pkg --overwrite --crs EPSG:31256
```
Expected: conversion succeeds and prints an object count. Without `--crs` it must still fail.

- [ ] **Step 8: Verify the whole suite, then commit**

Run: `just check`
Expected: PASS

```bash
git add crates/cityparquet/src/scan.rs crates/cityparquet/src/source.rs \
        crates/cityparquet/src/package.rs crates/cityparquet-cli/src/main.rs \
        crates/cityparquet/tests/crs_override.rs
git commit -m "feat(convert): --crs override for sources that declare none

An explicit operator-supplied CRS is neither an absent crs nor a guess, so
the spec's CRS rule is satisfied; city.other.crs_source records that it came
from the operator, not the source. Unchanged behaviour without the flag."
```

---

### Task 5: uv project scaffold and the ledger

**Files:**
- Create: `tools/catalog2cityparquet/pyproject.toml`
- Create: `tools/catalog2cityparquet/src/catalog2cityparquet/__init__.py` (empty)
- Create: `tools/catalog2cityparquet/src/catalog2cityparquet/ledger.py`
- Test: `tools/catalog2cityparquet/tests/test_ledger.py`

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `Status` = `Literal["converted", "failed", "skipped"]`
  - `REASONS: frozenset[str]` — the closed vocabulary from Global Constraints.
  - `@dataclass(frozen=True) Record(collection: str, item_id: str, status: Status, reason: str | None = None, error: str | None = None, bytes: int = 0, seconds: float = 0.0)`
  - `Ledger(reports_dir: Path)` with `.record(rec: Record) -> None`, `.histogram() -> dict[str, int]`, `.write_summary() -> Path`, `.counts(collection: str) -> dict[str, int]`

- [ ] **Step 1: Create the uv project**

`tools/catalog2cityparquet/pyproject.toml`:

```toml
[project]
name = "catalog2cityparquet"
version = "0.1.0"
description = "Convert the published City3D STAC catalogue into CityParquet packages"
requires-python = ">=3.11"
dependencies = [
    "httpx>=0.27",
    "duckdb>=1.0",
    "PyYAML>=6.0",
]

[project.optional-dependencies]
dev = ["pytest>=8.0"]

[build-system]
requires = ["hatchling"]
build-backend = "hatchling.build"

[tool.hatch.build.targets.wheel]
packages = ["src/catalog2cityparquet"]

[tool.pytest.ini_options]
testpaths = ["tests"]
```

```bash
mkdir -p tools/catalog2cityparquet/src/catalog2cityparquet tools/catalog2cityparquet/tests
touch tools/catalog2cityparquet/src/catalog2cityparquet/__init__.py
```

- [ ] **Step 2: Write the failing test**

`tools/catalog2cityparquet/tests/test_ledger.py`:

```python
import csv

import pytest

from catalog2cityparquet.ledger import REASONS, Ledger, Record


def test_records_are_appended_as_jsonl_per_collection(tmp_path):
    ledger = Ledger(tmp_path)
    ledger.record(Record("rotterdam-3d", "a", "converted", seconds=1.5, bytes=10))
    ledger.record(Record("rotterdam-3d", "b", "failed", reason="convert_failed", error="boom"))

    lines = (tmp_path / "rotterdam-3d.jsonl").read_text().strip().splitlines()
    assert len(lines) == 2


def test_histogram_counts_reasons_across_collections(tmp_path):
    ledger = Ledger(tmp_path)
    ledger.record(Record("a", "1", "failed", reason="no_crs"))
    ledger.record(Record("b", "2", "failed", reason="no_crs"))
    ledger.record(Record("b", "3", "failed", reason="download_failed"))

    assert ledger.histogram() == {"no_crs": 2, "download_failed": 1}


def test_summary_csv_rolls_up_per_collection(tmp_path):
    ledger = Ledger(tmp_path)
    ledger.record(Record("a", "1", "converted"))
    ledger.record(Record("a", "2", "failed", reason="no_crs"))
    ledger.record(Record("a", "3", "skipped", reason="duplicate_bundle"))

    path = ledger.write_summary()
    rows = list(csv.DictReader(path.open()))
    assert len(rows) == 1
    assert rows[0]["collection"] == "a"
    assert rows[0]["converted"] == "1"
    assert rows[0]["failed"] == "1"
    assert rows[0]["skipped"] == "1"


def test_an_unknown_reason_is_rejected(tmp_path):
    # The vocabulary is closed so the histogram stays meaningful; a typo must
    # fail loudly rather than create a new silent category.
    ledger = Ledger(tmp_path)
    with pytest.raises(ValueError, match="unknown reason"):
        ledger.record(Record("a", "1", "failed", reason="whoops"))


def test_the_vocabulary_is_the_documented_closed_set():
    assert REASONS == {
        "download_failed",
        "unsupported_archive",
        "unsupported_citygml_version",
        "unsupported_cityjson_version",
        "no_crs",
        "geographic_crs",
        "convert_failed",
        "empty_collection",
        "duplicate_bundle",
        "stale_item_index",
    }
```

- [ ] **Step 3: Run it to verify it fails**

Run: `cd tools/catalog2cityparquet && uv run --extra dev pytest tests/test_ledger.py -v`
Expected: FAIL — `ModuleNotFoundError: No module named 'catalog2cityparquet.ledger'`

- [ ] **Step 4: Implement the ledger**

`tools/catalog2cityparquet/src/catalog2cityparquet/ledger.py`:

```python
"""Per-item outcome records for a catalogue conversion run.

The run is a conformance measurement as much as a conversion: a failure is
data, not an abort. Every item lands here with a reason drawn from a closed
vocabulary, so the end-of-run histogram is comparable between runs.
"""

from __future__ import annotations

import csv
import json
from collections import Counter, defaultdict
from dataclasses import dataclass, field
from pathlib import Path
from typing import Literal

Status = Literal["converted", "failed", "skipped"]

#: Closed set. A reason outside it is a programming error, not a new category —
#: silently admitting typos would make the histogram meaningless.
REASONS = frozenset(
    {
        "download_failed",
        "unsupported_archive",
        "unsupported_citygml_version",
        "unsupported_cityjson_version",
        "no_crs",
        "geographic_crs",
        "convert_failed",
        "empty_collection",
        "duplicate_bundle",
        "stale_item_index",
    }
)


@dataclass(frozen=True)
class Record:
    collection: str
    item_id: str
    status: Status
    reason: str | None = None
    error: str | None = None
    bytes: int = 0
    seconds: float = 0.0


@dataclass
class Ledger:
    """Append-only JSONL per collection, plus a rolled-up summary."""

    reports_dir: Path
    _counts: dict[str, Counter] = field(default_factory=lambda: defaultdict(Counter))
    _reasons: Counter = field(default_factory=Counter)

    def __post_init__(self) -> None:
        self.reports_dir.mkdir(parents=True, exist_ok=True)

    def record(self, rec: Record) -> None:
        if rec.reason is not None and rec.reason not in REASONS:
            raise ValueError(f"unknown reason {rec.reason!r}; expected one of {sorted(REASONS)}")
        path = self.reports_dir / f"{rec.collection}.jsonl"
        with path.open("a", encoding="utf-8") as fh:
            fh.write(json.dumps(rec.__dict__, ensure_ascii=False) + "\n")
        self._counts[rec.collection][rec.status] += 1
        if rec.reason is not None:
            self._reasons[rec.reason] += 1

    def counts(self, collection: str) -> dict[str, int]:
        return dict(self._counts[collection])

    def histogram(self) -> dict[str, int]:
        return dict(self._reasons)

    def write_summary(self) -> Path:
        path = self.reports_dir / "summary.csv"
        with path.open("w", newline="", encoding="utf-8") as fh:
            writer = csv.writer(fh)
            writer.writerow(["collection", "converted", "failed", "skipped"])
            for collection in sorted(self._counts):
                c = self._counts[collection]
                writer.writerow(
                    [collection, c["converted"], c["failed"], c["skipped"]]
                )
        return path
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cd tools/catalog2cityparquet && uv run --extra dev pytest tests/test_ledger.py -v`
Expected: PASS (5 tests)

- [ ] **Step 6: Commit**

```bash
git add tools/catalog2cityparquet/
git commit -m "feat(catalog2cityparquet): uv project scaffold and outcome ledger"
```

---

### Task 6: Item discovery with index reconciliation

**Files:**
- Create: `tools/catalog2cityparquet/src/catalog2cityparquet/discover.py`
- Test: `tools/catalog2cityparquet/tests/test_discover.py`
- Test helper: `tools/catalog2cityparquet/tests/conftest.py`

**Interfaces:**
- Consumes: `ledger.Record`, `ledger.Ledger`.
- Produces:
  - `@dataclass(frozen=True) Item(collection: str, item_id: str, href: str, media_type: str | None, source_item_url: str | None)`
  - `collection_ids(base_url: str, client) -> list[str]`
  - `fetch_collection(base_url: str, cid: str, client) -> dict`
  - `list_item_objects(bucket_api: str, cid: str, client) -> list[str]` — **fully paginated**
  - `items_from_parquet(url: str) -> list[Item] | None` — `None` when absent/unreadable
  - `items_from_listing(base_url: str, bucket_api: str, cid: str, client) -> list[Item]`
  - `enumerate_items(...) -> tuple[list[Item], str | None]` — the second element is a discrepancy note when the index was stale

**This is where the run can go silently wrong.** Japan's `items.parquet` lists 306 of 60,471 items, and the GCS API caps a page at 1000. Both are tested directly.

- [ ] **Step 1: Write the failing tests**

`tools/catalog2cityparquet/tests/conftest.py`:

```python
"""A throwaway local HTTP server standing in for the GCS-hosted catalogue.

Tests never touch the network: every fixture below is served from a temp dir,
so the suite is deterministic and runnable offline.
"""

import json
import threading
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import pytest


@pytest.fixture
def served_dir(tmp_path):
    """Serve `tmp_path` over HTTP; yields (root_path, base_url)."""
    handler = partial(SimpleHTTPRequestHandler, directory=str(tmp_path))
    server = ThreadingHTTPServer(("127.0.0.1", 0), handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    host, port = server.server_address
    try:
        yield tmp_path, f"http://{host}:{port}"
    finally:
        server.shutdown()
        server.server_close()


def write_json(path: Path, payload) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload), encoding="utf-8")


def stac_item(item_id: str, href: str, media_type: str = "application/city+json") -> dict:
    return {
        "type": "Feature",
        "stac_version": "1.1.0",
        "id": item_id,
        "geometry": None,
        "bbox": None,
        "properties": {"datetime": None},
        "links": [],
        "assets": {
            "data": {"href": href, "type": media_type, "roles": ["data"]}
        },
    }
```

`tools/catalog2cityparquet/tests/test_discover.py`:

```python
import json

import httpx
import pytest

from catalog2cityparquet import discover
# pytest's default prepend import mode puts `tests/` on sys.path (it has no
# __init__.py), so the helpers are imported as top-level `conftest`, NOT as
# `tests.conftest` — the latter raises ModuleNotFoundError.
from conftest import stac_item, write_json


@pytest.fixture
def client():
    with httpx.Client(timeout=10) as c:
        yield c


def test_collection_ids_follow_child_links(served_dir, client):
    root, base = served_dir
    write_json(
        root / "catalog.json",
        {
            "type": "Catalog",
            "id": "c",
            "links": [
                {"rel": "child", "href": "./alpha/collection.json"},
                {"rel": "child", "href": "./beta/collection.json"},
                {"rel": "self", "href": "./catalog.json"},
            ],
        },
    )
    assert discover.collection_ids(base, client) == ["alpha", "beta"]


def test_listing_is_fully_paginated(served_dir, client, monkeypatch):
    # The GCS API caps a page at 1000 objects. Japan has 60,471 items, so a
    # driver that ignores nextPageToken converts 1.6% of it and reports
    # success. Two pages here prove the token is followed.
    root, base = served_dir
    write_json(
        root / "page1.json",
        {
            "items": [{"name": f"jp/items/{i}.json"} for i in range(3)],
            "nextPageToken": "TOK",
        },
    )
    write_json(root / "page2.json", {"items": [{"name": "jp/items/3.json"}]})

    calls = []

    def fake_get(url, **kwargs):
        calls.append(url)
        target = "page2.json" if "pageToken=TOK" in url else "page1.json"
        return client.get(f"{base}/{target}")

    names = discover.list_item_objects(
        bucket_api=f"{base}/o", cid="jp", client=type("C", (), {"get": staticmethod(fake_get)})()
    )
    assert names == [f"jp/items/{i}.json" for i in range(4)]
    assert len(calls) == 2, "the second page must be requested"


def test_stale_parquet_index_loses_to_the_listing(served_dir, client, monkeypatch):
    # japan-plateau-3d publishes an items.parquet listing 306 of its 60,471
    # items. Preferring the fast path there would silently convert 0.5%.
    root, base = served_dir
    listing_items = [stac_item(f"i{i}", f"{base}/data/i{i}.json") for i in range(4)]
    for item in listing_items:
        write_json(root / "jp" / "items" / f"{item['id']}.json", item)

    monkeypatch.setattr(
        discover, "items_from_parquet",
        lambda url: [discover.Item("jp", "i0", f"{base}/data/i0.json", None, None)],
    )
    monkeypatch.setattr(
        discover, "list_item_objects",
        lambda bucket_api, cid, client: [f"jp/items/i{i}.json" for i in range(4)],
    )

    items, note = discover.enumerate_items(
        base_url=base, bucket_api=f"{base}/o", cid="jp", collection={}, client=client
    )
    assert len(items) == 4, "the listing must win when the index disagrees"
    assert note is not None and "306" not in note
    assert "1" in note and "4" in note, f"the note must record both counts: {note}"


def test_matching_counts_use_the_fast_path(served_dir, client, monkeypatch):
    root, base = served_dir
    fast = [discover.Item("x", f"i{i}", f"{base}/d/i{i}.json", None, None) for i in range(2)]
    monkeypatch.setattr(discover, "items_from_parquet", lambda url: fast)
    monkeypatch.setattr(
        discover, "list_item_objects",
        lambda bucket_api, cid, client: ["x/items/i0.json", "x/items/i1.json"],
    )
    items, note = discover.enumerate_items(
        base_url=base, bucket_api=f"{base}/o", cid="x", collection={}, client=client
    )
    assert items == fast
    assert note is None


def test_an_empty_collection_yields_no_items(served_dir, client, monkeypatch):
    root, base = served_dir
    monkeypatch.setattr(discover, "items_from_parquet", lambda url: None)
    monkeypatch.setattr(discover, "list_item_objects", lambda bucket_api, cid, client: [])
    items, note = discover.enumerate_items(
        base_url=base, bucket_api=f"{base}/o", cid="empty",
        collection={"links": []}, client=client,
    )
    assert items == []
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cd tools/catalog2cityparquet && uv run --extra dev pytest tests/test_discover.py -v`
Expected: FAIL — `ModuleNotFoundError: No module named 'catalog2cityparquet.discover'`

- [ ] **Step 3: Implement discovery**

`tools/catalog2cityparquet/src/catalog2cityparquet/discover.py`:

```python
"""Walk the published catalogue: collections, then the items inside them.

Item enumeration has two independent sources — the collection's
`items.parquet` (fast: DuckDB range-reads it over HTTPS, no download) and the
object-store listing (slow but complete). Both are consulted and their counts
compared, because a published index can be badly out of date: at the time of
writing, `japan-plateau-3d` publishes an `items.parquet` describing 306 of its
60,471 items. Preferring the fast path unconditionally would convert one item
in two hundred and report success.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from urllib.parse import quote


@dataclass(frozen=True)
class Item:
    collection: str
    item_id: str
    href: str
    media_type: str | None
    source_item_url: str | None


def collection_ids(base_url: str, client) -> list[str]:
    catalog = client.get(f"{base_url}/catalog.json").json()
    ids = []
    for link in catalog.get("links", []):
        if link.get("rel") != "child":
            continue
        href = link.get("href", "")
        ids.append(href.removeprefix("./").removesuffix("/collection.json"))
    return ids


def fetch_collection(base_url: str, cid: str, client) -> dict:
    return client.get(f"{base_url}/{cid}/collection.json").json()


def list_item_objects(bucket_api: str, cid: str, client) -> list[str]:
    """Every object name under `<cid>/items/`, following every page.

    Pagination is mandatory, not an optimisation: the API caps a page at 1000
    objects and the largest collection has 60,471 items.
    """
    names: list[str] = []
    token: str | None = None
    while True:
        url = f"{bucket_api}?prefix={quote(cid + '/items/')}&maxResults=1000"
        if token:
            url += f"&pageToken={token}"
        payload = client.get(url).json()
        names.extend(obj["name"] for obj in payload.get("items", []))
        token = payload.get("nextPageToken")
        if not token:
            return names


def items_from_parquet(url: str) -> list[Item] | None:
    """Read a collection's stac-geoparquet index remotely, or `None`.

    `assets` is a struct whose keys differ per collection, so it is read
    generically as JSON rather than by a fixed schema.
    """
    import duckdb

    try:
        rows = duckdb.sql(
            f"""
            LOAD httpfs;
            SELECT id,
                   json_extract_string(to_json(assets), '$.data.href') AS href,
                   json_extract_string(to_json(assets), '$.data.type') AS media_type,
                   collection
            FROM read_parquet('{url}')
            """
        ).fetchall()
    except Exception:
        return None
    return [
        Item(collection=r[3] or "", item_id=r[0], href=r[1], media_type=r[2], source_item_url=None)
        for r in rows
        if r[1]
    ]


def items_from_listing(base_url: str, bucket_api: str, cid: str, client) -> list[Item]:
    items: list[Item] = []
    for name in list_item_objects(bucket_api, cid, client):
        if not name.endswith(".json"):
            continue
        url = f"{base_url}/{name}"
        try:
            doc = client.get(url).json()
        except Exception:
            continue
        asset = (doc.get("assets") or {}).get("data") or {}
        href = asset.get("href")
        if not href:
            continue
        items.append(
            Item(
                collection=cid,
                item_id=doc.get("id") or name.rsplit("/", 1)[-1].removesuffix(".json"),
                href=href,
                media_type=asset.get("type"),
                source_item_url=url,
            )
        )
    return items


def items_from_collection_links(base_url: str, cid: str, collection: dict, client) -> list[Item]:
    items: list[Item] = []
    for link in collection.get("links", []):
        if link.get("rel") != "item":
            continue
        url = f"{base_url}/{cid}/{link['href'].removeprefix('./')}"
        try:
            doc = client.get(url).json()
        except Exception:
            continue
        asset = (doc.get("assets") or {}).get("data") or {}
        if asset.get("href"):
            items.append(
                Item(cid, doc.get("id", ""), asset["href"], asset.get("type"), url)
            )
    return items


def enumerate_items(
    base_url: str, bucket_api: str, cid: str, collection: dict, client
) -> tuple[list[Item], str | None]:
    """Return this collection's items, plus a note when the index was stale.

    Policy: use `items.parquet` only when its row count agrees with the object
    listing. On any disagreement the listing wins and the discrepancy is
    reported, so a stale index can never silently truncate a run.
    """
    fast = items_from_parquet(f"{base_url}/{cid}/items.parquet")
    listed_names = list_item_objects(bucket_api, cid, client)
    listed_count = sum(1 for n in listed_names if n.endswith(".json"))

    if fast is not None and listed_count and len(fast) == listed_count:
        return fast, None

    if listed_count:
        note = None
        if fast is not None and len(fast) != listed_count:
            note = (
                f"stale item index: items.parquet lists {len(fast)} item(s) "
                f"but the object listing has {listed_count}; using the listing"
            )
        return items_from_listing(base_url, bucket_api, cid, client), note

    if fast:
        return fast, None
    return items_from_collection_links(base_url, cid, collection, client), None
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd tools/catalog2cityparquet && uv run --extra dev pytest tests/test_discover.py -v`
Expected: PASS (5 tests)

- [ ] **Step 5: Verify against the real catalogue**

```bash
cd tools/catalog2cityparquet
uv run python -c "
import httpx
from catalog2cityparquet import discover
c = httpx.Client(timeout=60, follow_redirects=True)
base='https://storage.googleapis.com/city3d-stac'
api='https://storage.googleapis.com/storage/v1/b/city3d-stac/o'
print('collections:', len(discover.collection_ids(base, c)))
print('jp objects:', len(discover.list_item_objects(api,'japan-plateau-3d',c)))
"
```
Expected: `collections: 53` and `jp objects: 60471` (proving pagination works against the real API).

- [ ] **Step 6: Commit**

```bash
git add tools/catalog2cityparquet/
git commit -m "feat(catalog2cityparquet): item discovery with index reconciliation

items.parquet is used only when its count agrees with the object listing:
japan-plateau-3d's index lists 306 of 60,471 items, so trusting it silently
converts 0.5% of the collection. Listing pagination is mandatory."
```

---

### Task 7: Fetch, sniff and normalise

**Files:**
- Create: `tools/catalog2cityparquet/src/catalog2cityparquet/fetch.py`
- Test: `tools/catalog2cityparquet/tests/test_fetch.py`

**Interfaces:**
- Consumes: `discover.Item`.
- Produces:
  - `CONVERTIBLE_SUFFIXES: frozenset[str]` = `{".json", ".jsonl", ".gml", ".xml"}`
  - `sniff(head: bytes) -> str` — `"zip" | "gzip" | "plain"`
  - `download(url: str, dest: Path, client, timeout: float) -> int` — returns bytes written
  - `normalise(path: Path, workdir: Path, max_depth: int = 3, max_bytes: int = 20 * 2**30) -> list[Path]`
  - `is_duplicate_bundle(item: Item) -> bool`

- [ ] **Step 1: Write the failing tests**

`tools/catalog2cityparquet/tests/test_fetch.py`:

```python
import gzip
import zipfile
from pathlib import Path

import pytest

from catalog2cityparquet import fetch
from catalog2cityparquet.discover import Item

FIXTURES = Path(__file__).resolve().parents[3] / "tests" / "fixtures"


def test_sniff_recognises_zip_gzip_and_plain():
    assert fetch.sniff(b"PK\x03\x04rest") == "zip"
    assert fetch.sniff(b"\x1f\x8b\x08rest") == "gzip"
    assert fetch.sniff(b'{"type":"CityJSON"}') == "plain"


def test_a_lying_media_type_does_not_fool_normalise(tmp_path):
    # hamburg-3d advertises application/gml+xml with a .GML extension and
    # serves a 468 MB ZIP. Detection must be by content, never by the declared
    # type or the extension.
    src = FIXTURES / "b1_lod2_s.gml"
    archive = tmp_path / "lying.GML"          # extension says GML
    with zipfile.ZipFile(archive, "w") as zf:
        zf.write(src, "inner.gml")            # content is a ZIP

    found = fetch.normalise(archive, tmp_path / "work")
    assert [p.name for p in found] == ["inner.gml"]


def test_gzip_is_decompressed(tmp_path):
    src = FIXTURES / "delft.city.jsonl"
    packed = tmp_path / "t.city.json.gz"
    packed.write_bytes(gzip.compress(src.read_bytes()))

    found = fetch.normalise(packed, tmp_path / "work")
    assert len(found) == 1
    assert found[0].read_bytes() == src.read_bytes()


def test_nested_archives_are_extracted(tmp_path):
    # geobremen-bremen is a zip inside a zip; single-level extraction finds
    # nothing convertible.
    src = FIXTURES / "b1_lod2_s.gml"
    inner = tmp_path / "inner.zip"
    with zipfile.ZipFile(inner, "w") as zf:
        zf.write(src, "model.gml")
    outer = tmp_path / "outer.zip"
    with zipfile.ZipFile(outer, "w") as zf:
        zf.write(inner, "inner.zip")

    found = fetch.normalise(outer, tmp_path / "work")
    assert [p.name for p in found] == ["model.gml"]


def test_many_members_are_all_returned(tmp_path):
    # Japan's whole-city packages hold 136 GMLs that must be passed to one
    # convert invocation.
    src = FIXTURES / "b1_lod2_s.gml"
    archive = tmp_path / "bundle.zip"
    with zipfile.ZipFile(archive, "w") as zf:
        for i in range(5):
            zf.write(src, f"udx/bldg/{i}.gml")
        zf.writestr("codelists/ignore.xsd", "not convertible")
        zf.writestr("metadata/readme.pdf", "not convertible")

    found = fetch.normalise(archive, tmp_path / "work")
    assert len(found) == 5
    assert all(p.suffix == ".gml" for p in found)


def test_path_traversal_members_are_rejected(tmp_path):
    archive = tmp_path / "evil.zip"
    with zipfile.ZipFile(archive, "w") as zf:
        zf.writestr("../escape.gml", "<x/>")

    with pytest.raises(ValueError, match="unsafe member"):
        fetch.normalise(archive, tmp_path / "work")


def test_an_oversized_archive_is_refused(tmp_path):
    archive = tmp_path / "bomb.zip"
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as zf:
        zf.writestr("big.gml", "0" * (1 << 20))

    with pytest.raises(ValueError, match="uncompressed size"):
        fetch.normalise(archive, tmp_path / "work", max_bytes=1024)


def test_an_archive_with_nothing_convertible_returns_empty(tmp_path):
    archive = tmp_path / "none.zip"
    with zipfile.ZipFile(archive, "w") as zf:
        zf.writestr("readme.txt", "hello")

    assert fetch.normalise(archive, tmp_path / "work") == []


def test_japan_whole_city_bundles_are_recognised():
    # The 381 *_citygml_* items repackage tiles converted separately.
    bundle = Item("japan-plateau-3d", "11348_hatoyama-machi_pref_2025_citygml_1_op", "u", None, None)
    tile = Item("japan-plateau-3d", "48395630_bldg_6697_op", "u", None, None)
    other = Item("rotterdam-3d", "x_citygml_y", "u", None, None)

    assert fetch.is_duplicate_bundle(bundle) is True
    assert fetch.is_duplicate_bundle(tile) is False
    assert fetch.is_duplicate_bundle(other) is False
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cd tools/catalog2cityparquet && uv run --extra dev pytest tests/test_fetch.py -v`
Expected: FAIL — `ModuleNotFoundError: No module named 'catalog2cityparquet.fetch'`

- [ ] **Step 3: Implement fetching and normalisation**

`tools/catalog2cityparquet/src/catalog2cityparquet/fetch.py`:

```python
"""Get an item's source bytes onto disk in a form the converter accepts.

Everything here is defensive because the catalogue's hosts are not: media
types are wrong (one collection advertises `application/gml+xml` and serves a
468 MB ZIP), archives nest (one is a zip inside a zip), one origin 403s
without a browser User-Agent, one serves query-string URLs with no filename,
and PLATEAU responses omit Content-Length entirely. Format is therefore
decided by magic bytes and nothing else.
"""

from __future__ import annotations

import gzip
import shutil
import zipfile
from pathlib import Path

from .discover import Item

CONVERTIBLE_SUFFIXES = frozenset({".json", ".jsonl", ".gml", ".xml"})

#: Some origins refuse a default client (montreal-3d returns 403).
USER_AGENT = (
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) "
    "Chrome/124.0 Safari/537.36"
)


def sniff(head: bytes) -> str:
    """Classify by magic bytes. Never trust a declared media type."""
    if head.startswith(b"PK\x03\x04"):
        return "zip"
    if head.startswith(b"\x1f\x8b"):
        return "gzip"
    return "plain"


def download(url: str, dest: Path, client, timeout: float = 900.0) -> int:
    """Stream `url` to `dest`, returning the byte count."""
    dest.parent.mkdir(parents=True, exist_ok=True)
    written = 0
    with client.stream(
        "GET", url, timeout=timeout, follow_redirects=True,
        headers={"User-Agent": USER_AGENT},
    ) as response:
        response.raise_for_status()
        with dest.open("wb") as fh:
            for chunk in response.iter_bytes(1 << 20):
                fh.write(chunk)
                written += len(chunk)
    return written


def is_duplicate_bundle(item: Item) -> bool:
    """True for Japan's whole-city ZIPs, which repackage tiles we convert.

    The 381 `*_citygml_*` items contain the same data as the 60,090 per-module
    tile items; converting both would encode Japan twice.
    """
    return item.collection == "japan-plateau-3d" and "_citygml_" in item.item_id


def _safe_extract(archive: Path, into: Path, max_bytes: int) -> list[Path]:
    into.mkdir(parents=True, exist_ok=True)
    extracted: list[Path] = []
    with zipfile.ZipFile(archive) as zf:
        total = sum(info.file_size for info in zf.infolist())
        if total > max_bytes:
            raise ValueError(
                f"{archive.name}: uncompressed size {total} exceeds the {max_bytes} limit"
            )
        for info in zf.infolist():
            if info.is_dir():
                continue
            target = (into / info.filename).resolve()
            if not str(target).startswith(str(into.resolve())):
                raise ValueError(f"{archive.name}: unsafe member {info.filename!r}")
            target.parent.mkdir(parents=True, exist_ok=True)
            with zf.open(info) as src, target.open("wb") as dst:
                shutil.copyfileobj(src, dst)
            extracted.append(target)
    return extracted


def normalise(
    path: Path, workdir: Path, max_depth: int = 3, max_bytes: int = 20 * 2**30
) -> list[Path]:
    """Decompress/extract `path` and return the convertible files inside.

    Recurses into nested archives up to `max_depth`. The result may hold many
    files: `cityparquet convert` accepts several inputs and merges them, which
    is what a multi-tile archive needs.
    """
    workdir.mkdir(parents=True, exist_ok=True)
    pending: list[tuple[Path, int]] = [(path, 0)]
    found: list[Path] = []

    while pending:
        current, depth = pending.pop()
        kind = sniff(current.open("rb").read(4))

        if kind == "zip":
            if depth >= max_depth:
                continue
            for member in _safe_extract(current, workdir / f"x{depth}_{current.stem}", max_bytes):
                pending.append((member, depth + 1))
            continue

        if kind == "gzip":
            if depth >= max_depth:
                continue
            name = current.name.removesuffix(".gz") or "decompressed"
            target = workdir / f"g{depth}_{name}"
            with gzip.open(current, "rb") as src, target.open("wb") as dst:
                shutil.copyfileobj(src, dst)
            pending.append((target, depth + 1))
            continue

        if current.suffix.lower() in CONVERTIBLE_SUFFIXES:
            found.append(current)

    return sorted(found)
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd tools/catalog2cityparquet && uv run --extra dev pytest tests/test_fetch.py -v`
Expected: PASS (9 tests)

- [ ] **Step 5: Commit**

```bash
git add tools/catalog2cityparquet/
git commit -m "feat(catalog2cityparquet): content-sniffing fetch and archive normalisation

Format is decided by magic bytes: one collection advertises gml+xml and
serves a ZIP. Handles nested archives, guards path traversal and zip bombs,
and skips Japan's whole-city bundles that repackage tiles."
```

---

### Task 8: Convert and stamp

**Files:**
- Create: `tools/catalog2cityparquet/src/catalog2cityparquet/convert.py`
- Test: `tools/catalog2cityparquet/tests/test_convert.py`

**Interfaces:**
- Consumes: `discover.Item`.
- Produces:
  - `class ConvertError(RuntimeError)` with `.reason: str` (a ledger vocabulary member) and `.detail: str`
  - `run_convert(binary: Path, inputs: list[Path], out_dir: Path, crs: str | None, timeout: float) -> int` — returns the object count; raises `ConvertError`
  - `classify_error(stderr: str) -> str` — maps converter stderr to a ledger reason
  - `stamp(pkg_dir: Path, item: Item) -> None`

- [ ] **Step 1: Write the failing tests**

`tools/catalog2cityparquet/tests/test_convert.py`:

```python
import json
from pathlib import Path

import pytest

from catalog2cityparquet import convert
from catalog2cityparquet.discover import Item


def test_converter_errors_map_to_ledger_reasons():
    # The driver must classify failures, because "it failed" is not a finding.
    # These strings are the converter's real messages.
    assert convert.classify_error(
        'unsupported CityGML version 1.0 (only CityGML 2.0 is supported)'
    ) == "unsupported_citygml_version"
    assert convert.classify_error(
        "CityGML srsName \"EPSG:4979\" resolves to geographic CRS 4979"
    ) == "geographic_crs"
    assert convert.classify_error(
        "source carries a CRS-bearing coordinate but declares no CRS a writer can resolve"
    ) == "no_crs"
    assert convert.classify_error(
        "invalid CityJSON: invalid type: integer `1`, expected a string"
    ) == "unsupported_cityjson_version"
    assert convert.classify_error("something else entirely") == "convert_failed"


def test_stamp_adds_collection_and_links_without_touching_properties(tmp_path):
    pkg = tmp_path / "pkg"
    pkg.mkdir()
    original = {
        "type": "Feature",
        "stac_version": "1.1.0",
        "id": "tile-1",
        "properties": {"city3d:city_objects": 873, "proj:code": "EPSG:7415"},
        "links": [],
        "assets": {"data": {"href": "./building.parquet"}},
    }
    (pkg / "metadata.json").write_text(json.dumps(original))

    item = Item(
        collection="netherlands-3d-bag",
        item_id="tile-1",
        href="https://data.3dbag.nl/x.city.json.gz",
        media_type="application/city+json",
        source_item_url="https://storage.googleapis.com/city3d-stac/netherlands-3d-bag/items/tile-1.json",
    )
    convert.stamp(pkg, item)

    written = json.loads((pkg / "metadata.json").read_text())
    assert written["collection"] == "netherlands-3d-bag"
    rels = {link["rel"]: link["href"] for link in written["links"]}
    assert rels["collection"] == "../../collection.json"
    assert rels["parent"] == "../../collection.json"
    assert rels["root"] == "../../../catalog.json"
    assert rels["via"] == item.href
    assert rels["derived_from"] == item.source_item_url
    # Footer-derived properties are authoritative; the driver must not edit them.
    assert written["properties"] == original["properties"]


def test_stamp_is_idempotent(tmp_path):
    # A resumed run may re-stamp; links must not accumulate duplicates.
    pkg = tmp_path / "pkg"
    pkg.mkdir()
    (pkg / "metadata.json").write_text(
        json.dumps({"type": "Feature", "id": "a", "properties": {}, "links": [], "assets": {}})
    )
    item = Item("c", "a", "https://h/x", None, "https://s/i.json")
    convert.stamp(pkg, item)
    convert.stamp(pkg, item)
    written = json.loads((pkg / "metadata.json").read_text())
    assert len(written["links"]) == len({link["rel"] for link in written["links"]})


def test_run_convert_raises_a_classified_error(tmp_path):
    fake = tmp_path / "fake-cityparquet"
    fake.write_text(
        "#!/bin/sh\n"
        "echo 'error: schema error: unsupported CityGML version 1.0 "
        "(only CityGML 2.0 is supported)' >&2\n"
        "exit 1\n"
    )
    fake.chmod(0o755)

    with pytest.raises(convert.ConvertError) as excinfo:
        convert.run_convert(fake, [tmp_path / "in.gml"], tmp_path / "out", None, timeout=30)
    assert excinfo.value.reason == "unsupported_citygml_version"


def test_run_convert_returns_the_object_count(tmp_path):
    fake = tmp_path / "fake-cityparquet"
    fake.write_text("#!/bin/sh\necho '2231 2 0 0 0 0 0 0 0'\nexit 0\n")
    fake.chmod(0o755)

    count = convert.run_convert(fake, [tmp_path / "in.json"], tmp_path / "out", None, timeout=30)
    assert count == 2231
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cd tools/catalog2cityparquet && uv run --extra dev pytest tests/test_convert.py -v`
Expected: FAIL — `ModuleNotFoundError: No module named 'catalog2cityparquet.convert'`

- [ ] **Step 3: Implement conversion and stamping**

`tools/catalog2cityparquet/src/catalog2cityparquet/convert.py`:

```python
"""Invoke the converter and finish the STAC Item it emits.

`cityparquet convert` already writes `metadata.json` as a STAC Item derived
from the Parquet footer. The driver adds only what a single package cannot
know: which collection it belongs to, and where it came from. Footer-derived
properties are never edited — the spec makes the footer authoritative where
Item and footer disagree.
"""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

from .discover import Item


class ConvertError(RuntimeError):
    def __init__(self, reason: str, detail: str) -> None:
        super().__init__(detail)
        self.reason = reason
        self.detail = detail


def classify_error(stderr: str) -> str:
    """Map converter stderr to a ledger reason.

    Classification is the point of the run: an unclassified pile of failures
    measures nothing.
    """
    text = stderr.lower()
    if "unsupported citygml version" in text:
        return "unsupported_citygml_version"
    if "geographic crs" in text:
        return "geographic_crs"
    if "declares no crs" in text:
        return "no_crs"
    if "invalid cityjson" in text:
        return "unsupported_cityjson_version"
    return "convert_failed"


def run_convert(
    binary: Path, inputs: list[Path], out_dir: Path, crs: str | None, timeout: float
) -> int:
    """Convert `inputs` into `out_dir`; return the city-object count."""
    cmd = [str(binary), "convert", *[str(p) for p in inputs], "-o", str(out_dir), "--overwrite"]
    if crs:
        cmd += ["--crs", crs]
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired as exc:
        raise ConvertError("convert_failed", f"timed out after {timeout}s") from exc
    if proc.returncode != 0:
        raise ConvertError(classify_error(proc.stderr), proc.stderr.strip()[:2000])
    first = proc.stdout.split()
    return int(first[0]) if first and first[0].isdigit() else 0


def stamp(pkg_dir: Path, item: Item) -> None:
    """Add collection membership and provenance to the emitted Item.

    Idempotent: a resumed run may re-stamp a package, and duplicated links
    would corrupt the aggregated collection.
    """
    path = pkg_dir / "metadata.json"
    doc = json.loads(path.read_text(encoding="utf-8"))
    doc["collection"] = item.collection

    links = [link for link in doc.get("links", []) if link.get("rel") not in
             {"collection", "parent", "root", "via", "derived_from"}]
    links.append({"rel": "collection", "href": "../../collection.json", "type": "application/json"})
    links.append({"rel": "parent", "href": "../../collection.json", "type": "application/json"})
    links.append({"rel": "root", "href": "../../../catalog.json", "type": "application/json"})
    if item.href:
        links.append({"rel": "via", "href": item.href})
    if item.source_item_url:
        links.append({"rel": "derived_from", "href": item.source_item_url,
                      "type": "application/json"})
    doc["links"] = links

    path.write_text(json.dumps(doc, indent=2, ensure_ascii=False), encoding="utf-8")
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd tools/catalog2cityparquet && uv run --extra dev pytest tests/test_convert.py -v`
Expected: PASS (5 tests)

- [ ] **Step 5: Commit**

```bash
git add tools/catalog2cityparquet/
git commit -m "feat(catalog2cityparquet): convert invocation, error classification and Item stamping"
```

---

### Task 9: Aggregation into collections and a catalog

**Files:**
- Create: `tools/catalog2cityparquet/src/catalog2cityparquet/aggregate.py`
- Test: `tools/catalog2cityparquet/tests/test_aggregate.py`

**Interfaces:**
- Consumes: nothing from earlier tasks beyond paths.
- Produces:
  - `collection_config(collection_json: dict) -> dict` — the `CollectionConfigFile` shape
  - `write_config(config: dict, dest: Path) -> Path`
  - `update_collection(tool: Path, items_dir: Path, config: Path, out: Path, geoparquet: bool = True) -> None`
  - `update_catalog(tool: Path, collection_jsons: list[Path], out_dir: Path, config: Path) -> None`

- [ ] **Step 1: Write the failing tests**

`tools/catalog2cityparquet/tests/test_aggregate.py`:

```python
import yaml

from catalog2cityparquet import aggregate


def test_config_is_derived_from_the_published_collection():
    # No registry dependency: the collection.json fetched during traversal is
    # the metadata seed, so ids always match.
    published = {
        "id": "rotterdam-3d",
        "title": "Rotterdam 3D City Model",
        "description": "3D LoD2 city model of Rotterdam.",
        "license": "other",
        "keywords": ["3d city model", "buildings"],
        "providers": [{"name": "Municipality of Rotterdam", "roles": ["producer"]}],
        "links": [
            {"rel": "source", "href": "https://data.rotterdam.nl/", "type": "text/html"},
            {"rel": "self", "href": "./collection.json"},
            {"rel": "item", "href": "./items/x.json"},
        ],
        "extent": {"spatial": {"bbox": [[0, 0, 0, 1, 1, 1]]}},
    }
    config = aggregate.collection_config(published)

    assert config["id"] == "rotterdam-3d"
    assert config["title"] == "Rotterdam 3D City Model"
    assert config["license"] == "other"
    assert config["keywords"] == ["3d city model", "buildings"]
    assert config["providers"][0]["name"] == "Municipality of Rotterdam"
    # Structural links belong to the generated tree, not the config: carrying
    # them over would point the mirror at the source catalogue's items.
    rels = {link["rel"] for link in config["links"]}
    assert rels == {"source"}
    # Extent is recomputed by the tool from the generated items.
    assert "extent" not in config


def test_config_round_trips_through_yaml(tmp_path):
    config = aggregate.collection_config({"id": "x", "description": "d", "license": "CC-BY-4.0"})
    path = aggregate.write_config(config, tmp_path / "x.yaml")
    assert yaml.safe_load(path.read_text())["license"] == "CC-BY-4.0"


def test_missing_optional_fields_are_omitted_not_nulled():
    config = aggregate.collection_config({"id": "x", "description": "d"})
    assert "keywords" not in config
    assert "providers" not in config
    assert config["id"] == "x"
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cd tools/catalog2cityparquet && uv run --extra dev pytest tests/test_aggregate.py -v`
Expected: FAIL — `ModuleNotFoundError: No module named 'catalog2cityparquet.aggregate'`

- [ ] **Step 3: Implement aggregation**

`tools/catalog2cityparquet/src/catalog2cityparquet/aggregate.py`:

```python
"""Turn converted packages into a collection, and collections into a catalogue.

The published `collection.json` is the metadata seed: it is already fetched
during traversal and its id always matches. It is translated into the shape
`city3dstac`'s `--config` already accepts, so no tool change is needed for
metadata — only `--items-dir`.

Extent and summaries are deliberately NOT carried over: the tool recomputes
them from the generated items, so they describe the CityParquet mirror rather
than the source catalogue.
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import yaml

#: Links that describe the generated tree rather than the dataset. Carrying
#: these over would point the mirror's collection at the source catalogue.
STRUCTURAL_RELS = frozenset({"self", "root", "parent", "item", "child", "collection"})

_CARRIED = ("id", "title", "description", "license", "keywords", "providers")


def collection_config(collection_json: dict) -> dict:
    config = {key: collection_json[key] for key in _CARRIED if collection_json.get(key) is not None}
    links = [
        link
        for link in collection_json.get("links", [])
        if link.get("rel") not in STRUCTURAL_RELS
    ]
    if links:
        config["links"] = links
    return config


def catalog_config(catalog_json: dict) -> dict:
    return {
        key: catalog_json[key]
        for key in ("id", "title", "description")
        if catalog_json.get(key) is not None
    }


def write_config(config: dict, dest: Path) -> Path:
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_text(yaml.safe_dump(config, sort_keys=False, allow_unicode=True), encoding="utf-8")
    return dest


def update_collection(
    tool: Path, items_dir: Path, config: Path, out: Path, geoparquet: bool = True
) -> None:
    cmd = [
        str(tool), "update-collection",
        "--items-dir", str(items_dir),
        "--config", str(config),
        "-o", str(out),
    ]
    if geoparquet:
        cmd.append("--geoparquet")
    proc = subprocess.run(cmd, capture_output=True, text=True)
    if proc.returncode != 0:
        raise RuntimeError(f"update-collection failed: {proc.stderr.strip()[:2000]}")


def update_catalog(
    tool: Path, collection_jsons: list[Path], out_dir: Path, config: Path
) -> None:
    cmd = [str(tool), "update-catalog", *[str(p) for p in collection_jsons],
           "-o", str(out_dir), "--config", str(config)]
    proc = subprocess.run(cmd, capture_output=True, text=True)
    if proc.returncode != 0:
        raise RuntimeError(f"update-catalog failed: {proc.stderr.strip()[:2000]}")
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd tools/catalog2cityparquet && uv run --extra dev pytest tests/test_aggregate.py -v`
Expected: PASS (3 tests)

- [ ] **Step 5: Commit**

```bash
git add tools/catalog2cityparquet/
git commit -m "feat(catalog2cityparquet): collection/catalog aggregation via city3dstac"
```

---

### Task 10: CLI orchestration and failure isolation

**Files:**
- Create: `tools/catalog2cityparquet/src/catalog2cityparquet/__main__.py`
- Test: `tools/catalog2cityparquet/tests/test_orchestration.py`

**Interfaces:**
- Consumes: every module above.
- Produces: `python -m catalog2cityparquet` with flags `--out`, `--collection` (repeatable), `--limit-per-collection`, `--jobs`, `--keep-downloads`, `--no-skip-existing`, `--crs` (repeatable `COLLECTION=EPSG:xxxx`), `--binary`, `--tool`, `--base-url`, `--bucket-api`. Also `convert_collection(...)` and `run(...)` as importable functions.

**The central requirement:** a collection that fails must not stop the next one, and an item that fails must not stop the next item.

- [ ] **Step 1: Write the failing tests**

`tools/catalog2cityparquet/tests/test_orchestration.py`:

```python
import pytest

from catalog2cityparquet import __main__ as driver
from catalog2cityparquet.discover import Item
from catalog2cityparquet.ledger import Ledger


def test_a_failing_collection_does_not_stop_the_next(tmp_path, monkeypatch):
    # The brief's hard requirement: "if generation fails on a particular
    # collection, skip and go to the next; we don't terminate."
    attempted = []

    def fake_convert_collection(cid, **kwargs):
        attempted.append(cid)
        if cid == "boom":
            raise RuntimeError("collection exploded")

    monkeypatch.setattr(driver, "convert_collection", fake_convert_collection)

    ledger = Ledger(tmp_path / "_reports")
    driver.run_collections(["alpha", "boom", "omega"], ledger=ledger, config=driver.Config(
        out=tmp_path, binary=tmp_path / "b", tool=tmp_path / "t",
    ))

    assert attempted == ["alpha", "boom", "omega"], "every collection must be attempted"


def test_a_failing_item_does_not_stop_the_next(tmp_path, monkeypatch):
    processed = []

    def fake_process(item, **kwargs):
        processed.append(item.item_id)
        if item.item_id == "bad":
            raise RuntimeError("item exploded")
        return 1

    monkeypatch.setattr(driver, "process_item", fake_process)
    items = [Item("c", i, "u", None, None) for i in ("good1", "bad", "good2")]
    ledger = Ledger(tmp_path / "_reports")

    driver.convert_items(items, ledger=ledger, config=driver.Config(
        out=tmp_path, binary=tmp_path / "b", tool=tmp_path / "t", jobs=1,
    ))

    assert processed == ["good1", "bad", "good2"]
    counts = ledger.counts("c")
    assert counts["converted"] == 2
    assert counts["failed"] == 1


def test_already_converted_items_are_skipped_on_resume(tmp_path, monkeypatch):
    import json

    out = tmp_path / "out"
    pkg = out / "c" / "items" / "done"
    pkg.mkdir(parents=True)
    (pkg / "metadata.json").write_text(
        json.dumps({"type": "Feature", "stac_version": "1.1.0", "id": "done"})
    )

    processed = []
    monkeypatch.setattr(driver, "process_item", lambda item, **k: processed.append(item.item_id))

    items = [Item("c", "done", "u", None, None), Item("c", "todo", "u", None, None)]
    driver.convert_items(items, ledger=Ledger(tmp_path / "_reports"), config=driver.Config(
        out=out, binary=tmp_path / "b", tool=tmp_path / "t", jobs=1, skip_existing=True,
    ))

    assert processed == ["todo"], "a package with a valid Item must not be redone"


def test_duplicate_bundles_are_skipped_before_download(tmp_path, monkeypatch):
    processed = []
    monkeypatch.setattr(driver, "process_item", lambda item, **k: processed.append(item.item_id))

    items = [
        Item("japan-plateau-3d", "x_citygml_1_op", "u", None, None),
        Item("japan-plateau-3d", "48395630_bldg_6697_op", "u", None, None),
    ]
    ledger = Ledger(tmp_path / "_reports")
    driver.convert_items(items, ledger=ledger, config=driver.Config(
        out=tmp_path, binary=tmp_path / "b", tool=tmp_path / "t", jobs=1,
    ))

    assert processed == ["48395630_bldg_6697_op"]
    assert ledger.histogram()["duplicate_bundle"] == 1
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cd tools/catalog2cityparquet && uv run --extra dev pytest tests/test_orchestration.py -v`
Expected: FAIL — `AttributeError: module 'catalog2cityparquet.__main__' has no attribute 'run_collections'`

- [ ] **Step 3: Implement the orchestrator**

`tools/catalog2cityparquet/src/catalog2cityparquet/__main__.py`:

```python
"""Drive the whole conversion: catalogue in, CityParquet mirror out.

Failure isolation is the design centre. An item that fails is recorded and the
next one starts; a collection that fails is recorded and the next one starts.
The process exits non-zero only when the catalogue root itself is unreachable —
a run with failures is a successful run that measured failures.
"""

from __future__ import annotations

import argparse
import json
import shutil
import sys
import tempfile
import time
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field
from pathlib import Path

import httpx

from . import aggregate, convert, discover, fetch
from .discover import Item
from .ledger import Ledger, Record

BASE_URL = "https://storage.googleapis.com/city3d-stac"
BUCKET_API = "https://storage.googleapis.com/storage/v1/b/city3d-stac/o"


@dataclass
class Config:
    out: Path
    binary: Path
    tool: Path
    jobs: int = 8
    skip_existing: bool = True
    keep_downloads: bool = False
    limit_per_collection: int | None = None
    crs_by_collection: dict[str, str] = field(default_factory=dict)
    base_url: str = BASE_URL
    bucket_api: str = BUCKET_API
    download_timeout: float = 1800.0
    convert_timeout: float = 3600.0


def package_dir(config: Config, item: Item) -> Path:
    return config.out / item.collection / "items" / item.item_id


def already_converted(config: Config, item: Item) -> bool:
    path = package_dir(config, item) / "metadata.json"
    if not path.is_file():
        return False
    try:
        doc = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return False
    return doc.get("type") == "Feature" and "stac_version" in doc


def process_item(item: Item, *, config: Config, client: httpx.Client) -> int:
    """Download, normalise, convert and stamp one item. Returns object count."""
    workdir = Path(tempfile.mkdtemp(prefix="c2cp-"))
    try:
        source = workdir / "download.bin"
        fetch.download(item.href, source, client, timeout=config.download_timeout)
        inputs = fetch.normalise(source, workdir / "extract")
        if not inputs:
            raise convert.ConvertError("unsupported_archive", "no convertible file in the asset")
        out_dir = package_dir(config, item)
        count = convert.run_convert(
            config.binary, inputs, out_dir,
            config.crs_by_collection.get(item.collection),
            timeout=config.convert_timeout,
        )
        convert.stamp(out_dir, item)
        return count
    finally:
        if not config.keep_downloads:
            shutil.rmtree(workdir, ignore_errors=True)


def convert_items(items: list[Item], *, ledger: Ledger, config: Config) -> None:
    """Convert every item, isolating each failure to its own record."""
    client = httpx.Client(timeout=config.download_timeout, follow_redirects=True)

    def handle(item: Item) -> None:
        if fetch.is_duplicate_bundle(item):
            ledger.record(Record(item.collection, item.item_id, "skipped",
                                 reason="duplicate_bundle"))
            return
        if config.skip_existing and already_converted(config, item):
            return
        started = time.monotonic()
        try:
            process_item(item, config=config, client=client)
        except convert.ConvertError as exc:
            ledger.record(Record(item.collection, item.item_id, "failed",
                                 reason=exc.reason, error=exc.detail,
                                 seconds=time.monotonic() - started))
        except (httpx.HTTPError, OSError) as exc:
            # Transport and filesystem problems are the origin's fault, not the
            # converter's; keeping them a separate reason stops upstream
            # flakiness from inflating the converter's failure count.
            ledger.record(Record(item.collection, item.item_id, "failed",
                                 reason="download_failed", error=str(exc)[:2000],
                                 seconds=time.monotonic() - started))
        except Exception as exc:  # noqa: BLE001 - one item must never stop the run
            ledger.record(Record(item.collection, item.item_id, "failed",
                                 reason="convert_failed", error=str(exc)[:2000],
                                 seconds=time.monotonic() - started))
        else:
            ledger.record(Record(item.collection, item.item_id, "converted",
                                 seconds=time.monotonic() - started))

    try:
        if config.jobs <= 1:
            for item in items:
                handle(item)
        else:
            with ThreadPoolExecutor(max_workers=config.jobs) as pool:
                list(pool.map(handle, items))
    finally:
        client.close()


def convert_collection(cid: str, *, ledger: Ledger, config: Config, client: httpx.Client) -> None:
    collection = discover.fetch_collection(config.base_url, cid, client)
    items, note = discover.enumerate_items(
        config.base_url, config.bucket_api, cid, collection, client
    )
    if note:
        print(f"  ! {note}", file=sys.stderr)
        ledger.record(Record(cid, "-", "skipped", reason="stale_item_index", error=note))
    if not items:
        ledger.record(Record(cid, "-", "skipped", reason="empty_collection"))
        return
    if config.limit_per_collection:
        items = items[: config.limit_per_collection]
    print(f"==> {cid}: {len(items)} item(s)")
    convert_items(items, ledger=ledger, config=config)

    config_path = aggregate.write_config(
        aggregate.collection_config(collection), config.out / "_configs" / f"{cid}.yaml"
    )
    aggregate.update_collection(
        config.tool, config.out / cid / "items", config_path,
        config.out / cid / "collection.json",
    )


def run_collections(cids: list[str], *, ledger: Ledger, config: Config) -> None:
    """Attempt every collection; a failure is recorded, never fatal."""
    client = httpx.Client(timeout=120, follow_redirects=True)
    try:
        for cid in cids:
            try:
                convert_collection(cid, ledger=ledger, config=config, client=client)
            except Exception as exc:  # noqa: BLE001 - one collection must never stop the run
                print(f"  ! {cid} failed: {exc}", file=sys.stderr)
                ledger.record(Record(cid, "-", "failed", reason="convert_failed",
                                     error=str(exc)[:2000]))
    finally:
        client.close()


def aggregate_all(config: Config) -> None:
    client = httpx.Client(timeout=120, follow_redirects=True)
    try:
        catalog = client.get(f"{config.base_url}/catalog.json").json()
    finally:
        client.close()
    config_path = aggregate.write_config(
        aggregate.catalog_config(catalog), config.out / "_configs" / "catalog.yaml"
    )
    collections = sorted(config.out.glob("*/collection.json"))
    if collections:
        aggregate.update_catalog(config.tool, collections, config.out, config_path)


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(prog="catalog2cityparquet")
    parser.add_argument("--out", type=Path, default=Path("out/cityparquet-catalog"))
    parser.add_argument("--binary", type=Path, default=Path("target/release/cityparquet"))
    parser.add_argument(
        "--tool", type=Path,
        default=Path("vendor/city3d-stac-tool/target/release/city3dstac"),
    )
    parser.add_argument("--collection", action="append", dest="collections")
    parser.add_argument("--limit-per-collection", type=int)
    parser.add_argument("--jobs", type=int, default=8)
    parser.add_argument("--keep-downloads", action="store_true")
    parser.add_argument("--no-skip-existing", action="store_true")
    parser.add_argument("--crs", action="append", default=[],
                        metavar="COLLECTION=EPSG:xxxx")
    parser.add_argument("--base-url", default=BASE_URL)
    parser.add_argument("--bucket-api", default=BUCKET_API)
    parser.add_argument("--aggregate-only", action="store_true")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    config = Config(
        out=args.out, binary=args.binary, tool=args.tool, jobs=args.jobs,
        skip_existing=not args.no_skip_existing, keep_downloads=args.keep_downloads,
        limit_per_collection=args.limit_per_collection,
        crs_by_collection=dict(pair.split("=", 1) for pair in args.crs),
        base_url=args.base_url, bucket_api=args.bucket_api,
    )
    config.out.mkdir(parents=True, exist_ok=True)
    ledger = Ledger(config.out / "_reports")

    if not args.aggregate_only:
        client = httpx.Client(timeout=120, follow_redirects=True)
        try:
            cids = args.collections or discover.collection_ids(config.base_url, client)
        except Exception as exc:  # noqa: BLE001 - nothing can be attempted
            print(f"catalogue root unreachable: {exc}", file=sys.stderr)
            return 1
        finally:
            client.close()
        run_collections(cids, ledger=ledger, config=config)

    aggregate_all(config)
    summary = ledger.write_summary()
    print(f"\nsummary: {summary}")
    histogram = ledger.histogram()
    if histogram:
        print("failure reasons:")
        for reason, count in sorted(histogram.items(), key=lambda kv: -kv[1]):
            print(f"  {count:>7}  {reason}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd tools/catalog2cityparquet && uv run --extra dev pytest tests/test_orchestration.py -v`
Expected: PASS (4 tests)

- [ ] **Step 5: Run the whole Python suite**

Run: `cd tools/catalog2cityparquet && uv run --extra dev pytest -v`
Expected: PASS (26 tests)

- [ ] **Step 6: Commit**

```bash
git add tools/catalog2cityparquet/
git commit -m "feat(catalog2cityparquet): orchestration with per-item and per-collection isolation"
```

---

### Task 11: `just` recipes, end-to-end proof and documentation

**Files:**
- Modify: `justfile`
- Create: `tools/catalog2cityparquet/README.md`
- Modify: `CLAUDE.md` and `AGENTS.md` (keep the two in sync per repo convention)

**Interfaces:**
- Consumes: everything above.
- Produces: `just catalog-convert`, `just catalog-convert-collection`, `just catalog-aggregate`, `just catalog-test`.

- [ ] **Step 1: Add the recipes**

Append to `justfile`:

```
# ---------------------------------------------------------------------------
# STAC catalogue -> CityParquet mirror
# ---------------------------------------------------------------------------

# Build both binaries the driver shells out to.
catalog-tools:
    cargo build --release -p cityparquet-cli
    cargo build --release --manifest-path vendor/city3d-stac-tool/Cargo.toml

# Convert every collection of the published City3D catalogue into a CityParquet
# mirror under OUT. Resumable: an item whose package already carries a valid
# STAC Item is skipped, so a re-run continues where the last one stopped.
# Failures never abort the run - each is recorded in OUT/_reports/ and the next
# item (or collection) starts. Network-dependent; kept OUT of `just check`.
catalog-convert OUT='out/cityparquet-catalog' *ARGS: catalog-tools
    uv run --project tools/catalog2cityparquet python -m catalog2cityparquet \
        --out {{OUT}} {{ARGS}}

# Convert a single collection (e.g. `just catalog-convert-collection rotterdam-3d`).
catalog-convert-collection ID OUT='out/cityparquet-catalog' *ARGS: catalog-tools
    uv run --project tools/catalog2cityparquet python -m catalog2cityparquet \
        --out {{OUT}} --collection {{ID}} {{ARGS}}

# Rebuild collection.json/items.parquet/catalog.json from an existing output
# tree, with no downloads or conversions.
catalog-aggregate OUT='out/cityparquet-catalog': catalog-tools
    uv run --project tools/catalog2cityparquet python -m catalog2cityparquet \
        --out {{OUT}} --aggregate-only

# Driver unit tests (no network).
catalog-test:
    uv run --project tools/catalog2cityparquet --extra dev pytest -v
```

- [ ] **Step 2: Verify the recipes resolve**

Run: `just catalog-test`
Expected: PASS (26 tests)

- [ ] **Step 3: Prove the pipeline end-to-end on a small real collection**

Run: `just catalog-convert-collection rotterdam-3d out/e2e`

Expected: exit 0 and this tree —
```
out/e2e/rotterdam-3d/collection.json
out/e2e/rotterdam-3d/items.parquet
out/e2e/rotterdam-3d/items/3-20-DELFSHAVEN.city/building.parquet
out/e2e/rotterdam-3d/items/3-20-DELFSHAVEN.city/metadata.json
out/e2e/catalog.json
out/e2e/_reports/summary.csv
```

Verify the generated Item is properly wired:
```bash
python3 -c "
import json
d=json.load(open('out/e2e/rotterdam-3d/items/3-20-DELFSHAVEN.city/metadata.json'))
assert d['collection']=='rotterdam-3d', d.get('collection')
rels={l['rel'] for l in d['links']}
assert {'collection','parent','root','via'} <= rels, rels
print('object count:', d['properties']['city3d:city_objects'])
print('OK')
"
```
Expected: `object count: 853` and `OK`.

- [ ] **Step 4: Prove failure isolation against the real catalogue**

Run: `just catalog-convert-collection luxembourg-3d out/e2e-fail`

Expected: **exit 0** (a failure is not an abort), and `out/e2e-fail/_reports/luxembourg-3d.jsonl` containing one record with `"reason": "no_crs"`.

Then confirm the printed histogram names the reason rather than a bare stack trace.

- [ ] **Step 5: Write the driver README**

`tools/catalog2cityparquet/README.md` — covering: what it does; the five pipeline stages; the reconciliation policy and *why* (Japan's 306-of-60,471 index); the reason vocabulary as a table; the resume story; and a worked example of a full run with expected wall time. Include this table verbatim:

| Flag | Default | Meaning |
|---|---|---|
| `--out` | `out/cityparquet-catalog` | Output tree root |
| `--collection` | all | Repeatable; restrict to these collection ids |
| `--limit-per-collection` | none | Convert at most N items per collection |
| `--jobs` | 8 | Concurrent items; bounded out of politeness to origins |
| `--crs` | none | Repeatable `COLLECTION=EPSG:xxxx` override for CRS-less sources |
| `--keep-downloads` | off | Retain temp downloads for debugging |
| `--no-skip-existing` | off | Reconvert items that already have a package |
| `--aggregate-only` | off | Rebuild STAC from an existing tree, no downloads |

- [ ] **Step 6: Document the tool in the repo instructions**

Add a `tools/catalog2cityparquet` row to the crate/component table in `CLAUDE.md`, and add the four `catalog-*` recipes to its Commands section. **Mirror the identical change into `AGENTS.md`** — the repo convention is that the two stay in sync.

- [ ] **Step 7: Final verification and commit**

Run: `just check && just catalog-test`
Expected: both PASS

```bash
git add justfile tools/catalog2cityparquet/README.md CLAUDE.md AGENTS.md
git commit -m "feat(catalog2cityparquet): just recipes and documentation"
```

---

## Post-implementation: the first full run

Not a code task — the artefact the plan exists to produce.

```bash
just catalog-convert out/cityparquet-catalog -- --jobs 8
```

Expect hours, dominated by Japan's 60,090 tiles. The run is resumable, so it can be interrupted and restarted. When it finishes, `out/cityparquet-catalog/_reports/summary.csv` plus the printed histogram are the paper's coverage evidence.

Sanity-check the outcome against the probe's prediction (design §2.2): **15 collections converting, ~69,100 items**. A materially different number means either the catalogue changed or the driver has a bug — investigate before quoting the figures.
