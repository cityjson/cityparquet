# CityGML 2.0 writer — W-M1 design (CityParquet package → CityGML, Buildings + LoD Solid)

**Status:** approved design, ready for an implementation plan.
**Date:** 2026-07-15.
**Scope:** W-M1 only. This is the first milestone of a native CityGML 2.0 **writer**, the inverse-direction sibling of the existing `citygml` reader (which is at M3).

## Goal

Add a **standalone** path that serialises a CityParquet package directory into a **CityGML 2.0** document (`.gml`): `CityModel` skeleton + envelope + `srsName`, and one `bldg:Building` per Building-type object with its LoD `gml:Solid` geometry. "Standalone" means it emits GML **directly** and does **not** build a `cjseq::CityJSON` intermediate document or call the CityJSON `export` reconstruction — but it **does reuse** the crate's existing low-level read primitives (`reader`/`decode` for package/table iteration, `wkb_read::wkb_to_geometry` for WKB → coordinate rings). Re-implementing Parquet reading or WKB parsing is explicitly out of scope.

W-M1 deliberately stops at geometry. Semantic surfaces, CompositeSolid, BuildingParts, attributes, and appearance are later milestones (W-M2/W-M3) and are **not** in this spec.

## Non-goals (W-M1)

- No `bldg:boundedBy` semantic surfaces (Roof/Wall/GroundSurface) — W-M2.
- No `gml:CompositeSolid` / MultiSolid — W-M2. (A `GeometryCollection`-of-PolyhedralSurface WKB, i.e. a MultiSolid/CompositeSolid, is **skipped and counted** in W-M1, not emitted.)
- No BuildingParts / `bldg:consistsOfBuildingPart` — W-M2.
- No attributes (`bldg:` typed or `gen:` generic) — W-M3.
- No appearance (materials/textures) — later.
- No re-projection. CRS is provenance only, exactly as on the read side.

## Architecture

A new module **`crates/cityparquet/src/citygml/writer/`** (sibling of the reader's modules under `citygml/`), split by responsibility so each file has one job and can be tested in isolation:

| File | Responsibility |
|---|---|
| `citygml/writer/mod.rs` | Public entry `write_package`, the `WriteOptions`/`WriteReport` types, the top-level document loop (open manifest tables, iterate rows, drive the sub-writers), envelope accumulation. |
| `citygml/writer/document.rs` | `CityModel` open/close, namespace declarations, `gml:boundedBy`/`gml:Envelope` emission, `srsName` resolution from package CRS metadata. |
| `citygml/writer/building.rs` | One `bldg:Building` element: `gml:id`, and its `bldg:lod<k>Solid` for each LoD column that holds a Solid. |
| `citygml/writer/geometry.rs` | WKB `DecodedKind::PolyhedralSurface` → `gml:Solid/gml:exterior/gml:CompositeSurface` of `gml:Polygon`s with **re-closed** `gml:posList` rings. |
| `citygml/writer/xml.rs` | Thin, dependency-free XML emit helpers (element open/close, escaped text, indentation) OR a `quick-xml` `Writer` wrapper — match whatever the reader's `citygml/xml.rs` established for symmetry. |

`citygml/mod.rs` gains `pub mod writer;` and re-exports the entry point.

### Public interface

```rust
// citygml/writer/mod.rs
pub struct WriteOptions {
    pub package_dir: PathBuf,
    pub output: PathBuf,      // the .gml file to write
    pub overwrite: bool,
}

/// Counts mirroring the export report's drop-counter style.
#[derive(Debug, Default, PartialEq)]
pub struct WriteReport {
    pub buildings_written: usize,
    /// Rows skipped because object_type is not Building (W-M1 scope).
    pub non_building_skipped: usize,
    /// Building rows with no Solid geometry in any LoD column (or only
    /// MultiSurface / MultiSolid geometry, which W-M1 does not emit).
    pub buildings_without_solid_skipped: usize,
    /// Individual LoD solids skipped because the WKB was MultiSolid/
    /// CompositeSolid (GeometryCollection) — deferred to W-M2.
    pub composite_solids_skipped: usize,
}

pub fn write_package(opts: &WriteOptions) -> Result<WriteReport>;
```

### CLI wiring

Extend the existing `export` command's output-extension detection (`export.rs::output_format`, currently `.jsonl`/`.json`) with a `.gml` branch that routes to `citygml::writer::write_package` — **not** through the CityJSON `export` function. So:

```bash
cargo run -p cityparquet-cli -- export PACKAGE_DIR OUT.gml
```

writes CityGML. The `.city.jsonl`/`.city.json` paths are untouched. (Decision D1 below records the alternative of a separate subcommand; reusing `export` is the recommended default for UX consistency.)

## Data flow

```
PACKAGE_DIR/
  metadata.json  ──► PackageManifest.tables, CityParquetMetadata (crs, transform)
  <tables>.parquet
        │
        │  for each table in manifest.tables:
        │    open with ParquetRecordBatchReaderBuilder + CityParquetReaderBuilder
        │    (reuse reader::cityparquet_metadata() for CRS)
        │    for each RecordBatch, each row:
        │       object_type (dict Utf8) == "Building"?  ── no ──► non_building_skipped++
        │       id (Utf8), geometry_lod<k> (Binary WKB), bbox (struct) columns
        │            │
        │            │  wkb_read::wkb_to_geometry(blob) -> DecodedGeometry
        │            │     { coords: Vec<[f64;3]> (WORLD coords), kind }
        │            ▼
        │       kind == PolyhedralSurface(faces) ──► emit <bldg:lod{k}Solid>
        │       kind == GeometryCollection(..)   ──► composite_solids_skipped++ (W-M2)
        │       kind == MultiPolygon(..)/other   ──► not a Solid: skip for this LoD
        │       (no Solid in any LoD)            ──► buildings_without_solid_skipped++
        ▼
   <CityModel> … <gml:Envelope> … <cityObjectMember><bldg:Building> … </CityModel>
```

The dataset **envelope** is the union of the per-row `bbox` struct column, accumulated during iteration (there is no dataset-level bbox in metadata; unioning the row bboxes is authoritative and needs no second pass if the envelope is written after the members — see "Envelope ordering" below).

**Building emission rule (removes the counter ambiguity):** a `bldg:Building` is emitted **only if** it has at least one emittable `bldg:lod<k>Solid` (i.e. at least one LoD column decoding to a `PolyhedralSurface`). A Building row that ends with zero emittable solids is skipped entirely — W-M1 never emits an empty `bldg:Building` — and counted once in `buildings_without_solid_skipped`. `composite_solids_skipped` is an independent **per-LoD-column** diagnostic counter for the specific MultiSolid/CompositeSolid case; a Building whose only geometry is a composite solid therefore increments **both** `composite_solids_skipped` (once per composite LoD column) and `buildings_without_solid_skipped` (once, because it emitted no solid). This co-occurrence is intended.

## Geometry mapping (W-M1)

CityParquet stores a CityJSON `Solid` as WKB `PolyhedralSurfaceZ` (type 1015); `wkb_read::wkb_to_geometry` decodes it to `DecodedKind::PolyhedralSurface(faces)` where `faces: Vec<Vec<Vec<usize>>>` = faces → rings → indices into `DecodedGeometry.coords`. Emit:

```xml
<bldg:lod2Solid>
  <gml:Solid>
    <gml:exterior>
      <gml:CompositeSurface>
        <!-- one gml:surfaceMember per face -->
        <gml:surfaceMember>
          <gml:Polygon>
            <gml:exterior>
              <gml:LinearRing>
                <gml:posList srsDimension="3">X0 Y0 Z0 X1 Y1 Z1 … X0 Y0 Z0</gml:posList>
              </gml:LinearRing>
            </gml:exterior>
            <!-- ring[1..] → <gml:interior><gml:LinearRing>… for holes -->
          </gml:Polygon>
        </gml:surfaceMember>
      </gml:CompositeSurface>
    </gml:exterior>
  </gml:Solid>
</bldg:lod2Solid>
```

Rules:
- **Coordinates are world coordinates already.** WKB stores world coords (`wkb_write` applied `scale`/`translate` on write), so `DecodedGeometry.coords[i]` is `[x, y, z]` in the dataset CRS — emit directly into `posList`, no de-quantisation.
- **Re-close every ring.** `wkb_read` strips the WKB closing vertex on decode, so decoded rings are OPEN (last ≠ first). GML `gml:LinearRing` requires closed rings, so the writer appends the first coordinate again when serialising the `posList`. (This is the single most bug-prone detail; it gets its own unit test.)
- **`srsDimension="3"`** on `posList`; coordinates are `X Y Z` triples, space-separated, formatted so the value round-trips through the reader's `posList` parser (use the same float formatting the reader expects — plain decimal, full `f64` precision, e.g. `{}` / Rust default `Display`, matching `citygml/geometry.rs`'s parse).
- **LoD suffix.** The geometry column name `geometry_lod<k>` yields the element `bldg:lod<k>Solid` (e.g. `geometry_lod2_2` → LoD label per the crate's `Lod` column-suffix mapping → `bldg:lod2Solid`; confirm the exact major-LoD element name against CityGML 2.0 — `bldg:lod1Solid`/`bldg:lod2Solid`/`bldg:lod3Solid`/`bldg:lod4Solid` are the only valid ones, so a minor LoD like 2.2 maps to `lod2Solid`).
- A Building row may have Solids in **multiple** LoD columns; emit one `bldg:lod<k>Solid` per LoD that holds a PolyhedralSurface.

## CityModel skeleton, envelope, srsName

- **Root:** `<core:CityModel>` (default `xmlns` = CityGML 2.0 core `http://www.opengis.net/citygml/2.0`) with the `bldg` (`…/building/2.0`) and `gml` (`http://www.opengis.net/gml`) namespaces declared. Match the namespace URIs the reader accepts (see the committed `railway_lod3_fragment.gml` fixture header) so round-trip is exact.
- **Envelope:** `<gml:boundedBy><gml:Envelope srsName="…" srsDimension="3"><gml:lowerCorner>xmin ymin zmin</gml:lowerCorner><gml:upperCorner>xmax ymax zmax</gml:upperCorner></gml:Envelope></gml:boundedBy>`, from the unioned row bboxes. Omit `gml:boundedBy` entirely if the package has zero emitted geometry.
- **srsName:** derived from `CityParquetMetadata.crs`. **Round-trip constraint:** the emitted `srsName` MUST be a syntax the reader's `citygml::crs::resolve` accepts and resolves back to the same EPSG code, or CRS is silently lost on re-read. Before choosing the output form, read `citygml/crs.rs` to see which syntaxes `resolve` accepts (it documents "three EPSG syntaxes plus German AdV compound URNs"); extract the EPSG code from the package CRS (an OGC EPSG URL like `https://www.opengis.net/def/crs/EPSG/0/<code>`, or a PROJJSON `id.code`) and emit it in one of those accepted syntaxes (e.g. `urn:ogc:def:crs:EPSG::<code>`). A dedicated unit test asserts `resolve(emitted_srsName)` yields the original code. When the package has no CRS, omit `srsName` (a Building-only document with no envelope). Put the inverse-mapping helper (metadata CRS → srsName string) in `citygml/crs.rs` next to `resolve` for cohesion (D4).
- **Envelope ordering:** `gml:boundedBy` must appear **before** the `cityObjectMember`s in a valid CityGML document, but the envelope is only known after iterating all rows. Two acceptable implementations (implementer's choice, note which): (a) buffer member XML in memory, then write header + envelope + buffered members + footer; or (b) two passes over the tables (pass 1 accumulates the bbox union, pass 2 streams members). W-M1 datasets are small; (a) is simplest. Whichever is chosen, the emitted document MUST have `gml:boundedBy` before the first `cityObjectMember`.

## Error handling

- Missing/empty `metadata.json`, duplicate table names, unreadable table files → `Err` with context, mirroring `export`'s existing manifest checks (reuse the same error style).
- Malformed WKB → propagate the `wkb_read` error with the offending `id` in context.
- `overwrite` semantics match `export`/`convert`: refuse to clobber an existing output unless `overwrite` is set (confirm against how `export` handles its output today and match it).
- Skips (non-Building, no-Solid, composite) are **not** errors — they are counted in `WriteReport` and surfaced in the CLI report line.

## Testing (strict red-green TDD, real fixtures)

Every behavioural step starts with a failing test. No inline hand-authored CityGML/CityJSON — use the committed fixtures (`crates/cityparquet/tests/data/*.gml`, `tests/fixtures/*.city.jsonl`).

**Unit tests (`citygml/writer/*` modules):**
1. `geometry.rs`: a known `DecodedGeometry` (PolyhedralSurface, one cube face) → exact `gml:posList` string with the ring **re-closed** (last triple == first triple) and `srsDimension="3"`.
2. `geometry.rs`: a face with an interior ring → `gml:interior/gml:LinearRing` emitted after the exterior.
3. `document.rs`: envelope from a bbox union → exact `lowerCorner`/`upperCorner`; `srsName` from a PROJJSON/EPSG-URL CRS → expected string; no-CRS → no `srsName`, no envelope when empty.
4. `mod.rs`: `WriteReport` counts — a package with a non-Building row and a Building row with no Solid produces the right `non_building_skipped` / `buildings_without_solid_skipped`.

**Integration / round-trip oracle (the backbone):**
5. Take a reader-M1-supported `.gml` fixture (a Building with LoD Solid; the committed `railway_lod3_fragment.gml` or a small purpose-built LoD2-Solid fixture if the railway one has no plain Building Solid) → `convert` to a CityParquet package (via the existing pipeline) → `write_package` to `out.gml` → **re-read `out.gml` with the existing `citygml` reader**. **Scope of the assertion:** W-M1 emits geometry only — it deliberately drops attributes and semantic surfaces — so the oracle compares the **geometry projection**, NOT full feature equality: the set of Building `gml:id`s matches, and for each Building the LoD `Solid` boundary coordinates match the original within `f64` exactness (up to ring-rotation/closing conventions the writer normalises). Do **not** assert on attributes, semantics, or `gml:name`/`description` — those are expected to be absent after a W-M1 round-trip. This reuses the reader as the oracle; the only new test code is the geometry-projection comparison.
6. `export OUT.gml` CLI smoke test: convert a fixture, run the CLI export to `.gml`, assert the file exists, is non-empty, parses as XML, and its report line reports `buildings_written > 0`.

**Fixtures:** if no committed fixture is a bare `Building` + LoD Solid (the railway fragment's building may carry geometry only in `boundedBy`, which is W-M2), add ONE minimal real-derived CityGML 2.0 Building-with-`lod2Solid` fixture under `tests/data/` (documented provenance, like the existing fixtures) rather than hand-fabricating arbitrary coordinates.

## Codex external review

At the end of W-M1 (per the repo's milestone convention, see [[codex-external-review]]), run the Codex CLI review over the milestone diff and address/triage findings before tagging.

## Decisions / open items for the plan

- **D1 (CLI shape):** reuse `export` with `.gml` detection (recommended, in this spec) vs. a dedicated `export-citygml` subcommand. Recommended: reuse `export`.
- **D2 (envelope strategy):** buffer-members-in-memory (recommended for W-M1's small inputs) vs. two-pass. Implementer picks; document the choice.
- **D3 (xml layer):** reuse the reader's `citygml/xml.rs` approach (quick-xml `Writer` vs. hand-rolled string emit) for symmetry — confirm which the reader uses and mirror it.
- **D4 (srsName home):** inverse CRS-mapping helper in `citygml/crs.rs` (next to `resolve`) vs. `writer/document.rs`. Prefer `citygml/crs.rs` for cohesion.

These are small, local decisions; none blocks starting the plan.
