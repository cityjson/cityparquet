# CityGML 2.0 reader/writer — supported scope & known limitations

Status as of 2026-07-17. The native CityGML 2.0 reader (`crates/core/src/citygml/`)
and writer round-trip 3D city models through the CityParquet package. This
document records what is supported and what remains, for the paper's scope
section and future work. The **package round-trip is lossless** for everything
listed as supported — where the emitted CityGML is not strictly XSD-valid, the
lenient reader still reads it back identically.

## Supported

**Reader (CityGML → CityJSON/CityParquet):**

- `bldg:Building` with LoD1/LoD2 `gml:Solid`/`gml:CompositeSolid`, semantic
  surfaces (`bldg:boundedBy`) referenced inline or by `xlink:href` (CG-1),
  including nested `bldg:opening` Door/Window as their own semantic surfaces.
- The boundedBy-only case (no `lodNSolid`) as a `MultiSurface` with semantics.
- `BuildingPart` (`consistsOfBuildingPart`) and `{outer,interior}Building­Installation`
  (`BuildingInstallation`/`IntBuildingInstallation`) as child objects (CG-5).
- Generic 1st-level **non-building** objects (CG-7): WaterBody, LandUse,
  CityFurniture, SolitaryVegetationObject, PlantCover, Bridge, Tunnel,
  GenericCityObject, CityObjectGroup, Road, Railway, Square (→TransportSquare) —
  with `lodNSolid`, `lodNMultiSurface`, and `lodNGeometry` (dispatched by inner
  gml type), `gen:` generic attributes, and typed module leaf-text properties.
- Materials (`app:X3DMaterial`) + textures (`app:ParameterizedTexture`), feature-
  local and **CityModel-level** (`app:appearanceMember`, CG-3); reversed
  `gml:OrientableSurface` texture UVs kept aligned (CG-2).
- Building attributes (typed `bldg:` + `gen:` generics).

**Writer (CityParquet → CityGML), scope W-M1…W-M5:**

- `Building`/`BuildingPart` with LoD solids/CompositeSolid, semantic surfaces,
  attributes, materials, textures.

## Deferred / not yet supported

### Needs a cross-cutting `cityparquet-schema` (Arrow) change

A CityObject's non-geometry members are encoded from **fixed columns**, so these
need a new column + encoder/decoder work (not just reader/writer):

- **`bldg:address` (CG-4)** — CityJSON free-form `address` array (xAL on the
  CityGML side). No address column today.
- **`ImplicitGeometry` ↔ CityJSON GeometryInstance / geometry-templates (CG-8)** —
  template plumbing.

### Feasible as reader/writer only (NO Arrow-schema change)

Geometry `semantics` is stored **verbatim as JSON**, so extra members round-trip
for free:

- **Openings under `bldg:opening` (CG-6, main item)** — CityJSON models openings
  via semantic-surface `parent`/`children`. Today the reader flattens Door/Window
  as sibling surfaces (no link) and the writer emits them as top-level
  `bldg:{type}` (XSD-invalid, but package-lossless). Fix = reader records each
  surface's parent index + emits `{type, parent?, children?}`; writer nests a
  child surface under `bldg:opening`.

### Writer round-trip for the reader-only features above

The reader ingests these but the CityGML **writer** does not yet emit them (they
still round-trip CityGML→parquet→**CityJSON**, just not →CityGML):

- Non-building 1st-level objects (CG-7 writer side; `non_building_skipped`).
- BuildingInstallation emission (CG-5 writer side).
- CityModel-level appearance / non-building appearance emission.

### Other documented limitations

- Non-building **semantic surfaces / nested parts / appearance** on non-building
  objects (reader reads geometry + attributes only).
- `ReliefFeature` not mapped (only a TIN maps cleanly to `TINRelief`;
  raster/breakline/mass-point reliefs would misclassify).
- `MultiSolid` cannot be written (CityGML 2.0 `Building` has no `lodNMultiSolid`
  slot — a genuine format asymmetry, `multi_solids_skipped`).
- **Multi-LoD semantics**: the reader builds ONE building-wide `surfaces` array,
  so at most one LoD per building carries `boundedBy` semantics.
- XSD polish: canonical element ordering, `xs:date`/`gYear` lexical validity.
- ADE / foreign elements sharing a standard local name (namespace-agnostic
  object/geometry matching); `TexCoordGen` / texture UV seams; cross-building
  shared/external geometry; coordinate-magnitude CRS sniffing.

## Provenance

Gap-filling campaign (CG-1…CG-8), 2026-07-16/17. Merged & `gpt-5.6-sol`-reviewed:
CG-1 `6e75c34`, CG-2 `01f5d79`, CG-3 `66e2cee`, CG-7 `f23025f`, CG-5 `46169cf`.
Deferred by the author: CG-4, CG-6, CG-8.
