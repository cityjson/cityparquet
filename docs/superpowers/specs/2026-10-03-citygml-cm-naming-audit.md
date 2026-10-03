# CityGML CM naming audit — CityJSON-derived names in the specification

**Status:** open, not decided. Recorded 2026-10-03 while revising the paper (§3 Design), where the prose now says "implicit geometry" but the format still says `geometry_templates`.

**Principle:** CityParquet encodes the CityGML 3.0 conceptual model, not CityJSON. Names should follow the CityGML CM where the CM has a term; CityJSON names belong only in the encoder/decoder mapping (`07-mapping-cityjson.mdx`). Several names in the specification are CityJSON's instead. This list records them so a rename can be decided in one pass. The paper must match whatever the specification says, so the paper changes only after the specification does.

Line references are to `documents/docs/03-specification/` at commit `e0ba044`.

## 1. Rename candidates

| # | Current name (where) | CityJSON origin | CityGML 3.0 CM term | Proposal |
| --- | --- | --- | --- | --- |
| 1 | `geometry_templates.parquet` sidecar (`01-dataset-package`, `04-appearance-templates` §geometry_templates.parquet, `05-metadata`) | `"geometry-templates"` object | `ImplicitGeometry` (Core), whose shared shape is its `relativeGeometry` | `implicit_geometries.parquet` (**decide**) |
| 2 | `template` column, `STRUCT<id, point, transformationMatrix>` (`02-object-table-schema` reserved columns; `04-appearance-templates`) | `GeometryInstance` with `"template"`, `"boundaries"` (reference point), `"transformationMatrix"` | `ImplicitGeometry` with `referencePoint` and `transformationMatrix` | column `implicit_geometry`; field `point` → `reference_point`; field naming style (camelCase `transformationMatrix` vs snake_case) to be made consistent (**decide**) |
| 3 | Page title "Appearance & templates" and prose "geometry template(s)" throughout | — | implicit geometry | follows #1–2 |

**Scope of #1–2 (files mentioning `geometry_templates`):** `lib/cityparquet-rs` 23, `lib/duckdb-cityjson` 25, `lib/citylake` 6, `documents` 5, `ai` 4, `benchmark` 2, `test` 1. This is a breaking format change. It also needs the paper's package figure (`fig:package`), reserved-columns table and §3.4 updated (paper `#todo` in `03-design.typ` §3.4).

## 2. Consider: CityJSON structure where CityGML has specific associations

| # | Current name | CityJSON origin | CityGML 3.0 CM | Note |
| --- | --- | --- | --- | --- |
| 4 | `parents`, `children` (reserved columns) | `"parents"`, `"children"` on a CityObject | Hierarchy through specific associations (e.g. a `Building`'s building parts; `CityObjectGroup` group members) | Generic columns are a deliberate simplification, so keep them, but say so in the spec rather than leave them as unexplained CityJSON names |
| 5 | `children_roles` | `"children_roles"` on `CityObjectGroup` | Group member role in `CityObjectGroup` (verify the exact CM property name) | Same as #4 |
| 6 | `geometry_properties.surfaces` and the semantic surface's `parent` / `children` indices (`03-geometry-semantics`) | Semantic Object (`"surfaces"`, `"values"`, `"parent"`, `"children"`) | Thematic surfaces (`AbstractThematicSurface`) bounding a space; openings (`Window`, `Door`) attached to a surface | `surfaces` is generic enough; document the mapping. Verify the CM terms before renaming anything |
| 7 | `feature_id` (root of the parent chain) | `CityJSONFeature` (one root object and its children) | Top-level feature | Keep; define it in CM terms in the spec |

## 3. Keep, but document as CityJSON provenance

| # | Current name | Why it is CityJSON-specific | Note |
| --- | --- | --- | --- |
| 8 | LoD minor suffix `lod<major>_<minor>` (`geometry_lod2_2`) | CityJSON allows refined LoD strings such as `"2.2"` | CityGML 3.0 has LoD 0–3 with integer levels (no LoD4, no refined LoDs). The suffix is needed for CityJSON sources; say so where the grammar is defined |
| 9 | `city.appearance_defaults` (`05-metadata`) | `"default-theme-texture"` / `"default-theme-material"` | CityGML has no default theme; already optional, label it as source provenance |
| 10 | `ex_` prefix replacing `+` (`06-extensions`) | CityJSON Extension naming | Superseded by S6 (namespace prefix, closer to CityGML ADE XML namespaces) |
| 11 | "restored into the object's `attributes`" (`02-object-table-schema`, the `other` column) | CityJSON `"attributes"` member | Spec wording only: describe it as attributes/properties on export rather than a CityJSON member |
| 12 | `textures.parquet` `image_type` ("not a MIME type") | CityJSON texture `"type"` | CityGML `ParameterizedTexture` uses a MIME type; current choice is fine but should be noted |
| 13 | `city.other` carrying the source `transform` | CityJSON `"transform"` | Informational only; no change |

## 4. Outside this repository

- `city3d:co_types` in the STAC extension (`cityjson/stac-city3d`) is a CityJSON-style abbreviation ("CityObject types") and keeps the source `+` prefix. Any change belongs in that repository.

## Already aligned (no action)

- File per CityGML 3.0 module; `object_type` uses CityGML 3.0 class names, with the four CityJSON names that differ mapped on import (`TransportSquare` → `Square`, `GenericCityObject` → `GenericOccupiedSpace`, `BuildingStorey` → `Storey`, `TunnelHollowSpace` → `HollowSpace`).
- `materials.parquet` fields (`ambientIntensity`, `diffuseColor`, …) and `textures.parquet` `wrapMode`, `textureType`, `borderColor` follow the CityGML/X3D appearance vocabulary that CityJSON also uses.

## Related open issue

The CityGML 3.0 CM, Requirement 2 (`/req/core/isorestrictions`), says solids "SHALL only include exterior boundaries". `geometry_properties.shells` admits interior shells. Decide whether interior shells are a CityGML 2.0 / CityJSON source feature that CityParquet carries beyond the 3.0 CM, and say so in `03-geometry-semantics` (the paper has the same open question in §2 and §3.3).

## Decision (2026-10-03)

The author decided #1–2: the sidecar becomes `implicit_geometries.parquet` and the column `implicit_geometry`. The paper (`03-design.typ`: package figure, reserved-columns table, §3.4) already uses the new names, ahead of the format. The specification and the writers must follow before submission. Still open: field names inside the struct (`point` → `reference_point`; `transformationMatrix` style).

## Specification fixes raised by the paper revision

Changes the paper (`paper/chapters/03-design.typ`) relies on or has exposed. Each one must land in the specification before submission.

| # | Page | Fix | Status |
| --- | --- | --- | --- |
| S1 | `03-specification/03-geometry-semantics.mdx`, § WKB encoding | Said `PolyhedralSurfaceZ` "lies outside Simple Features". SFA 1.2.1 defines `PolyhedralSurface`; it is GeoParquet that does not permit it (GeoParquet admits only the seven basic types, `Point` to `GeometryCollection`) | Edited in the working tree on 2026-10-03, **not committed**; review, then commit |
| S2 | `01`, `02`, `04`, `05`, `07` specification pages, design decision 04 | Rename `geometry_templates.parquet` → `implicit_geometries.parquet` and `template` → `implicit_geometry` (see Decision above), plus every writer | Decided, not started |
| S3 | `03-specification/03-geometry-semantics.mdx`, `shells` | Interior shells vs CityGML 3.0 Requirement 2 (see Related open issue). The paper (2026-10-03) now keeps interior shells as a stated departure: CityJSON 2.0.1 permits them (§3.2: "the first shell being the exterior shell of the solid, and the others the interior shells"), and the §3.3 example shows a solid with a cavity, `shells: [[6, 6]]`. The specification should say the same in `03-geometry-semantics` | Paper direction set; specification wording to add |
| S4 | `03-specification/06-extensions.mdx`, `05-metadata.mdx` (`city.extensions`) | Paper §3.5 says extension declarations are referenced by a schema URL (JSON Schema or XSD); the specification stores the declaration verbatim as JSON plus a source-name ↔ class ↔ module mapping | Author decision taken; specification not updated |
| S5 | `03-specification/06-extensions.mdx`, § Extended feature types | The example "an `Energy` ADE defining `SolarPanel` and `Thermostat` both in an `Energy` module" is invented: the real Energy ADE (Agugiaro et al. 2018) has no `SolarPanel`/`Thermostat` classes and is split into Core, Building Physics, Occupant Behaviour, Material and Construction, and Energy Systems modules. The paper now uses `ThermalZone` and `ThermalBoundary` (Building Physics) sharing `building_physics.parquet`; use a real example in the spec too | Paper updated; specification not |
| S6 | `03-specification/06-extensions.mdx`, `01-dataset-package.mdx`, `02-object-table-schema.mdx`, `03-geometry-semantics.mdx`, `05-metadata.mdx` | **Namespace prefix instead of `ex_` (author decision 2026-10-03).** Every name an extension adds carries a short extension namespace as a prefix: attributes (`energy_heatCapacity`), semantic surfaces in `geometry_properties` (`energy_PartyWallSurface`), and module files (`energy_building_physics.parquet`). Replaces `ex_` and the unchanged surface names; also removes the core/extension module-name collision (Energy ADE 3.0 revision declares a module named `Building`). To decide: where the namespace is declared (`city.extensions`), how it is derived from a CityJSON Extension / ADE, and whether `object_type` values are prefixed | Paper updated (§3.5, round-trip table); specification and writers not |

## Next step

Decide #1–2 first, since they are the only clear mismatches and the paper is waiting on them. Then update the specification, `cityparquet-rs`, `duckdb-cityjson` and `citylake` together, and last of all the paper.
