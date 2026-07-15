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
}

/// Counts mirroring the export report's drop-counter style.
#[derive(Debug, Default, PartialEq)]
pub struct WriteReport {
    pub buildings_written: usize,
    /// Rows skipped because object_type is not Building (W-M1 scope).
    pub non_building_skipped: usize,
    /// Building rows with no emittable Solid in any major LoD (only
    /// MultiSurface / MultiSolid / no geometry) — skipped entirely.
    pub buildings_without_solid_skipped: usize,
    /// Individual LoD columns skipped because the WKB was MultiSolid/
    /// CompositeSolid (GeometryCollection) — deferred to W-M2.
    pub composite_solids_skipped: usize,
    /// LoD columns skipped because they collided on a major LoD already
    /// emitted for that building (see "LoD major mapping" below), or mapped
    /// to an unrepresentable LoD (0, >4, or the lodless `geometry` column).
    pub lod_columns_skipped: usize,
}

pub fn write_package(opts: &WriteOptions) -> Result<WriteReport>;
```

**Overwrite (Codex review):** the existing CityJSON `export` has **no** overwrite option and uses `File::create` (truncates). W-M1 matches that real behaviour — no `overwrite` field — rather than claiming a semantics `export` does not have. Adding create-new/overwrite protection is a deliberate future cross-cutting item, out of W-M1 scope.

**Reads the geometry-properties column too (Codex review):** because the WKB `PolyhedralSurfaceZ` flattens a Solid's shells, the writer MUST read the paired `geometry_properties_lod<k>` JSON column to recover the shell partition (`type == "Solid"` + `solid_shell_faces`) and validate it against the decoded face count. See "Geometry mapping".

### CLI wiring

Extend the existing `export` command's output-extension detection with a `.gml` branch that routes to `citygml::writer::write_package` — **not** through the CityJSON `export` function. `export.rs::output_format` is currently **private** (Codex review), so the dispatch happens at the CLI layer (`cityparquet-cli/src/main.rs`, the `Export` arm): detect a `.gml` output extension there and call `write_package`, leaving the `.city.jsonl`/`.city.json` arm calling the existing `export`. No change to `output_format`'s visibility is required. So:

```bash
cargo run -p cityparquet-cli -- export PACKAGE_DIR OUT.gml
```

writes CityGML. The `.city.jsonl`/`.city.json` paths are untouched. (Decision D1 below records the alternative of a separate subcommand; reusing `export` is the recommended default for UX consistency.)

## Data flow

```
PACKAGE_DIR/
  metadata.json  ──► PackageManifest.tables            (manifest ONLY)
  <tables>.parquet ─► CityParquetMetadata (crs, transform) from the FIRST table's
                      Parquet FOOTER via reader::cityparquet_metadata()
        │
        │  for each table in manifest.tables (first table's meta/schema authoritative —
        │    verify later tables' version + rendered schema match, as export does):
        │    open with ParquetRecordBatchReaderBuilder + CityParquetReaderBuilder
        │    for each RecordBatch, each row:
        │       object_type (dict Utf8) == "Building"?  ── no ──► non_building_skipped++
        │       id (Utf8, validated NCName + unique), and per LoD column:
        │         geometry_lod<k> (Binary WKB) + geometry_properties_lod<k> (JSON)
        │            │
        │            │  wkb_read::wkb_to_geometry(blob) -> DecodedGeometry
        │            │     { coords: Vec<[f64;3]> (WORLD coords), kind }
        │            ▼
        │       kind == PolyhedralSurface(faces) AND props.type == "Solid":
        │            partition faces into shells via props.solid_shell_faces,
        │            map column to major LoD (1..4); emit ONE <bldg:lod{major}Solid>
        │            (shell 0 = gml:exterior, shells 1+ = gml:interior);
        │            accumulate emitted coords into the envelope
        │       kind == GeometryCollection(..)         ──► composite_solids_skipped++ (W-M2)
        │       kind == MultiPolygon(..)/other/no props ─► not a W-M1 Solid: skip column
        │       major LoD collision / lod0 / lod>4 / lodless ─► lod_columns_skipped++
        │       (no emittable Solid in any major LoD)   ──► buildings_without_solid_skipped++
        ▼
   <CityModel> … <gml:boundedBy><gml:Envelope> … <cityObjectMember><bldg:Building> … </CityModel>
```

**Envelope from emitted geometry, not the row bbox (Codex review).** The per-row `bbox` column is a **superset** — it bounds *all* of a row's source geometry, including LoDs not emitted, skipped composites, and (if all rows were unioned) non-Building rows. The document envelope must bound exactly what was written, so it is accumulated from the **world coordinates of the solids actually emitted** (min/max over every `posList` coordinate). If no solid is emitted, no `gml:boundedBy` is written.

**Building emission rule:** a `bldg:Building` is emitted **only if** it has at least one emittable `bldg:lod<major>Solid`. A Building with zero emittable solids is skipped entirely (never an empty `bldg:Building`) and counted once in `buildings_without_solid_skipped`. The per-column counters (`composite_solids_skipped`, `lod_columns_skipped`) are independent diagnostics and may co-occur with `buildings_without_solid_skipped`.

**LoD major mapping (Codex review).** CityGML 2.0 `bldg:lod<n>Solid` exists only for majors **1..4** and has cardinality **0..1** per building. Each `geometry_lod<k>` column maps to its major LoD via a `Lod::major()` accessor (add it to `cityparquet-schema::types::Lod` if absent). Rules: emit at most one `bldg:lod<major>Solid` per major; if two columns share a major (e.g. `geometry_lod2` and `geometry_lod2_2` both → major 2), keep the **most detailed** (highest minor) deterministically and count the other in `lod_columns_skipped`; a column mapping to LoD 0, LoD > 4, or the lodless `geometry` column carrying a Solid is not representable as a `lodNSolid` and is counted in `lod_columns_skipped`. Emit the `bldg:lod<major>Solid` properties in **ascending** major order.

## Geometry mapping (W-M1)

CityParquet stores a CityJSON `Solid` as WKB `PolyhedralSurfaceZ` (type 1015); `wkb_read::wkb_to_geometry` decodes it to `DecodedKind::PolyhedralSurface(faces)` where `faces: Vec<Vec<Vec<usize>>>` = faces → rings → indices into `DecodedGeometry.coords`.

**Shell partitioning (Codex review — Critical).** WKB **flattens** a Solid's shells into one flat face list; the shell partition survives ONLY in the paired `geometry_properties_lod<k>` column (`type == "Solid"`, `solid_shell_faces`). A CityJSON/GML `Solid` is one **exterior** shell plus zero or more **interior** shells (cavities). Ignoring the partition and emitting all faces as one `gml:exterior` corrupts any multi-shell solid. The writer therefore reads the properties column, requires `type == "Solid"`, validates that the shell face counts sum **exactly** to the decoded face count (error with the row `id` otherwise), and emits shell 0 as `gml:exterior` and each remaining shell as its own `gml:interior`. Reuse export's shell-partition helpers (`export.rs::shell_faces_flat` / `shell_faces_nested`, extracting them to a shared location if needed) rather than re-deriving the partition. A single-shell solid (the common 3DBAG case) yields exactly one `gml:exterior`, no `gml:interior`.

```xml
<bldg:lod2Solid>
  <gml:Solid>
    <gml:exterior>                             <!-- shell 0 -->
      <gml:CompositeSurface>
        <!-- one gml:surfaceMember per face in this shell -->
        <gml:surfaceMember>
          <gml:Polygon>
            <gml:exterior>
              <gml:LinearRing>
                <gml:posList srsDimension="3">X0 Y0 Z0 X1 Y1 Z1 … X0 Y0 Z0</gml:posList>
              </gml:LinearRing>
            </gml:exterior>
            <!-- ring[1..] → <gml:interior><gml:LinearRing>… for polygon holes -->
          </gml:Polygon>
        </gml:surfaceMember>
      </gml:CompositeSurface>
    </gml:exterior>
    <!-- shells 1.. → one <gml:interior><gml:CompositeSurface>… each -->
  </gml:Solid>
</bldg:lod2Solid>
```

Note the two levels of exterior/interior: **shell** level (`gml:Solid` → exterior/interior *shells*) and **face** level (`gml:Polygon` → exterior/interior *rings* = holes). Both come from the data — shells from `solid_shell_faces`, rings from the decoded face's ring list.

Rules:
- **Coordinates are world coordinates already.** WKB stores world coords (`wkb_write` applied `scale`/`translate` on write), so `DecodedGeometry.coords[i]` is `[x, y, z]` in the dataset CRS — emit directly into `posList`, no de-quantisation.
- **Re-close every ring.** `wkb_read` strips the WKB closing vertex on decode, so decoded rings are OPEN (last ≠ first). GML `gml:LinearRing` requires closed rings, so the writer appends the first coordinate again when serialising the `posList`. (This is the single most bug-prone detail; it gets its own unit test.)
- **`srsDimension="3"`** on `posList`; coordinates are `X Y Z` triples, space-separated, formatted so the value round-trips through the reader's `posList` parser (use the same float formatting the reader expects — plain decimal, full `f64` precision, e.g. `{}` / Rust default `Display`, matching `citygml/geometry.rs`'s parse).
- **`gml:id`.** The `gml:Solid`/`gml:Polygon` etc. may carry generated `gml:id`s if needed; the Building's `gml:id` comes from the row `id` (see the id-validation rule under "Error handling"). Only `bldg:lod1Solid`..`bldg:lod4Solid` are valid CityGML 2.0 elements — LoD mapping is governed by the "LoD major mapping" rules above, not the raw column suffix.

## CityModel skeleton, envelope, srsName

- **Root (Codex review — unprefixed core).** A default `xmlns` does **not** bind a `core:` prefix, so `<core:CityModel>` with only a default namespace is invalid. Emit an **unprefixed** root, `<CityModel xmlns="http://www.opengis.net/citygml/2.0">`, and unprefixed `cityObjectMember`, with `xmlns:bldg="…/building/2.0"`, `xmlns:gml="http://www.opengis.net/gml"`, and `xmlns:xlink="http://www.w3.org/1999/xlink"` declared. This matches the committed reader fixtures (`railway_lod3_fragment.gml`, `savenow_ingolstadt_lod2.gml`), so round-trip is exact.
- **Envelope (from emitted coords):** `<gml:boundedBy><gml:Envelope srsName="…" srsDimension="3"><gml:lowerCorner>xmin ymin zmin</gml:lowerCorner><gml:upperCorner>xmax ymax zmax</gml:upperCorner></gml:Envelope></gml:boundedBy>`, computed from the world coordinates of the solids **actually emitted** (not the row `bbox` column). Omit `gml:boundedBy` entirely when zero solids were emitted. `srsName` is present on the envelope only when a CRS resolved (below); its absence does not suppress the envelope.
- **srsName (Codex review — strict EPSG only):** derived from `CityParquetMetadata.crs` (read from the first table's footer). Accept the CRS **only** when it is an OGC EPSG URL (`…/EPSG/0/<code>`) or a PROJJSON object with `id.authority == "EPSG"` and a numeric `id.code`; extract that code and build `urn:ogc:def:crs:EPSG::<code>`. **Validate** by calling `citygml::crs::resolve(built_srs)` and requiring `CrsResolution::Epsg(code)` for the same code — guaranteeing the CRS round-trips through the reader. If the code is a **geographic** CRS (e.g. 4326) or otherwise unsupported, `resolve` errors; W-M1 propagates that as an error (the 1 mm quantiser makes degrees meaningless, same stance as the reader) rather than silently mislabelling. When the package has **no** CRS, omit `srsName` only — the envelope is still emitted. Put the inverse-mapping helper (CRS metadata → validated srsName) in `citygml/crs.rs` next to `resolve` (D4).
- **Envelope ordering:** `gml:boundedBy` must appear **before** the `cityObjectMember`s, but the emitted-coordinate envelope is only known after iterating all rows. Implementer's choice (note which): (a) buffer member XML in memory, then write header + envelope + buffered members + footer; or (b) two passes (pass 1 emits members and accumulates the coordinate envelope; pass 2 writes header+envelope then the buffered/re-emitted members). W-M1 inputs are small; (a) is simplest. Either way the document MUST have `gml:boundedBy` before the first `cityObjectMember`.

## Error handling

- Missing/empty `metadata.json`, duplicate table names, unreadable table files → `Err` with context, mirroring `export`'s existing manifest checks (reuse the same error style).
- **Multi-table integrity (Codex review).** `export` treats the **first** table's footer metadata + rendered schema as authoritative and verifies every later table's `cityparquet_version` and full rendered schema match before decoding it. The writer MUST apply the same checks (share or reproduce export's helper) so it never silently serialises heterogeneous tables.
- **`gml:id` validity (Codex review).** A row `id` is arbitrary UTF-8 and not guaranteed unique across features; XML-escaping does not make it a valid `xs:ID`/NCName. Before writing, validate that each Building `id` is a syntactically valid NCName **and** unique across the document; on violation, return an id-specific `Err` (do **not** silently sanitise — that would break the id round-trip the format guarantees). (Real 3DBAG ids like `NL.IMBAG.Pand.0503100000013175-0` are valid NCNames, so this errors only on genuinely non-conforming data.)
- **Shell-count mismatch:** if `solid_shell_faces` does not sum exactly to the decoded face count, error with the offending `id` (corrupt/heterogeneous data), never guess.
- Malformed WKB → propagate the `wkb_read` error with the offending `id` in context.
- **Output file:** matches `export`'s current behaviour — `File::create` (truncate). No overwrite guard in W-M1 (see the Overwrite note under "Public interface").
- Skips (non-Building, no-Solid, composite, LoD-collision) are **not** errors — they are counted in `WriteReport` and surfaced in the CLI report line.

## Testing (strict red-green TDD, real fixtures)

Every behavioural step starts with a failing test. No inline hand-authored CityGML/CityJSON — use the committed fixtures (`crates/cityparquet/tests/data/*.gml`, `tests/fixtures/*.city.jsonl`).

**Unit tests (`citygml/writer/*` modules):**
1. `geometry.rs`: a known `DecodedGeometry` (PolyhedralSurface, one cube face) → exact `gml:posList` string with the ring **re-closed** (last triple == first triple) and `srsDimension="3"`.
2. `geometry.rs`: a face with an interior ring → `gml:interior/gml:LinearRing` (polygon hole) emitted after the exterior.
3. `geometry.rs` (**shell partitioning**): a two-shell solid (outer + one inner cavity) with `solid_shell_faces` → shell 0 as `gml:exterior`, shell 1 as `gml:interior`, faces assigned to the right shell; and a shell-count mismatch → error.
4. `document.rs`: envelope from **emitted coordinates** → exact `lowerCorner`/`upperCorner`; `srsName` from an EPSG-URL and from an EPSG PROJJSON CRS → the `urn:ogc:def:crs:EPSG::<code>` string, and `resolve(that)` == `Epsg(code)`; a geographic CRS (4326) → error; no CRS → no `srsName` but envelope still present.
5. `mod.rs`: `WriteReport` counts — a package with a non-Building row, a Building with no Solid, and a Building whose `geometry_lod2` and `geometry_lod2_2` both hold Solids → correct `non_building_skipped` / `buildings_without_solid_skipped` / `lod_columns_skipped` (one major-2 collision skipped).
6. `mod.rs` (**gml:id**): a package whose Building `id` is not a valid NCName (or duplicated) → id-specific `Err`.

**Integration / round-trip oracle (the backbone):**
7. Use the committed **`crates/cityparquet/tests/data/savenow_ingolstadt_lod2.gml`** fixture (three `bldg:Building` with `bldg:lod2Solid`, per Codex — no new fixture needed) → `convert` to a CityParquet package (existing pipeline) → `write_package` to `out.gml` → **re-read `out.gml` with the existing `citygml` reader**. **Scope of the assertion:** W-M1 emits geometry only — it deliberately drops attributes and semantic surfaces — so the oracle compares the **geometry projection**, NOT full feature equality: the set of Building `gml:id`s matches, and for each Building the LoD `Solid` boundary coordinates match the original within `f64` exactness (accounting for ring closing/rotation the writer normalises). Do **not** assert on attributes, semantics, or `gml:name`/`description` — those are expected absent after a W-M1 round-trip.
8. `export OUT.gml` CLI smoke test: convert the Ingolstadt fixture, run the CLI export to `.gml`, assert the file exists, is non-empty, parses as XML, and the report line reports `buildings_written == 3`.

**Fixtures:** no new fixture is required for the happy path — `savenow_ingolstadt_lod2.gml` covers Building + `lod2Solid`. For the interior-shell test (unit test 3), construct the `DecodedGeometry` + `solid_shell_faces` in-code from a small **real** two-shell solid if one exists in the fixtures, or drive it through a `convert` of a CityJSON `Solid` with two shells from `tests/fixtures/` — do not hand-fabricate arbitrary coordinates for the *round-trip* test (unit tests over a constructed `DecodedGeometry` value are fine, as they exercise pure serialisation, not parsing).

## Codex external review

At the end of W-M1 (per the repo's milestone convention, see [[codex-external-review]]), run the Codex CLI review over the milestone diff and address/triage findings before tagging.

## Decisions / open items for the plan

- **D1 (CLI shape):** reuse `export` with `.gml` detection (recommended, in this spec) vs. a dedicated `export-citygml` subcommand. Recommended: reuse `export`.
- **D2 (envelope strategy):** buffer-members-in-memory (recommended for W-M1's small inputs) vs. two-pass. Implementer picks; document the choice.
- **D3 (xml layer):** reuse the reader's `citygml/xml.rs` approach (quick-xml `Writer` vs. hand-rolled string emit) for symmetry — confirm which the reader uses and mirror it.
- **D4 (srsName home):** inverse CRS-mapping helper in `citygml/crs.rs` (next to `resolve`) vs. `writer/document.rs`. Prefer `citygml/crs.rs` for cohesion.

These are small, local decisions; none blocks starting the plan.
