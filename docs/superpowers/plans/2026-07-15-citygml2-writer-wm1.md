# CityGML 2.0 writer — W-M1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A standalone `citygml::writer` that serialises a CityParquet package directory into a CityGML 2.0 `.gml` document — `CityModel` + envelope + `srsName`, and one `bldg:Building` per Building-type row with its LoD `gml:Solid` geometry — reusing the crate's existing read primitives (`reader`/`decode`/`wkb_read`) and export's shell-partition helpers, with no `cjseq` intermediate.

**Architecture:** New module `crates/cityparquet/src/citygml/writer/` (sibling of the reader), split `mod`/`document`/`building`/`geometry`, using `quick_xml::Writer` for element structure. The CLI `export` arm routes a `.gml` output extension to `write_package`. Round-trip is proved by reading the writer's output back through the existing `citygml` reader and comparing the geometry projection.

**Tech Stack:** Rust workspace (`cityparquet-schema`, `cityparquet`, `cityparquet-cli`); `quick-xml`, `arrow`/`parquet`, `serde_json`, `cjseq`. Strict red-green TDD with real fixtures.

**Design spec:** `docs/superpowers/specs/2026-07-15-citygml2-writer-wm1-design.md` (revised per the Codex design review). Read it before Task 1.

## Global Constraints

- **TDD, red-green:** every behavioural change starts with a failing test; run it and see it fail before implementing. No inline hand-authored CityGML/CityJSON *for parsing/round-trip* tests — use the committed fixtures. In-code `DecodedGeometry`/`Value` literals for **pure-serialisation** unit tests are allowed (they exercise emit, not parse).
- **Green gate:** `just check` (clippy `-D warnings`, tests, fmt) must pass before each commit. No `dead_code`/unused warnings.
- **Standalone:** no `cjseq::CityJSON` intermediate document, no call into the CityJSON `export` function. DO reuse `wkb_read::wkb_to_geometry`, the `reader` extension trait, `row_json_object`, and export's `partition_shells`/`shell_faces_flat`.
- **CityGML 2.0 shape:** unprefixed `<CityModel xmlns="http://www.opengis.net/citygml/2.0">` + unprefixed `cityObjectMember`; declare `xmlns:bldg`, `xmlns:gml`, `xmlns:xlink`. `gml:boundedBy` MUST precede the first `cityObjectMember`. Only `bldg:lod1Solid`..`bldg:lod4Solid` are valid. `posList` carries `srsDimension="3"` and world coordinates; every ring is re-closed (first coord repeated).
- **Fixture:** round-trip uses `crates/cityparquet/tests/data/savenow_ingolstadt_lod2.gml` (3 Buildings, `bldg:lod2Solid`). No new fixture for the happy path.
- **Codex external review** at the end of W-M1 (see [[codex-external-review]]), then triage/fix before tagging.
- British English in prose/docs.

---

### Task 1: `Lod::major()` accessor

The LoD-major mapping (spec) needs the major component of a `Lod`; the field is private and there is no accessor.

**Files:**
- Modify: `crates/cityparquet-schema/src/types.rs` (the `impl Lod` block, near `column_suffix` ~:27)
- Test: same file's `#[cfg(test)] mod tests`

**Interfaces:**
- Produces: `Lod::major(&self) -> u8`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn lod_major_extracts_major_component() {
    assert_eq!(Lod::parse("2").unwrap().major(), 2);
    assert_eq!(Lod::parse("2.2").unwrap().major(), 2);
    assert_eq!(Lod::parse("1").unwrap().major(), 1);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cityparquet-schema lod_major_extracts_major_component`
Expected: FAIL — no method `major` (compile error).

- [ ] **Step 3: Implement**

In `impl Lod`:

```rust
/// The major LoD component (`2` for both `2` and `2.2`).
pub fn major(&self) -> u8 {
    self.major
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p cityparquet-schema`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/cityparquet-schema/src/types.rs
git commit -m "feat(schema): add Lod::major() accessor"
```

---

### Task 2: writer geometry — face → `gml:Polygon`, ring re-closing, `posList`

Pure serialisation: one decoded face (rings of coord indices + the coord pool) → a `gml:Polygon` with a re-closed exterior `gml:posList` and `gml:interior` rings for holes. Also creates the `citygml::writer` module skeleton and wires it into `citygml/mod.rs`.

**Files:**
- Create: `crates/cityparquet/src/citygml/writer/mod.rs`, `crates/cityparquet/src/citygml/writer/geometry.rs`
- Modify: `crates/cityparquet/src/citygml/mod.rs` (add `pub mod writer;`)

**Interfaces:**
- Consumes: `wkb_read::DecodedGeometry` (`coords: Vec<[f64;3]>`), a face `&[Vec<usize>]` (rings of coord indices).
- Produces: `citygml::writer::geometry::write_polygon(w: &mut quick_xml::Writer<W>, coords: &[[f64;3]], face: &[Vec<usize>]) -> Result<()>` — emits one `<gml:Polygon>`. Helper `pos_list(coords, ring) -> String` — space-joined `X Y Z …` with the ring **re-closed** (first coord appended).

- [ ] **Step 1: Write the failing test**

In `geometry.rs`'s test module:

```rust
use quick_xml::Writer;

fn emit<F: Fn(&mut Writer<Vec<u8>>) -> crate::Result<()>>(f: F) -> String {
    let mut w = Writer::new(Vec::new());
    f(&mut w).unwrap();
    String::from_utf8(w.into_inner()).unwrap()
}

#[test]
fn pos_list_reclose_appends_first_coord() {
    let coords = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]];
    // open ring (reader-decoded shape: last != first)
    let ring = vec![0usize, 1, 2];
    assert_eq!(pos_list(&coords, &ring), "0 0 0 1 0 0 1 1 0 0 0 0");
}

#[test]
fn write_polygon_emits_exterior_and_interior_rings() {
    let coords = vec![
        [0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [4.0, 4.0, 0.0], [0.0, 4.0, 0.0], // outer
        [1.0, 1.0, 0.0], [2.0, 1.0, 0.0], [2.0, 2.0, 0.0],                   // hole
    ];
    let face = vec![vec![0usize, 1, 2, 3], vec![4usize, 5, 6]];
    let xml = emit(|w| write_polygon(w, &coords, &face));
    assert!(xml.contains("<gml:Polygon>"));
    assert!(xml.contains("<gml:exterior><gml:LinearRing><gml:posList srsDimension=\"3\">0 0 0 4 0 0 4 4 0 0 4 0 0 0 0</gml:posList>"));
    assert!(xml.contains("<gml:interior><gml:LinearRing><gml:posList srsDimension=\"3\">1 1 0 2 1 0 2 2 0 1 1 0</gml:posList>"));
}
```

> Float formatting: use Rust's default `{}` `Display` for `f64` (matches how the reader's `posList` parser reads plain decimals; `0.0` prints as `0`). Confirm against `citygml/geometry.rs`'s parse — it splits on whitespace and `parse::<f64>()`, which accepts both `0` and `0.0`, so `{}` is safe.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cityparquet citygml::writer::geometry`
Expected: FAIL — module/functions do not exist (compile error).

- [ ] **Step 3: Implement**

`citygml/mod.rs`: add `pub mod writer;`.

`citygml/writer/mod.rs` (skeleton — will grow in later tasks):

```rust
//! Native CityGML 2.0 output writer (CityParquet package -> .gml).
//!
//! W-M1: CityModel + envelope + srsName, and bldg:Building with LoD gml:Solid.
//! Standalone — reuses wkb_read/reader/export shell helpers, no cjseq document.

pub mod geometry;
```

`citygml/writer/geometry.rs`:

```rust
use std::io::Write;
use quick_xml::Writer;
use crate::Result;

/// One ring's `posList` text: `X Y Z` per vertex, world coords, **re-closed**
/// (the WKB reader strips the closing vertex, GML requires it back).
pub fn pos_list(coords: &[[f64; 3]], ring: &[usize]) -> String {
    let mut out = String::new();
    let mut push = |i: usize| {
        let c = coords[i];
        if !out.is_empty() { out.push(' '); }
        out.push_str(&format!("{} {} {}", c[0], c[1], c[2]));
    };
    for &i in ring { push(i); }
    if let Some(&first) = ring.first() { push(first); } // re-close
    out
}

/// One face (rings of coord indices) -> a `<gml:Polygon>`: ring 0 exterior,
/// ring 1.. interior (holes).
pub fn write_polygon<W: Write>(
    w: &mut Writer<W>,
    coords: &[[f64; 3]],
    face: &[Vec<usize>],
) -> Result<()> {
    // Emit with quick_xml's raw writer; posList text written verbatim.
    // (Use w.get_mut().write_all(...) or quick_xml element builders — pick one
    // idiom and keep it consistent across the writer modules; the exact call
    // sequence is the implementer's, the OUTPUT must match the tests.)
    // Structure:
    //   <gml:Polygon>
    //     <gml:exterior><gml:LinearRing><gml:posList srsDimension="3">…</…></…></…>
    //     for hole in &face[1..]:
    //       <gml:interior><gml:LinearRing><gml:posList srsDimension="3">…</…></…></…>
    //   </gml:Polygon>
    unimplemented!("emit per the doc comment; tests pin the exact bytes")
}
```

Implement `write_polygon` so the two tests' `assert!(xml.contains(...))` pass. A face with no rings is a caller error (upstream guarantees ≥1 ring); a ring is emitted via `pos_list`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p cityparquet citygml::writer::geometry` then `just check`
Expected: PASS, no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/cityparquet/src/citygml/writer/mod.rs crates/cityparquet/src/citygml/writer/geometry.rs crates/cityparquet/src/citygml/mod.rs
git commit -m "feat(citygml): writer geometry — gml:Polygon posList with ring re-closing"
```

---

### Task 3: solid shell partitioning → `gml:Solid`

`DecodedKind::PolyhedralSurface(faces)` + `geometry_properties.solid_shell_faces` → `<gml:Solid>` with shell 0 as `gml:exterior` and shells 1.. as `gml:interior`, each a `gml:CompositeSurface` of the shell's `gml:Polygon`s. Reuses export's shell helpers (made `pub(crate)`).

**Files:**
- Modify: `crates/cityparquet/src/export.rs` — change `fn partition_shells` and `fn shell_faces_flat` from private to `pub(crate)`.
- Modify: `crates/cityparquet/src/citygml/writer/geometry.rs` — add `write_solid`.
- Test: `geometry.rs` test module.

**Interfaces:**
- Consumes: `export::partition_shells(faces, counts) -> Result<Vec<Vec<Vec<Vec<usize>>>>>`, `export::shell_faces_flat(props: Option<&Value>) -> Result<Option<Vec<usize>>>`.
- Produces: `write_solid<W: Write>(w, coords, faces: &[Vec<Vec<usize>>], props: Option<&serde_json::Value>) -> Result<()>` — emits `<gml:Solid>`; errors on shell-count mismatch (via `partition_shells`).

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn write_solid_partitions_exterior_and_interior_shells() {
    // 4 coords forming two trivial triangular faces per shell is overkill;
    // use a minimal 2-shell case: shell0 = 1 face, shell1 = 1 face.
    let coords = vec![
        [0.0,0.0,0.0],[1.0,0.0,0.0],[1.0,1.0,0.0], // face 0 (outer shell)
        [0.2,0.2,0.2],[0.4,0.2,0.2],[0.4,0.4,0.2], // face 1 (inner shell)
    ];
    let faces = vec![vec![vec![0usize,1,2]], vec![vec![3usize,4,5]]];
    let props = serde_json::json!({ "type": "Solid", "solid_shell_faces": [1, 1] });
    let xml = emit(|w| write_solid(w, &coords, &faces, Some(&props)));
    assert!(xml.starts_with("<gml:Solid>"));
    assert!(xml.contains("<gml:exterior><gml:CompositeSurface>"));
    assert!(xml.contains("<gml:interior><gml:CompositeSurface>"));
    // exactly one exterior, one interior
    assert_eq!(xml.matches("<gml:exterior>").count(), 1);
    assert_eq!(xml.matches("<gml:interior>").count(), 1);
}

#[test]
fn write_solid_single_shell_has_no_interior() {
    let coords = vec![[0.0,0.0,0.0],[1.0,0.0,0.0],[1.0,1.0,0.0]];
    let faces = vec![vec![vec![0usize,1,2]]];
    // No solid_shell_faces -> single shell fallback.
    let xml = emit(|w| write_solid(w, &coords, &faces, None));
    assert_eq!(xml.matches("<gml:exterior>").count(), 1);
    assert_eq!(xml.matches("<gml:interior>").count(), 0);
}

#[test]
fn write_solid_shell_count_mismatch_errors() {
    let coords = vec![[0.0,0.0,0.0],[1.0,0.0,0.0],[1.0,1.0,0.0]];
    let faces = vec![vec![vec![0usize,1,2]]]; // 1 face
    let props = serde_json::json!({ "solid_shell_faces": [1, 1] }); // claims 2
    let mut w = Writer::new(Vec::new());
    assert!(write_solid(&mut w, &coords, &faces, Some(&props)).is_err());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cityparquet citygml::writer::geometry::tests::write_solid`
Expected: FAIL — `write_solid` undefined; `export::partition_shells` not accessible (compile error).

- [ ] **Step 3: Implement**

In `export.rs`, change the two helpers' visibility:

```rust
pub(crate) fn partition_shells( /* unchanged body */ ) -> Result<Vec<Vec<Vec<Vec<usize>>>>> { … }
pub(crate) fn shell_faces_flat(props: Option<&Value>) -> Result<Option<Vec<usize>>> { … }
```

In `geometry.rs`:

```rust
use serde_json::Value;

/// A `PolyhedralSurface`'s flat face list + its `geometry_properties` ->
/// `<gml:Solid>` (shell 0 exterior, shells 1.. interior).
pub fn write_solid<W: Write>(
    w: &mut Writer<W>,
    coords: &[[f64; 3]],
    faces: &[Vec<Vec<usize>>],
    props: Option<&Value>,
) -> Result<()> {
    let counts = crate::export::shell_faces_flat(props)?;
    let shells = crate::export::partition_shells(faces.to_vec(), counts.as_deref())?;
    // <gml:Solid>
    //   <gml:exterior><gml:CompositeSurface> shell[0] faces… </…></gml:exterior>
    //   for shell in &shells[1..]:
    //     <gml:interior><gml:CompositeSurface> faces… </…></gml:interior>
    // </gml:Solid>
    // Each face -> <gml:surfaceMember>{write_polygon}</gml:surfaceMember>.
    unimplemented!("emit per the doc comment; tests pin the structure")
}
```

Implement so the three tests pass. `partition_shells` already errors on a count mismatch — propagate it.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p cityparquet citygml::writer::geometry` then `just check`
Expected: PASS, no warnings (export's helpers are now used by the writer, so `pub(crate)` introduces no dead-code).

- [ ] **Step 5: Commit**

```bash
git add crates/cityparquet/src/export.rs crates/cityparquet/src/citygml/writer/geometry.rs
git commit -m "feat(citygml): writer gml:Solid with exterior/interior shell partitioning"
```

---

### Task 4: CRS → validated `srsName`

Map the package CRS metadata to a `urn:ogc:def:crs:EPSG::<code>` `srsName`, validated by the reader's `resolve`. Lives in `citygml/crs.rs` next to `resolve` (D4).

**Files:**
- Modify: `crates/cityparquet/src/citygml/crs.rs` (add the inverse helper + tests)

**Interfaces:**
- Consumes: `CityParquetMetadata.crs: Option<serde_json::Value>` (an OGC EPSG URL string or a PROJJSON object), `citygml::crs::resolve`.
- Produces: `citygml::crs::srs_name_for(crs: Option<&serde_json::Value>) -> Result<Option<String>>` — `Ok(None)` when no CRS; `Ok(Some(urn))` for a validated projected EPSG code; `Err` for geographic/unsupported/non-EPSG.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn srs_name_from_epsg_url_round_trips_through_resolve() {
    let crs = serde_json::json!("https://www.opengis.net/def/crs/EPSG/0/28992");
    let srs = srs_name_for(Some(&crs)).unwrap().unwrap();
    assert_eq!(srs, "urn:ogc:def:crs:EPSG::28992");
    // The emitted srsName must resolve back to the SAME code.
    assert!(matches!(resolve(&srs).unwrap(), CrsResolution::Epsg(c) if c == "28992"));
}

#[test]
fn srs_name_from_projjson_epsg_object() {
    let crs = serde_json::json!({ "id": { "authority": "EPSG", "code": 28992 } });
    assert_eq!(srs_name_for(Some(&crs)).unwrap().unwrap(), "urn:ogc:def:crs:EPSG::28992");
}

#[test]
fn srs_name_none_when_no_crs() {
    assert_eq!(srs_name_for(None).unwrap(), None);
}

#[test]
fn srs_name_geographic_crs_errors() {
    let crs = serde_json::json!("https://www.opengis.net/def/crs/EPSG/0/4326");
    assert!(srs_name_for(Some(&crs)).is_err());
}
```

> Confirm `CrsResolution`'s projected variant name/shape (the spec calls it `Epsg(code)`) by reading `crs.rs` — match the real variant. Confirm `resolve` errors on 4326 (geographic) as the spec states; if `resolve` accepts a bare `urn:ogc:def:crs:EPSG::4326`, the geographic guard must be applied in `srs_name_for` itself (reuse `crs.rs`'s `GEOGRAPHIC_EPSG` list).

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cityparquet citygml::crs::tests::srs_name`
Expected: FAIL — `srs_name_for` undefined.

- [ ] **Step 3: Implement**

Add to `crs.rs`: parse the code out of an OGC EPSG URL (`…/EPSG/0/<code>` or `…/EPSG/<code>`) or a PROJJSON object (`id.authority == "EPSG"` && numeric `id.code`); reject other authorities. Build `urn:ogc:def:crs:EPSG::<code>`, then call `resolve(&urn)` and require the projected `CrsResolution` for the same code (this reuses the existing geographic-rejection). Return `Err` otherwise. `None` CRS → `Ok(None)`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p cityparquet citygml::crs` then `just check`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/cityparquet/src/citygml/crs.rs
git commit -m "feat(citygml): CRS -> validated EPSG srsName for the writer"
```

---

### Task 5: document skeleton + envelope

`citygml/writer/document.rs`: open/close `CityModel` with namespaces; emit `gml:boundedBy/gml:Envelope` from an accumulated coordinate bound; place `srsName` (from Task 4) on the envelope.

**Files:**
- Create: `crates/cityparquet/src/citygml/writer/document.rs`
- Modify: `citygml/writer/mod.rs` (`pub mod document;`)

**Interfaces:**
- Produces:
  - `struct Bounds { min: [f64;3], max: [f64;3], any: bool }` with `fn add(&mut self, c: [f64;3])` and `fn new()`.
  - `write_city_model_open<W: Write>(w, srs_name: Option<&str>, bounds: &Bounds) -> Result<()>` — root element + namespaces + `gml:boundedBy` (omitted when `!bounds.any`).
  - `write_city_model_close<W: Write>(w) -> Result<()>`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn envelope_from_bounds_exact_corners_with_srs() {
    let mut b = Bounds::new();
    b.add([1.0, 2.0, 3.0]); b.add([4.0, 5.0, 6.0]);
    let xml = emit(|w| write_city_model_open(w, Some("urn:ogc:def:crs:EPSG::28992"), &b));
    assert!(xml.contains("<CityModel xmlns=\"http://www.opengis.net/citygml/2.0\""));
    assert!(xml.contains("xmlns:bldg=\"http://www.opengis.net/citygml/building/2.0\""));
    assert!(xml.contains("<gml:Envelope srsName=\"urn:ogc:def:crs:EPSG::28992\" srsDimension=\"3\">"));
    assert!(xml.contains("<gml:lowerCorner>1 2 3</gml:lowerCorner>"));
    assert!(xml.contains("<gml:upperCorner>4 5 6</gml:upperCorner>"));
}

#[test]
fn no_geometry_means_no_envelope() {
    let b = Bounds::new(); // nothing added
    let xml = emit(|w| write_city_model_open(w, None, &b));
    assert!(!xml.contains("gml:boundedBy"));
    assert!(xml.contains("<CityModel"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cityparquet citygml::writer::document`
Expected: FAIL — module/functions undefined.

- [ ] **Step 3: Implement**

`Bounds` accumulates min/max over added coords (`any` flips true on first add). `write_city_model_open` emits the XML declaration + root element with the four namespaces (unprefixed default = citygml/2.0, `bldg`, `gml`, `xlink`), then, when `bounds.any`, a `gml:boundedBy/gml:Envelope` (`srsName` attr only when `srs_name.is_some()`) with lower/upper corners formatted like `pos_list` (`{}` floats, space-joined). `write_city_model_close` closes `</CityModel>`.

- [ ] **Step 4: Run the tests + commit**

Run: `cargo test -p cityparquet citygml::writer::document` then `just check`

```bash
git add crates/cityparquet/src/citygml/writer/document.rs crates/cityparquet/src/citygml/writer/mod.rs
git commit -m "feat(citygml): writer CityModel skeleton + envelope"
```

---

### Task 6: building element — LoD major mapping + `gml:id` validation

`citygml/writer/building.rs`: from a row's `id` + its per-LoD `(DecodedGeometry, props)` solids, emit one `bldg:Building` with one `bldg:lod<major>Solid` per major (collision → keep highest minor, count the rest). Validate `gml:id` NCName + uniqueness at the driver level; the building writer validates NCName syntax.

**Files:**
- Create: `crates/cityparquet/src/citygml/writer/building.rs`
- Modify: `citygml/writer/mod.rs` (`pub mod building;`)

**Interfaces:**
- Consumes: `Lod::major` (Task 1), `geometry::write_solid` (Task 3).
- Produces:
  - `fn is_ncname(id: &str) -> bool` (valid XML NCName: first char letter/`_`, rest letter/digit/`.`/`-`/`_`, no `:`).
  - `struct BuildingSolids { pub id: String, pub solids: Vec<(Lod, DecodedGeometry, Option<Value>)> }`.
  - `fn write_building<W: Write>(w, b: &BuildingSolids, bounds: &mut Bounds, report: &mut WriteReport) -> Result<bool>` — emits `<cityObjectMember><bldg:Building gml:id="…">` with the deduped, ascending `lod<major>Solid`s, accumulating emitted coords into `bounds`; returns `Ok(false)` (nothing emitted, caller counts `buildings_without_solid_skipped`) when no solid survives; `Err` on invalid NCName.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn is_ncname_accepts_3dbag_ids_rejects_bad() {
    assert!(is_ncname("NL.IMBAG.Pand.0503100000013175-0"));
    assert!(is_ncname("_x"));
    assert!(!is_ncname("3leadingdigit"));
    assert!(!is_ncname("has:colon"));
    assert!(!is_ncname(""));
}

#[test]
fn major_lod_collision_keeps_highest_minor_and_counts_the_rest() {
    // Two solids on major 2: lod2 and lod2_2 -> one bldg:lod2Solid, one skip.
    let coords = vec![[0.0,0.0,0.0],[1.0,0.0,0.0],[1.0,1.0,0.0]];
    let g = DecodedGeometry { coords: coords.clone(),
        kind: DecodedKind::PolyhedralSurface(vec![vec![vec![0,1,2]]]) };
    let b = BuildingSolids {
        id: "B1".into(),
        solids: vec![
            (Lod::parse("2").unwrap(),   g.clone(), None),
            (Lod::parse("2.2").unwrap(), g.clone(), None),
        ],
    };
    let mut bounds = Bounds::new();
    let mut report = WriteReport::default();
    let xml = { let mut w = Writer::new(Vec::new());
        assert!(write_building(&mut w, &b, &mut bounds, &mut report).unwrap());
        String::from_utf8(w.into_inner()).unwrap() };
    assert_eq!(xml.matches("<bldg:lod2Solid>").count(), 1);
    assert_eq!(report.lod_columns_skipped, 1);
    assert!(bounds.any);
}

#[test]
fn invalid_ncname_id_errors() {
    let b = BuildingSolids { id: "3bad".into(), solids: vec![] };
    let mut w = Writer::new(Vec::new());
    assert!(write_building(&mut w, &b, &mut Bounds::new(), &mut WriteReport::default()).is_err());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cityparquet citygml::writer::building`
Expected: FAIL — undefined.

- [ ] **Step 3: Implement**

- `is_ncname` per the XML NCName production (ASCII-pragmatic: `[A-Za-z_][A-Za-z0-9._-]*`; that is what real CityGML ids use).
- `write_building`: error if `!is_ncname(&b.id)`. Group `solids` by `lod.major()`; drop majors outside `1..=4` (count `lod_columns_skipped`); within a major keep the entry with the highest `minor` (treat `None` as lower than any `Some`), counting the dropped as `lod_columns_skipped`. If nothing remains, emit nothing and return `Ok(false)`. Else emit `<cityObjectMember><bldg:Building gml:id="{id}">`, then for each kept major in ascending order a `<bldg:lod{major}Solid>{write_solid}</bldg:lod{major}Solid>`, feeding every emitted coord to `bounds.add`. Close tags; return `Ok(true)`. A non-`PolyhedralSurface` kind here is a caller invariant violation → treat as skip (the driver only passes solids, but guard defensively and count).

> Bounds accumulation: `write_solid` writes coords but `write_building` also needs them for the envelope. Simplest: iterate the kept solids' `coords` used by their faces and `bounds.add` each while emitting. Since a solid's every coord index appears in its faces, feeding all of `g.coords` referenced by its faces is correct; feeding the whole `g.coords` pool is acceptable only if the pool has no unused vertices — to be safe, add only coords actually referenced by the emitted faces.

- [ ] **Step 4: Run tests + commit**

Run: `cargo test -p cityparquet citygml::writer::building` then `just check`

```bash
git add crates/cityparquet/src/citygml/writer/building.rs crates/cityparquet/src/citygml/writer/mod.rs
git commit -m "feat(citygml): writer bldg:Building — LoD major mapping + gml:id validation"
```

---

### Task 7: `write_package` driver + round-trip oracle

`citygml/writer/mod.rs`: `write_package` opens the manifest tables (first-table-authoritative integrity checks like `export`), iterates Building rows, builds `BuildingSolids`, drives `write_building`, accumulates the envelope, enforces document-wide `gml:id` uniqueness, and writes the whole document (envelope before members). Plus the Ingolstadt round-trip integration test.

**Files:**
- Modify: `crates/cityparquet/src/citygml/writer/mod.rs` (`WriteOptions`, `WriteReport`, `write_package`)
- Test: `crates/cityparquet/tests/citygml_writer_real_data.rs` (new integration test file)

**Interfaces:**
- Produces: `WriteOptions { package_dir, output }`, `WriteReport { buildings_written, non_building_skipped, buildings_without_solid_skipped, composite_solids_skipped, lod_columns_skipped }`, `pub fn write_package(opts: &WriteOptions) -> Result<WriteReport>`.
- Consumes: `reader::CityParquetReaderBuilder`, `ParquetRecordBatchReaderBuilder`, `wkb_read::wkb_to_geometry`, `export::row_json_object` (make it `pub(crate)` if not already), the Task 2–6 sub-writers, `citygml::crs::srs_name_for`.

- [ ] **Step 1: Write the failing round-trip test**

`tests/citygml_writer_real_data.rs`:

```rust
use cityparquet::citygml::{FeatureReader, writer::{WriteOptions, write_package}};

#[test]
fn ingolstadt_lod2_solids_round_trip_gml_to_parquet_to_gml() {
    let tmp = tempfile::tempdir().unwrap();
    let pkg = tmp.path().join("pkg");
    let out_gml = tmp.path().join("out.gml");
    let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/savenow_ingolstadt_lod2.gml");

    // 1. original .gml -> read building geometry via the reader (the oracle).
    let original = read_building_solids(fixture); // helper: id -> sorted coord multiset

    // 2. convert the fixture to a CityParquet package (existing pipeline).
    convert_gml_to_package(fixture, &pkg); // helper using package::convert over a CityGML Source

    // 3. write .gml from the package.
    let report = write_package(&WriteOptions { package_dir: pkg.clone(), output: out_gml.clone() }).unwrap();
    assert_eq!(report.buildings_written, 3);

    // 4. re-read out.gml, compare the geometry projection (ids + solid coords).
    let round = read_building_solids(out_gml.to_str().unwrap());
    assert_eq!(original.keys().collect::<std::collections::BTreeSet<_>>(),
               round.keys().collect::<std::collections::BTreeSet<_>>());
    for (id, coords) in &original {
        assert_eq!(round.get(id), Some(coords), "geometry mismatch for {id}");
    }
}
```

> Write the two helpers (`read_building_solids` reads a `.gml` through the existing `citygml::FeatureReader` and projects each building to `id -> sorted set of solid boundary coordinates`; `convert_gml_to_package` drives `package::convert` with a CityGML `Source` over the fixture). Reuse whatever the reader's existing integration tests use to open a CityGML `Source` and run `convert` (grep `tests/` for the reader's round-trip harness). Compare a **sorted coordinate multiset** so ring rotation/closing differences don't cause false negatives; do NOT compare attributes/semantics.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cityparquet --test citygml_writer_real_data`
Expected: FAIL — `write_package` unimplemented.

- [ ] **Step 3: Implement `write_package`**

- Read `metadata.json` → `PackageManifest`; reject empty/duplicate tables (mirror `export`, reuse the same checks — grep export.rs:960-1000).
- Open the first table with `ParquetRecordBatchReaderBuilder::try_new`; take `cityparquet_metadata()` (CRS) and `cityparquet_arrow_schema()`; for later tables verify version + rendered schema match the first (reuse export's per-table checks at export.rs:1093).
- `srs_name = citygml::crs::srs_name_for(meta.crs.as_ref())?`.
- Iterate every table's `RecordBatch`es and rows. For each row: read `object_type` (dict Utf8); if != `"Building"`, `non_building_skipped++`, continue. Read `id` (Utf8). For each `geometry_lod<k>` column that is non-null: `wkb_to_geometry(blob)`; branch on `kind` (`PolyhedralSurface` → collect `(Lod, DecodedGeometry, props)` where `props = row_json_object(batch, "geometry_properties_<suffix>", row)`; `GeometryCollection` → `composite_solids_skipped++`; else skip). Assemble `BuildingSolids`.
- Enforce document-wide `gml:id` uniqueness: track a `HashSet<String>` of emitted ids; a duplicate is an `Err`.
- Envelope ordering: buffer member XML in memory (D2 option a), accumulate `Bounds`, then write header (`write_city_model_open` with `srs_name` + `bounds`) + buffered members + `write_city_model_close` to `File::create(output)`.
- `write_building` returning `Ok(false)` → `buildings_without_solid_skipped++`; `Ok(true)` → `buildings_written++`.

Add `WriteOptions`/`WriteReport` (fields per the spec) to `mod.rs`. Make `export::row_json_object` `pub(crate)` if the writer reuses it (else replicate its small body).

- [ ] **Step 4: Run the tests**

Run: `cargo test -p cityparquet --test citygml_writer_real_data` then `cargo test -p cityparquet` then `just check`
Expected: PASS — round-trip holds; no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/cityparquet/src/citygml/writer/mod.rs crates/cityparquet/src/export.rs crates/cityparquet/tests/citygml_writer_real_data.rs
git commit -m "feat(citygml): write_package driver + Ingolstadt round-trip oracle"
```

---

### Task 8: CLI `export OUT.gml` wiring

Route a `.gml` output extension in the CLI `Export` arm to `write_package`; report the counts.

**Files:**
- Modify: `crates/cityparquet-cli/src/main.rs` (the `Commands::Export` arm)
- Test: `crates/cityparquet-cli/tests/cli.rs`

**Interfaces:**
- Consumes: `cityparquet::citygml::writer::{WriteOptions, write_package}`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn export_to_gml_writes_citygml() {
    let out = tempdir().unwrap();
    let pkg = out.path().join("pkg");
    // convert the Ingolstadt fixture (or the delft fixture) to a package first
    convert_fixture_to_pkg(&pkg);
    let gml = out.path().join("model.gml");
    run_cli(&["export", pkg.to_str().unwrap(), gml.to_str().unwrap()]).assert().success();
    let text = std::fs::read_to_string(&gml).unwrap();
    assert!(text.contains("<CityModel"));
    assert!(text.contains("<bldg:Building"));
}
```

> Use `cli.rs`'s existing CLI-runner + fixture helpers. If converting a CityGML fixture through the CLI is awkward, convert a CityJSON fixture that yields Building Solids; the assertion only needs a Building + CityModel in the output.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cityparquet-cli --test cli export_to_gml_writes_citygml`
Expected: FAIL — `.gml` extension unsupported (the export arm errors).

- [ ] **Step 3: Implement**

In the `Export` arm of `main.rs`, before calling the CityJSON `export`, check the output extension: if `Some("gml")`, call `cityparquet::citygml::writer::write_package(&WriteOptions { package_dir, output })` and print a report line (`buildings_written non_building_skipped buildings_without_solid_skipped composite_solids_skipped lod_columns_skipped`); otherwise the existing `.city.jsonl`/`.city.json` path.

- [ ] **Step 4: Run the tests + commit**

Run: `cargo test -p cityparquet-cli` then `cargo test --workspace` then `just check`

```bash
git add crates/cityparquet-cli/src/main.rs crates/cityparquet-cli/tests/cli.rs
git commit -m "feat(cli): export PACKAGE_DIR OUT.gml -> CityGML 2.0"
```

---

## Post-plan: milestone review

- [ ] **Full workspace:** `cargo test --workspace` and `just check` green.
- [ ] **Codex external review** (see [[codex-external-review]]): `codex exec --cd "$(pwd)" --sandbox read-only "review the W-M1 CityGML writer diff <base>..HEAD for correctness / CityGML 2.0 validity / round-trip fidelity"`; triage findings, fix real ones, re-run tests.
- [ ] Update the design spec's status if any decisions changed; note the milestone in memory.

## Self-Review notes

- **Spec coverage:** shell partitioning → Task 3; LoD major mapping + gml:id → Task 6; CRS srsName → Task 4; envelope-from-emitted-coords → Tasks 5+6+7; unprefixed namespace → Task 5; multi-table integrity → Task 7; CLI/overwrite reality → Task 8/Task 7. Round-trip oracle (Ingolstadt) → Task 7. Ring re-closing + world coords → Task 2.
- **Type consistency:** `Bounds`, `BuildingSolids`, `WriteReport`, `WriteOptions`, `write_polygon`/`write_solid`/`write_building`/`write_city_model_open`/`srs_name_for`/`is_ncname`/`Lod::major` are referenced with the same signatures across tasks.
- **Deferred (W-M2/M3), not in this plan:** semantic surfaces, CompositeSolid/MultiSolid emission, BuildingParts, attributes, appearance, overwrite protection.
