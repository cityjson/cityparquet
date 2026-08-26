# CityParquet design & format

This document describes the CityParquet data model as implemented by
`cityparquet-rs`: how a CityJSON / CityJSONSeq model is laid out as a
directory of Parquet files, and why. It is the readable companion to the
code; the round-trip test suite is the authoritative statement of what is
and isn't preserved.

CityParquet is a research format (part of a TU Delft paper on cloud-native
delivery of 3D city models), not yet a stable external standard.

## Goals

Store 3D city models in a **columnar, cloud-native** layout that can be
queried efficiently straight from object storage, while keeping enough
CityJSON semantics to reconstruct the source model. The layout is tuned for
analytical access — filter by object type, attribute, LoD, and bounding box;
prune with Parquet row-group statistics; read only the columns a query
touches — rather than for whole-model exchange.

Design principles:

- **one city object per row** in the main table;
- **WKB geometry**, so any GIS/database stack can read the geometry column;
- **separate columns** for the CityJSON information WKB cannot carry
  (semantics, appearance references, template instances);
- **typed attribute columns** inferred at import time, not a generic
  key/value table;
- **stable, nullable** structural columns — a column stays in the schema even
  when the dataset has no values for it;
- **sidecar Parquet files** for shared resources (materials, textures,
  geometry templates);
- **geometry separated from appearance**, following the OBJ / glTF lineage.

## Two profiles

| Profile           | Writes                                                                                   | Round-trip fidelity                                                                            |
| ----------------- | ---------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| **Core**          | main object table(s) + `metadata.json`                                                   | identity, hierarchy, attributes, per-LoD geometry, geometry semantics                          |
| **Compatibility** | the above **plus** `materials.parquet`, `textures.parquet`, `geometry_templates.parquet` | all of the above **plus** appearance definitions, texture UVs, and geometry-template instances |

Both profiles write the _same_ main-table column content (including the
`material`/`texture`/`template` references). The only difference is whether
the sidecar **definition** files are written. Under Core, references that
have nowhere to resolve to are dropped on export and counted; under
Compatibility they resolve against the sidecars.

A sidecar is only written when the dataset actually has that kind of data, so
a Compatibility conversion of a model with no textures simply omits
`textures.parquet` — asking for Compatibility never costs anything on data
that doesn't use it.

## Package layout

A CityParquet dataset is a **directory**, not a single file:

```text
mydataset/
  metadata.json              # package manifest (profile, LoDs, tables, sidecars)
  building.parquet            # one main object table per 1st-level CityObject
  bridge.parquet               # family actually present in the dataset
  materials.parquet          # Compatibility only, when present
  textures.parquet           # Compatibility only, when present
  geometry_templates.parquet # Compatibility only, when present
```

The main object table is split **by type**, unconditionally: one table per
**1st-level** (top-level) CityObject type — `building.parquet`,
`bridge.parquet`, `tunnel.parquet`, `cityfurniture.parquet`, and so on — for
schema clarity and type-selective queries. Per the CityJSON 2.0.1 spec's
1st-level vs 2nd-level city object
distinction, a 2nd-level object type (`BuildingPart`,
`BuildingInstallation`, `BuildingConstructiveElement`, `BuildingFurniture`,
`BuildingStorey`, `BuildingRoom`, `BuildingUnit`; the equivalent `Bridge*`
and `Tunnel*` types) is never given its own file — it is written into its
1st-level parent's table instead (all of the above into `building.parquet`;
the `Bridge*` family into `bridge.parquet`; the `Tunnel*` family into
`tunnel.parquet`). Every other type is already 1st-level and keeps its own
file. The `object_type` column (dictionary-encoded — see below) is
unaffected by this grouping and still carries each row's actual type, so a
query against `building.parquet` can still distinguish `Building` rows from
`BuildingPart` rows within it. `metadata.json` lists whichever tables were
written; readers consult the manifest, never the directory listing.

## Main object table

One row per `CityObject` — a parent `Building` and each of its
`BuildingPart` children each get their own row. Columns are written in a
fixed order: reserved structural columns first, then geometry, then
appearance/template references, then the inferred attribute columns.

### Reserved columns

| Column                                                  | Arrow type                                                     | Notes                                                                    |
| ------------------------------------------------------- | -------------------------------------------------------------- | ------------------------------------------------------------------------ |
| `id`                                                    | `Utf8` (non-null)                                              | CityObject id, preserved verbatim end-to-end                             |
| `feature_id`                                            | `Utf8`                                                         | CityJSONSeq feature grouping id                                          |
| `object_type`                                           | `Dictionary<Int32, Utf8>` (non-null)                           | dictionary-encoded — few distinct values over many rows                  |
| `parents`                                               | `List<Utf8>`                                                   | parent ids                                                               |
| `children`                                              | `List<Utf8>`                                                   | child ids                                                                |
| `children_roles`                                        | `List<Utf8>`                                                   | one role per child, from CityJSON `children_roles`                       |
| `bbox`                                                  | `Struct<xmin,ymin,zmin,xmax,ymax,zmax: Float64>`               | see below                                                                |
| `geometry` _or_ `geometry_lod<k>`                       | `Binary` (WKB)                                                 | one column per LoD                                                       |
| `geometry_properties` _or_ `geometry_properties_lod<k>` | `Struct<type, surfaces, face_semantics, shells>`               | semantics WKB can't carry                                                |
| `material`                                              | JSON                                                           | surface→material map into `materials.parquet`                            |
| `texture`                                               | JSON                                                           | surface→texture map into `textures.parquet`                              |
| `template`                                              | `Struct<id: Int64, point: Binary, transformationMatrix: JSON>` | geometry-instance data; `id` matches `geometry_templates.parquet`'s `id` |
| `other`                                                 | JSON                                                           | source fields not otherwise mapped                                       |

Every field carries self-describing metadata: a `cityparquet:role` key
(`reserved` / `attribute` / `extension`) and, on geometry columns, a
`cityparquet:lod` key. The metadata column lists in `metadata.json` are
derived from this field metadata, so they can never drift from the schema.

Geometry columns are tagged with the **geoarrow.wkb** Arrow extension type
and carry the dataset CRS in their extension metadata, so GeoParquet-aware
readers pick them up automatically.

JSON columns are `Utf8` tagged with the **arrow.json** extension type.

### Attributes

Each source attribute becomes its own typed column, inferred from the data at
import:

| Source value                      | Column type                          |
| --------------------------------- | ------------------------------------ |
| boolean                           | `Boolean`                            |
| integer                           | `Int64`                              |
| floating point                    | `Float64`                            |
| string                            | `Utf8`                               |
| confidently date/time-like string | `Date` / `Time` / `Timestamp`        |
| array of strings                  | `List<Utf8>`                         |
| object / heterogeneous array      | JSON                                 |
| inconsistent sampled types        | promoted if safe, else `Utf8` / JSON |

Reserved columns win any name clash. CityJSON extension attributes (`+name`)
are renamed to `ex_name` and tagged with the `extension` role. The inferred
attribute-column list is recorded in metadata so a reader can tell source
attributes from structural columns. Where a source object carries both an
unmapped top-level member and an attribute of the same name, the attribute
wins: the member is dropped from `other` with a warning, so the writer never
emits an `other` entry that duplicates an attribute.

### Geometry

Boundaries are encoded to **little-endian ISO-WKB** in the dataset CRS
(CityJSON integer vertices × `transform` are applied on write). The mapping:

| CityJSON geometry                   | WKB                                     |
| ----------------------------------- | --------------------------------------- |
| `MultiPoint`                        | `MultiPointZ`                           |
| `MultiLineString`                   | `MultiLineStringZ`                      |
| `MultiSurface` / `CompositeSurface` | `MultiPolygonZ`                         |
| `Solid`                             | `PolyhedralSurfaceZ` (type 1015)        |
| `MultiSolid` / `CompositeSolid`     | `GeometryCollectionZ`                   |
| `GeometryInstance`                  | `template` struct + referenced template |

The WKB carries geometry only. Semantic surfaces, the `values` surface→
semantics mapping, material/texture maps, and the source vertex-index
structure all live in the sibling `geometry_properties` / `material` /
`texture` columns.

**LoD is expressed by column layout, and every LoD is suffixed — including
LoD0**: a dataset with LoD 1 and LoD 2.2 gets `geometry_lod1_0` +
`geometry_properties_lod1_0` and `geometry_lod2_2` +
`geometry_properties_lod2_2`; a dataset with LoD 0 gets `geometry_lod0_0` +
`geometry_properties_lod0_0` the same way (spec "Levels of detail" — "a suffix
always carries a minor" and "there is no un-suffixed `geometry`... column").
The LoD lives only in the column name, never re-stored as a value: a source
`"1"` and a source `"1.0"` both map to `geometry_lod1_0` and export in the
canonical `"1.0"` spelling. A dataset with no LoD labels at all (no analysis
geometry — only `GeometryInstance`s, or none) uses a single un-suffixed
`geometry` / `geometry_properties` pair (all-null), a separate fallback that
does not apply once any LoD is present. A second geometry for the same
`(object, LoD)` is skipped and counted.

**LoD0 synthesis** (`src/lod0.rs`): when an object has no source LoD0, the
writer can synthesise a footprint from its lowest higher LoD — semantics-first
(`GroundSurface` faces), else a geometric fallback (downward-facing faces →
2D union → Z re-drape → `MultiPolygonZ`). The footprint is
exported as a real `lod:"0.0"` geometry, landing in `geometry_lod0_0` like any
other LoD. The CLI enables it by default (`--no-lod0` to disable); the
library `ConvertOptions` is source-faithful unless `generate_lod0` is set.

### Bounding box & spatial ordering

Every row has a 3D `bbox` in the geometry CRS. `bbox` is the union of the
object's own geometry bboxes across every LoD it carries, a cycle-guarded
recursive union over its whole descendant subtree, and the source's declared
extent (`geographicalExtent`) when present; it is null only when nothing in the
object's subtree has geometry and the source declared no extent. This ensures a
consumer pruning on a parent's `bbox` never misses geometry held by its
descendants, and never ends up narrower than a source-declared extent. Null
bboxes don't enter Parquet min/max statistics, so row-group pruning stays
sound.

Rows can optionally be reordered along a **2D Hilbert curve** over the
bbox centroid (`x`/`y` only — city models are height-thin, so `z` would waste
curve resolution), clustering spatially-near features into the same or
adjacent row groups. This improves bbox row-group pruning for windowed
spatial reads. It buffers the whole dataset in memory to sort, so it is
opt-in.

### Appearance

Appearance is stored separately from geometry. The main table's `material`
and `texture` columns keep the CityJSON theme shape
(`{"<theme>": {"values": ...}}` or `{"value": n}`) with two normalisations:

- integer indices are **dataset-global sidecar row ids** — definitions are
  deduplicated across every feature by canonical JSON content — not the
  feature-local indices CityJSONSeq uses;
- texture UV indices are replaced **inline** by the actual `[u, v]` pair
  (`[t, [u,v], [u,v], ...]`), so the main table is self-contained and no
  stored UV pool is needed.

On export under Compatibility, the global ids are sliced back to a
feature-local subset and the inlined UV pairs are re-interned into a
feature-local `vertices-texture` pool — the exact inverse of the import
rewrite.

`materials.parquet` and `textures.parquet` carry one dataset-global
definition per row; well-known CityJSON members become typed columns and
anything else is preserved under a JSON `other` column. The `id` column is
`Int64`, written as the dense `0..n` row index but resolved **by value**: a
geometry's `material`/`texture` reference matches the `id`, never the row
position. Readers require only that ids are non-null and unique, because
merging two packages offset-shifts a whole sidecar's ids (`dst_max + 1 -
src_min`) and may leave gaps — the same rule that applies to
`geometry_templates.parquet`, and the reason all three sidecar ids are
integers.
One deliberate normalisation: the numeric material scalars
(`ambientIntensity`/`transparency`/`shininess`) round-trip through `Float64`,
so an integer literal `1` reads back as `1.0` — value-exact, not
literal-exact.

### Geometry templates

Reusable templates live in `geometry_templates.parquet`, one per row, using
the same WKB-plus-`geometry_properties` strategy as the main table, **per-LoD
suffixed exactly like the main object table's own geometry and appearance
columns**: `geometry_lod*`/`geometry_properties_lod*`/`material_lod*`/
`texture_lod*`, one column set per LoD present among the templates being
rendered. A template row populates exactly the column set matching its own
LoD and leaves every other LoD's columns null — sparse by construction, like
the main table. There is no `lod` column (the column name already carries
it, just as in the main table) and no `other` column (a geometry template is
a plain geometry — WKB + properties + appearance — with no members left over
to preserve). Template vertices are **raw floats** — CityJSON
`vertices-templates` are _not_ subject to the dataset transform — so they are
interned by exact `f64` bit pattern rather than through the quantised
transform, and a template's `geometry_lod*` carries no `geoarrow.wkb`/CRS
tagging: template coordinates are in the template's own local frame, exempt
from the file CRS.

Its `id` is a `BIGINT`, written as the template's ordinal position and
matching the main-table `template.id` that references it. An integer rather
than a label because sidecar ids are renumbered by an integer offset when
packages merge, exactly as `materials.parquet` and `textures.parquet` are — a
string id could not be offset-shifted, and would need its own collision
strategy. That leaves nowhere for a source's own template identifier to
survive, so the optional `name` column holds it; it is null for CityJSON
sources, whose `geometry-templates.templates` is a bare array with no
identifiers. Readers must resolve `template.id` by matching the `id` value,
never by row position: a merged package's ids do not start at zero.

An object that instantiates a template stores the reference point (WKB
`PointZ`) and `transformationMatrix` in its `template` column.

## Dataset metadata

Dataset-level metadata is written both to the Parquet file's key/value
metadata and to `metadata.json` for package-level discovery. Keys include
`cityparquet_version`, `source_format`/`source_version`, `crs` (PROJJSON),
`transform`, `extensions`, `attributes` (the inferred attribute-column list;
any column not named here is a reserved structural column, so no separate
`reserved_columns` key is written), `default_geometry`, `bbox_column`,
`sidecar_files`, `source_metadata` (the source header `metadata`, verbatim —
with the one exclusion described below), `appearance_defaults`, and `other`
(free-form producer metadata). A GeoParquet `geo` key is derived so GeoParquet
readers recognise the geometry columns.

`crs` is tri-state, following GeoParquet: PROJJSON when known, an explicit
`null` when the file holds CRS-bearing coordinates whose CRS is unknown or
unresolvable, and absent only for a file with no CRS-bearing coordinate at all
(the `geometry_templates.parquet` sidecar, an attributes-only object table).
A source carrying CRS-bearing coordinates but declaring no resolvable CRS
therefore converts to `crs: null` plus a conversion diagnostic on
`ConvertReport::crs_diagnostic` (the CLI prints it as a `warning:`) — the
writer neither guesses nor omits the key, since an absent `crs` is read as
OGC:CRS84 and would silently mis-georeference the data. Such a package exports
with no `referenceSystem` at all, matching the source, and declares no
`proj:*` STAC fields. The CLI's `--crs EPSG:<code>` is the operator's explicit
declaration for such a source (a no-op for a source that declares its own
CRS), which makes the CRS resolvable _before_ the writer runs; when it is
actually applied, `other` carries `crs_source: "operator-supplied"` so the
footer never implies the source declared a CRS it did not carry.

**The one exclusion from `source_metadata`'s verbatim passthrough** follows
from that: an applied `--crs` is injected into the _in-memory_ header so the
scan can resolve it, and is removed again before the header's `metadata` is
written out — otherwise the passthrough would assert the source declared a CRS
it never carried, exactly the untruth `crs_source` exists to prevent. Nothing
else is ever removed, and for a source that declared its own CRS (including
one input of several in a merge — `merge_sources` enforces a single shared
CRS, so one declaring input makes the merged CRS source-declared) the
`referenceSystem` stays put and no `crs_source` is stamped.

A geographic (degree-valued) CRS is refused wherever it comes from — a CityJSON
`referenceSystem`, a CityGML `srsName`, or `--crs` — and refused at scan time,
before any output is touched. Nothing here reprojects and coordinates are
quantised at millimetre scale, so a degree coordinate (0.001° ≈ 111 m) would be
destroyed by the encoding: converting such a source "successfully" would write
a corrupt package and report a success, which is worse than any refusal. The
known-geographic EPSG list is common-but-not-exhaustive (an unlisted geographic
code is a documented residual limitation): the writer refuses what it can
recognise and never guesses from coordinate magnitudes.

`metadata.json` (the `PackageManifest`) is the **authoritative** description
of a package: profile, LoDs, the list of tables, and the list of sidecar
files actually written. Export reads the manifest, not the directory — a
sidecar file present on disk but absent from the manifest is ignored, and one
listed but missing is an error, never a silent drop.

`metadata.json` is a **STAC Item** (the 3D city models `city3d:*` extension)
describing that one package — see `crates/core/src/stac/`. A
dataset-level `collection.json` (a STAC **Collection** curating _multiple_
CityParquet packages/tiles into one aggregated dataset) is **not yet
implemented** — it needs a multi-package conversion workflow this CLI doesn't
yet have (`convert` writes one package per run); tracked as a follow-up.

## Round-trip semantics

The round-trip `source → package → exported CityJSON` is checked for
**semantic** equality (not byte equality) by the `compare` tool. Preserved
under both profiles:

- object ids, hierarchy (parents/children as sets), and type;
- typed attributes (timestamps compared as UTC instants);
- per-LoD geometry within quantisation tolerance;
- geometry semantics (boundary trees, per-surface material/texture refs).

Compatibility additionally preserves appearance definitions + UVs and
geometry-template definitions + instances, resolved through the sidecars
rather than by raw index (feature-local index numbering is an implementation
detail, exactly as vertex indices are compared as coordinates, not identity).

Documented exclusions:

- **Structurally degenerate rings** — a ring left with fewer than 3 entries
  after closure normalisation is dropped on write, with semantics realigned.
  This is applied identically on both comparison sides, and independently
  re-implemented in the comparator so a writer bug can't hide behind a shared
  normalisation.
- **Coordinate-degenerate rings** — rings whose indices are distinct but all
  dequantise to the same coordinate (a real 3DBAG occurrence) are likewise
  dropped on both sides.
- Unknown per-object members outside the round-tripped data model (`children_roles`
  IS now round-tripped and compared, G5).
- Under **Core only**: appearance definitions, per-geometry material/texture
  refs, and geometry-template definitions/instances — dropped on export and
  counted (they are present under Compatibility).

## Status & known limitations

Implemented and tested against real CityJSON and CityGML fixtures through
milestones M1–M5 (schema, writer, reader/round-trip, Compatibility profile,
benchmark suite), and exercised over the 30-dataset published corpus the read
benchmark uses. Current limitations:

- `Solid` geometry is WKB `PolyhedralSurfaceZ` (type 1015), which some
  geometry-auto-decoding GeoParquet readers (GeoPandas, DuckDB `spatial`) do
  not yet support; `MultiSurface`-derived columns decode fine there.
- The `geo` metadata omits `crs` PROJJSON until PROJJSON support lands, so
  readers that consult it assume `OGC:CRS84` while coordinates are actually in
  the source CRS (which _is_ recorded, as an OGC CRS URL, in the `crs` KV
  entry).
- The `other` column is currently always null (`children_roles` is now populated, G5).
