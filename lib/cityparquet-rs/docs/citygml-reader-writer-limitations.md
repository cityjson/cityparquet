# CityGML 2.0 reader/writer — supported scope & known limitations

Status as of 2026-10-14. The CityGML 2.0 reader (the `citygml2` crate in
`lib/citygml-rs`, adapted in `crates/core/src/citygml/`) and the writer
round-trip 3D city models through the CityParquet package. This document
records what is supported and what remains, for the paper's scope section and
future work. The **package round-trip is lossless** for everything listed as
supported — where the emitted CityGML is not strictly XSD-valid, the lenient
reader still reads it back identically.

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
  LoD the document spells, LoD 4 included. A shell member that is an
  `xlink:href` to an identified `gml:CompositeSurface` or `gml:MultiSurface`
  stands for all of that aggregate's polygons, and a nested object (a room)
  resolves xlinks to polygons its parent defines.
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

**Writer (CityParquet → CityGML), scope W-M1…W-M5:**

- `Building`/`BuildingPart` with LoD solids/CompositeSolid (`lod1Solid` to
  `lod4Solid`), semantic surfaces, attributes, materials, textures; a child
  surface (a Door or Window) nested in its parent wall's `bldg:opening`.

## Deferred / not yet supported

### Writer round-trip for the reader-only features above

The reader ingests these but the CityGML **writer** does not yet emit them (they
still round-trip CityGML→parquet→**CityJSON**, just not →CityGML):

- Non-building 1st-level objects (CG-7 writer side; `non_building_skipped`).
- BuildingInstallation emission (CG-5 writer side).
- CityModel-level appearance / non-building appearance emission.
- `ImplicitGeometry` emission.

### Other documented limitations

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
  one geometry per LoD. For the same reason a point or curve geometry
  (`lodNMultiCurve`, `lodNNetwork`) at a LoD that also has surfaces is
  dropped.
- ADE elements: a text-only ADE property becomes a string attribute; ADE
  classes and structured ADE properties are skipped.
- `MultiSolid` cannot be written (CityGML 2.0 `Building` has no `lodNMultiSolid`
  slot — a genuine format asymmetry, `multi_solids_skipped`).
- XSD polish: canonical element ordering, `xs:date`/`gYear` lexical validity.
- `TexCoordGen` / texture UV seams; cross-building
  shared/external geometry; coordinate-magnitude CRS sniffing.

## Provenance

Gap-filling campaign (CG-1…CG-8), 2026-07-16/17. Merged & `gpt-5.6-sol`-reviewed:
CG-1 `6e75c34`, CG-2 `01f5d79`, CG-3 `66e2cee`, CG-7 `f23025f`, CG-5 `46169cf`.
Deferred by the author: CG-4, CG-6, CG-8.
