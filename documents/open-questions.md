# Open questions

These are the **undecided** design questions of the
[CityParquet specification](https://cityjson.github.io/cityparquet/specification).
Two of them still affect normative column shapes. The specification marks each point
that depends on one of them with 🟡 and links here.

The wording below deliberately preserves the alternatives rather than leaning towards
a resolution.

## Questions that affect normative column shapes

### Duplicate geometry at the same (object, LoD)

A source model does not guarantee one geometry per object per LoD, but the
[column-per-LoD layout](https://cityjson.github.io/cityparquet/specification/geometry-semantics) gives each `(object, LoD)`
pair a single slot. The open question is whether to **prohibit duplicates as invalid
input**, or to **define a collection representation** — for example promoting the slot
to a list, or merging the duplicates into a `MultiSolid`. Until this is decided the
reference implementation silently drops the extra geometries, so no choice can be
enforced yet.

### Multiple implicit geometries per object

The `implicit_geometry` column [defined in the object table schema](https://cityjson.github.io/cityparquet/specification/object-table-schema)
is a single struct, so an object with several implicit geometries cannot currently be
represented. The choice is between **one implicit geometry per object** (rejecting the
rest) and a **list-valued `implicit_geometry` column** that can hold several. A third
option surfaced during the 2026-07-21 review and was set aside as scope creep:
**per-LoD `implicit_geometry_lod*` columns**, which would resolve the case where the
implicit geometries differ by LoD but not the case where they do not. The reference
implementation keeps only the first implicit geometry today.

## Other open questions

### Extensions: attribution across several extensions, and ADE object-class metadata

An extension's identity is carried by its [namespace prefix](https://cityjson.github.io/cityparquet/specification/extensions):
every name an extension adds — attribute columns, `object_type` values, semantic surface
types and module files — carries it, and `city.extensions` maps it to the extension's
schema document. Two parts remain open:

- **Attributing names across several extensions.** When a CityJSON source declares
  several extensions, attributing each `+` name to one of them needs the extensions'
  schema documents (`extraAttributes`, `extraCityObjects`, `extraSemanticSurfaces`).
  How a writer obtains and reads them is not yet settled.
- **Full ADE object-class metadata.** Beyond the class name, its module file and the
  extension declaration, a richer scheme may be needed to carry an ADE's object-class
  information faithfully.

### Strict 1:1 face_semantics ↔ source surfaces

Whether to guarantee a **strict 1:1** correspondence between `face_semantics` and the
source's surfaces by requiring that degenerate faces are never dropped. Preserving
every source surface keeps the mapping exact but forces degenerate faces into the WKB;
dropping them yields cleaner geometry but breaks the index alignment. The format
currently requires only that `face_semantics` match the faces actually emitted.

### Pruning a solid column

GeoParquet 2.0 removed `covering`, and Parquet-native geospatial statistics replaced
it: a `GEOMETRY`-annotated column carries a per-chunk bounding box, `zmin`/`zmax`
included, so `bbox_lod*` covering columns are no longer worth defining. That settles
the question for footprint columns.

It leaves it open for solid columns, which cannot be annotated — their
`PolyhedralSurface Z` bytes are outside every consuming engine's geometry model — and
so get no per-chunk spatial statistics of their own. They fall back to the row-level
[`bbox`](https://cityjson.github.io/cityparquet/specification/geometry-semantics) column, which is subtree-unioned rather
than per-LoD and therefore coarser than a true covering. Whether that is worth
narrowing, and at what cost in row width, is unsettled. See the
[metadata page](https://cityjson.github.io/cityparquet/specification/metadata).
