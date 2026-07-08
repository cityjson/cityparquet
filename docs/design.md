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

| Profile | Writes | Round-trip fidelity |
|---|---|---|
| **Core** | main object table(s) + `metadata.json` | identity, hierarchy, attributes, per-LoD geometry, geometry semantics |
| **Compatibility** | the above **plus** `materials.parquet`, `textures.parquet`, `geometry_templates.parquet` | all of the above **plus** appearance definitions, texture UVs, and geometry-template instances |

Both profiles write the *same* main-table column content (including the
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
  cityobjects.parquet        # the main object table (Single layout)
  materials.parquet          # Compatibility only, when present
  textures.parquet           # Compatibility only, when present
  geometry_templates.parquet # Compatibility only, when present
```

Under the **by-type** table layout the single `cityobjects.parquet` is split
into one table per object type — `cityobjects_building.parquet`,
`cityobjects_transportation.parquet`, and so on — for schema clarity and
type-selective queries. `metadata.json` lists whichever tables were written;
readers consult the manifest, never the directory listing.

## Main object table

One row per `CityObject` — a parent `Building` and each of its
`BuildingPart` children each get their own row. Columns are written in a
fixed order: reserved structural columns first, then geometry, then
appearance/template references, then the inferred attribute columns.

### Reserved columns

| Column | Arrow type | Notes |
|---|---|---|
| `id` | `Utf8` (non-null) | CityObject id, preserved verbatim end-to-end |
| `feature_id` | `Utf8` | CityJSONSeq feature grouping id |
| `object_type` | `Dictionary<Int32, Utf8>` (non-null) | dictionary-encoded — few distinct values over many rows |
| `parents` | `List<Utf8>` | parent ids |
| `children` | `List<Utf8>` | child ids |
| `children_roles` | `List<Utf8>` | currently always null |
| `bbox` | `Struct<xmin,ymin,zmin,xmax,ymax,zmax: Float64>` | see below |
| `geometry` *or* `geometry_lod<k>` | `Binary` (WKB) | one column per LoD |
| `geometry_properties` *or* `geometry_properties_lod<k>` | JSON | semantics WKB can't carry |
| `material` | JSON | surface→material map into `materials.parquet` |
| `texture` | JSON | surface→texture map into `textures.parquet` |
| `template` | `Struct<id: Utf8, point: Binary, transformationMatrix: JSON>` | geometry-instance data |
| `other` | JSON | source fields not otherwise mapped |

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

| Source value | Column type |
|---|---|
| boolean | `Boolean` |
| integer | `Int64` |
| floating point | `Float64` |
| string | `Utf8` |
| confidently date/time-like string | `Date` / `Time` / `Timestamp` |
| array of strings | `List<Utf8>` |
| object / heterogeneous array | JSON |
| inconsistent sampled types | promoted if safe, else `Utf8` / JSON |

Reserved columns win any name clash. CityJSON extension attributes (`+name`)
are renamed to `ex_name` and tagged with the `extension` role. The inferred
attribute-column list is recorded in metadata so a reader can tell source
attributes from structural columns.

### Geometry

Boundaries are encoded to **little-endian ISO-WKB** in the dataset CRS
(CityJSON integer vertices × `transform` are applied on write). The mapping:

| CityJSON geometry | WKB |
|---|---|
| `MultiPoint` | `MultiPointZ` |
| `MultiLineString` | `MultiLineStringZ` |
| `MultiSurface` / `CompositeSurface` | `MultiPolygonZ` |
| `Solid` | `PolyhedralSurfaceZ` (type 1015) |
| `MultiSolid` / `CompositeSolid` | `GeometryCollectionZ` |
| `GeometryInstance` | `template` struct + referenced template |

The WKB carries geometry only. Semantic surfaces, the `values` surface→
semantics mapping, material/texture maps, and the source vertex-index
structure all live in the sibling `geometry_properties` / `material` /
`texture` columns.

**LoD is expressed by column layout**: a dataset with LoD 1 and LoD 2.2 gets
`geometry_lod1` + `geometry_properties_lod1` and `geometry_lod2_2` +
`geometry_properties_lod2_2`. A dataset with no LoD labels at all gets a
single un-suffixed `geometry` column instead. A second geometry for the same
`(object, LoD)` is skipped and counted.

### Bounding box & spatial ordering

Every row has a 3D `bbox` in the geometry CRS. For an object whose geometry
lives on its descendants (a `Building` with geometry only on its
`BuildingPart`s), `bbox` is the **union of the descendants' geometry**; it is
null only when nothing in the object's subtree has geometry. Null bboxes
don't enter Parquet min/max statistics, so row-group pruning stays sound.
When a row carries multiple LoDs, `bbox` is taken from the highest LoD.

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
the dense `0..n` row index as `Int64` (readers validate `id == position`).
One deliberate normalisation: the numeric material scalars
(`ambientIntensity`/`transparency`/`shininess`) round-trip through `Float64`,
so an integer literal `1` reads back as `1.0` — value-exact, not
literal-exact.

### Geometry templates

Reusable templates live in `geometry_templates.parquet`, one per row, using
the same WKB-plus-`geometry_properties` strategy as the main table. Template
vertices are **raw floats** — CityJSON `vertices-templates` are *not* subject
to the dataset transform — so they are interned by exact `f64` bit pattern
rather than through the quantised transform. A template row also carries its
`lod` inside `geometry_properties` (templates have no per-LoD column name to
carry it). Its `id` is the template's position, matching the main-table
`template.id`. An object that instantiates a template stores the reference
point (WKB `PointZ`) and `transformationMatrix` in its `template` column.

## Dataset metadata

Dataset-level metadata is written both to the Parquet file's key/value
metadata and to `metadata.json` for package-level discovery. Keys include
`cityparquet_version`, `source_format`/`source_version`, `crs` (PROJJSON),
`transform`, `extensions`, `attribute_columns`, `reserved_columns`,
`default_geometry`, `bbox_column`, `sidecar_files`, `source_metadata` (the
source header `metadata`, verbatim), and `appearance_defaults`. A GeoParquet
`geo` key is derived so GeoParquet readers recognise the geometry columns.

`metadata.json` (the `PackageManifest`) is the **authoritative** description
of a package: profile, LoDs, the list of tables, and the list of sidecar
files actually written. Export reads the manifest, not the directory — a
sidecar file present on disk but absent from the manifest is ignored, and one
listed but missing is an error, never a silent drop.

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
- `children_roles` and unknown per-object members (outside the round-tripped
  data model).
- Under **Core only**: appearance definitions, per-geometry material/texture
  refs, and geometry-template definitions/instances — dropped on export and
  counted (they are present under Compatibility).

## Status & known limitations

Implemented and tested against real CityJSON fixtures and 3DBAG tiles through
milestones M1–M5 (schema, writer, reader/round-trip, Compatibility profile,
benchmark suite). Current limitations:

- `Solid` geometry is WKB `PolyhedralSurfaceZ` (type 1015), which some
  geometry-auto-decoding GeoParquet readers (GeoPandas, DuckDB `spatial`) do
  not yet support; `MultiSurface`-derived columns decode fine there.
- The `geo` metadata omits `crs` PROJJSON until PROJJSON support lands, so
  readers that consult it assume `OGC:CRS84` while coordinates are actually in
  the source CRS (which *is* recorded, as an OGC CRS URL, in the `crs` KV
  entry).
- `children_roles` and `other` columns are currently always null.
