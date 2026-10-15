# CityGML reader/writer — supported scope & known limitations

Status as of 2026-10-15. The CityGML 2.0 and 3.0 readers (the `citygml2` and
`citygml3` crates in `lib/citygml-rs`, chosen by the document's namespace and
adapted in `crates/core/src/citygml/`) and the same crates' CityGML 2.0 and
3.0 writers, driven by `export`, round-trip 3D city models through the
CityParquet package. This document records what is supported and what
remains, for the paper's scope section and future work. The **package
round-trip is lossless** for everything listed as supported, and the CityGML
written is valid against the CityGML schemas and its ADEs'.

## Supported

**Reader (CityGML → CityJSON/CityParquet).** On six PLATEAU files (building,
bridge, city furniture, transportation, vegetation, water body) and a
`tran:Track` fragment, the CityJSON it produces agrees with `citygml-tools
to-cityjson` on objects per type, geometry types and LoDs, semantic surfaces,
attribute names and types, addresses and appearance palettes
(`lib/citygml-rs/crates/citygml2/tests/oracle.rs`).

- Every CityGML 2.0 class CityJSON has a type for: Building, BuildingPart,
  installations, Room (→ BuildingRoom), BuildingFurniture; Bridge, BridgePart,
  BridgeConstructionElement (→ BridgeConstructiveElement), installations,
  BridgeRoom, BridgeFurniture; Tunnel, TunnelPart, installations, HollowSpace
  (→ TunnelHollowSpace), TunnelFurniture; WaterBody, LandUse, CityFurniture,
  SolitaryVegetationObject, PlantCover, GenericCityObject, CityObjectGroup;
  Road, Railway, Square (→ TransportSquare), and Track and
  TransportationComplex (→ Road, as CityJSON models them); TINRelief, read
  through its `dem:ReliefFeature`.
- Geometry: `lodNSolid` (`gml:Solid`/`gml:CompositeSolid`), `lodNMultiSurface`,
  `lodNGeometry` (dispatched by inner GML type, points and curves included),
  `lodNMultiCurve`, a transportation object's `lodNNetwork`, a building's
  `lod0FootPrint`/`lod0RoofEdge`, and a TIN's `gml:TriangulatedSurface`. Every
  LoD the document spells, LoD 4 included. A shell or MultiSurface member
  that is an `xlink:href` is a face, in the orientation of any
  `gml:OrientableSurface` around it; one to an identified
  `gml:CompositeSurface` or `gml:MultiSurface` stands for all of that
  aggregate's polygons, and a nested object (a storey, a room) resolves
  xlinks to polygons its parent or the parent's other children define.
- Semantic surfaces (`boundedBy`, `trafficArea`, `auxiliaryTrafficArea`)
  referenced inline or by `xlink:href`, with a Door or Window inside a wall's
  `opening` as that wall's child surface.
- Attributes: `gml:name`, `gml:description`, every text-only property of a
  CityGML module (numbers, booleans and number lists typed from a table of the
  2.0 schema), `gen:` generic attributes, and `relationToConstruction` for an
  installation (from its outer or interior class).
- Addresses (`address`), xAL leaves mapped to CityJSON's address members, with
  the `MultiPoint` location.
- `CityObjectGroup` members (`grp:groupMember`) with their roles, as the group's
  `children` and `children_roles`. A member nested in the group is the group's
  child in its feature; one referenced by `xlink:href` stays a feature of its
  own and names the group among its `parents`, wherever it sits in the
  document (the reader indexes the groups in its up-front pass).
- `gen:GenericAttributeSet`, as an attribute holding its members.
- `ImplicitGeometry` (`lodNImplicitRepresentation`): each relative geometry
  becomes a CityJSON template in the header's `geometry-templates`, coloured
  and textured by the CityModel-level appearance that targets it, and each
  `ImplicitGeometry` a `GeometryInstance` with its reference point and
  transformation matrix, whether its `relativeGMLGeometry` is inline or an
  `xlink:href` to a relative geometry another `ImplicitGeometry` writes inline,
  anywhere in the document. On the LoD 3
  Railway's trees, templates, their appearance and every instance agree with
  `citygml-tools to-cityjson`.
- Materials (`app:X3DMaterial`, unnamed ones included) and textures
  (`app:ParameterizedTexture`), feature-local and **CityModel-level**
  (`app:appearanceMember`), targeting polygons or the surface aggregates
  holding them; reversed `gml:OrientableSurface` texture UVs kept aligned.

**CityGML 3.0 reader.** It shares the 2.0 reader's geometry assembly,
appearance, implicit geometries, groups and ADE handling (`citygml-core`), and
reads 3.0's encoding of the same content. On four PLATEAU files (building,
bridge, transportation, vegetation) upgraded by `citygml-tools upgrade`, the
LoD 4 and CompositeSolid samples and the LoD 3 Railway's trees upgraded the
same way, and the OGC's storeys-and-units sample, the CityJSON it produces
agrees with `citygml-tools to-cityjson` as the 2.0 reader's does
(`lib/citygml-rs/crates/citygml3/tests/oracle.rs`), with two differences the
test states: where citygml4j writes a solid and a MultiSurface at one LoD the
reader keeps the solid, and a face a storey xlinks from a room's boundary
surface is not tagged with that surface.

- The 3.0 classes CityJSON has a type for, including OtherConstruction, the
  generic spaces (→ GenericCityObject), Waterway, BuildingConstructiveElement
  and its bridge and tunnel counterparts, Storey (→ BuildingStorey) and
  BuildingUnit, nested in their building as rooms and parts are; a storey's
  or unit's `xlink:href` to a room or unit nests nothing.
- Geometry: the core module's `lodNSolid`, `lodNMultiSurface`,
  `lodNMultiCurve` and `lod0Point`, GML 3.2 solids (`gml:Shell`), and
  `lodNImplicitRepresentation` (`relativeGeometry`).
- Boundary surfaces (`boundary`) with their `fillingSurface`s, a wall's doors
  and windows, as child surfaces. A transportation object's spaces
  (`trafficSpace`, `auxiliaryTrafficSpace`, `hole`, `section`,
  `intersection`) are folded into it: their boundary surfaces (traffic and
  auxiliary traffic areas, holes) and `marking`s become the object's own
  semantic surfaces, as CityJSON and CityGML 2.0 model a road.
- Attributes: as 2.0's, typed from the 3.0 schema; `gen:` generic attributes
  by their `gen:name` and `gen:value`; a construction's `con:height` whose
  status is `measured` as `measuredHeight`.
- Addresses in xAL 3, each part named as xAL 2 names it (`thoroughfareName`,
  `postcode`, …).
- ADEs through the `adeOf<Class>` hooks: the hook's container element is
  unwrapped, and its properties are the object's `+` attributes, declared on
  the CityJSON type of the object; an ADE class specialising a CityGML class
  goes to that class's table when its schema is read.

**Writers (CityParquet → CityGML 2.0 or 3.0).** `export` rebuilds each
feature as CityJSON and writes it with the `citygml2` writer, or the
`citygml3` one under `--citygml-version 3.0`: every class the version has,
each property in its content model's order, boundary surfaces with their
openings, implicit geometries and their templates, materials and textures,
addresses, group members (referred to by `xlink:href` across features), and
ADE properties and classes under their namespaces, in CityGML 3.0 in the
`adeOf…` hook and container the package's extension declaration names. On
the PLATEAU tiles with their i-UR ADEs, the CityGML 3.0 samples and the OGC
Test ADE, a LoD 4 building, building parts, appearances and 3DBAG's Delft
tile, the document is valid in `citygml-tools validate` and reads back equal
to the source (`crates/core/tests/citygml_export.rs`).

What a version cannot hold is left out and counted in the export report
(`ExportReport::citygml`), which the CLI prints as warnings: a geometry at a
whole LoD its object already has one of that kind at (3DBAG's LoD 1.2 and 1.3
are both LoD 1), a boolean generic attribute (written as text), a semantic
surface's LoD 1 faces in CityGML 2.0 (kept untagged), a LoD 4 geometry in
CityGML 3.0 (written at LoD 3 when its object has none there), a class the
version lacks, and an ADE property for another CityGML version. In CityGML
3.0 a member a group holds is written after it as an object of its own,
since `grp:groupMember` only refers.

## Deferred / not yet supported

- A group's `grp:parent` and `grp:geometry` are not read. A member read from a
  stream (`FeatureReader::from_reader`, one pass) does not name the groups that
  reference it.
- An `ImplicitGeometry` is dropped when its relative geometry is a
  `libraryObject` only, lives in another document, is an `xlink:href` to a
  geometry no `ImplicitGeometry` writes inline (one in an object's
  `lodNGeometry`), or is a kind the reader does not read as a relative
  geometry (`gml:MultiSolid`, a bare `gml:OrientableSurface` or
  `gml:TriangulatedSurface`). Every `ImplicitGeometry` read from a stream
  (`FeatureReader::from_reader`) is dropped too, since its header has no
  templates. At a LoD that also has surface or solid geometry the instance is
  dropped, a CityParquet object holding one geometry per LoD. A material or
  texture an object's own `app:appearance` puts on a relative geometry is not
  carried onto the template.
- A document with implicit geometries is read in full by the up-front pass,
  buildings included, so it costs one more pass over the file than one
  without.
- Raster, breakline and mass-point relief components have no CityJSON type
  and are tallied as skipped members.
- At a LoD with a solid, boundary-surface polygons that are not faces of the
  solid (an opening in a room's wall) are dropped: a CityParquet object holds
  one geometry per LoD. The readers keep a point or curve geometry
  (`lodNMultiCurve`, `lodNNetwork`) beside the surfaces of its LoD, but a
  package stores only the first geometry of a LoD and counts the rest
  (`skipped_same_lod_geometries`).
- An ADE's schema is read only when `xsi:schemaLocation` names it by a path
  relative to the document that resolves, as in the PLATEAU archive; one named
  by URL is not fetched. Without its schema an ADE class goes to its ADE's own
  file (`urf.parquet`) even when it specialises a CityGML class, and an ADE
  value is a string unless it is a measure (has `uom`). With its schema, a
  value that is not one of its declared type (`N/A` in an `xs:integer`) stays a
  string, which makes its whole column a JSON column. GML geometry inside an ADE
  property, text between an ADE element's child elements, an ADE property of
  a boundary surface or of the CityModel, and ADE appearance are left out; the reader tallies the geometry by property
  name (`ade_geometries`).
- CityGML 3.0 only: a transportation space's own geometry and attributes, a
  height whose status is not `measured`, a
  constructive element's `con:filling` (a door or window as an object), the
  dynamizer, point cloud and versioning modules are not read. A city object in
  a `core:featureMember` is read as one in a `cityObjectMember` is. A `gen:GenericThematicSurface` is an untyped face, CityJSON
  having no type for it. The `citygml-tools upgrade` of a PLATEAU file holds
  no i-UR ADE content, since citygml-tools drops what it has no module for.
- A boundary surface's own properties (`gml:name`, generic attributes,
  appearance, ADE properties) are not read; only its geometry and its
  nested doors and windows are.
- A `MultiSolid` is written only by a CityGML 2.0 class with a
  `lodNMultiSolid` or `lodNGeometry` property; elsewhere, and in CityGML 3.0,
  it is left out and counted.
- `TexCoordGen` / texture UV seams; cross-building
  shared/external geometry; coordinate-magnitude CRS sniffing.

## Provenance

Gap-filling campaign (CG-1…CG-8), 2026-07-16/17. Merged & `gpt-5.6-sol`-reviewed:
CG-1 `6e75c34`, CG-2 `01f5d79`, CG-3 `66e2cee`, CG-7 `f23025f`, CG-5 `46169cf`.
Deferred by the author: CG-4, CG-6, CG-8.
