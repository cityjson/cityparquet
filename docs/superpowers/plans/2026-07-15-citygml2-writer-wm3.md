# CityGML 2.0 Writer — W-M3 (Semantic Surfaces) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Emit `bldg:boundedBy` semantic surfaces so `geometry_properties.semantics` round-trips back to CityGML 2.0 (Solid/CompositeSolid via xlink, MultiSurface inline).

**Architecture:** A new `writer/semantics.rs` parses the semantics object, resolves each WKB face to its surface index, allocates document-unique `gml:id`s, and emits the semantics-aware forms (xlinked solid + inline `boundedBy`, or inline-`boundedBy`-only). `write_building` dispatches to it when a geometry carries `semantics`; otherwise the W-M2 inline path is unchanged.

**Tech Stack:** Rust, `quick_xml::Writer`, `serde_json`, existing `export::{partition_shells, shell_faces_flat, shell_faces_nested}` + `geometry::{write_polygon, write_composite_surface}`.

## Global Constraints

- Round-trip invariant is PACKAGE-LEVEL (geometry + `semantics`), keyed by `(building_id, major_lod)`; oracles use CityGML-sourced fixtures only (b1, railway).
- Never error on unrepresentable data — geometry-only fallback + counter.
- Emit exactly one `bldg:boundedBy` per `surfaces` entry, in array order — never dedupe/reorder/merge; a zero-face surface is an empty element.
- `gml:id`s: document-global monotonic counter, prefix `_cpq_p`, inserted into the existing `seen_ids` set (bump on collision) so they are document-unique and never clash with a CityObject id.
- Solid faces with a `null` value are emitted inline in the solid's `surfaceMember` (no `boundedBy`, no id).
- `just check` is the gate; Codex review at milestone end.
- Types: `Face = Vec<Vec<usize>>` (rings of coord indices); a shell is `Vec<Face>`; `DecodedKind::{PolyhedralSurface(Vec<Face>), GeometryCollection(Vec<DecodedKind>), MultiPolygon(Vec<Face>)}`.

---

### Task 1: Semantics model + face→surface resolution (`writer/semantics.rs`)

Pure parsing/resolution, unit-tested with in-code values. No emission yet.

**Files:**
- Create: `crates/cityparquet/src/citygml/writer/semantics.rs`
- Modify: `crates/cityparquet/src/citygml/writer/mod.rs` (add `pub mod semantics;`)

**Interfaces:**
- Produces:
  - `pub struct Semantics { pub surfaces: Vec<String>, pub values: serde_json::Value }`
  - `pub fn parse_semantics(props: Option<&Value>) -> Option<Semantics>` — `None` when absent/malformed.
  - `pub fn surfaces_emittable(s: &Semantics) -> bool` — every surface `type` is a valid XML NCName (`building::is_ncname`); a CityJSON extension type (`+Foo`) makes this `false`.
  - `pub fn solid_face_surfaces(values: &Value, shells: &[Vec<Face>], nsurfaces: usize) -> Result<Vec<Option<usize>>>` — flat per-face surface index in shell-concatenation order; validates nesting lengths and index range.
  - `pub fn composite_face_surfaces(values: &Value, members: &[Vec<Vec<Face>>], nsurfaces: usize) -> Result<Vec<Vec<Option<usize>>>>` — one flat vec per member.
  - `pub fn multisurface_face_surfaces(values: &Value, nfaces: usize, nsurfaces: usize) -> Result<Vec<Option<usize>>>` — flat `[position]`.
  - Type alias `pub type Face = Vec<Vec<usize>>;`

- [ ] **Step 1: Write the failing tests**

Create `semantics.rs` with a test module (impl absent → red). Cover: parse extracts surfaces+values; `surfaces_emittable` rejects `+Foo`; `solid_face_surfaces` maps `[[0,null],[1]]` over shells `[[f,f],[f]]` → `[Some(0),None,Some(1)]`; nesting-length mismatch → Err; index ≥ nsurfaces → Err; `composite_face_surfaces` over 2 members; `multisurface_face_surfaces` flat.

```rust
//! Parse `geometry_properties.semantics` and resolve each WKB face to its
//! surface index, in the face-walk order the solid/multisurface emitter uses.
use serde_json::Value;
use cityparquet_schema::CityParquetError;
use crate::Result;
use super::building::is_ncname;

pub type Face = Vec<Vec<usize>>;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn f() -> Face { vec![vec![0, 1, 2]] }

    #[test]
    fn parse_extracts_surfaces_and_values() {
        let props = json!({"type":"Solid","semantics":{
            "surfaces":[{"type":"WallSurface"},{"type":"RoofSurface"}],
            "values":[[0,1]]}});
        let s = parse_semantics(Some(&props)).unwrap();
        assert_eq!(s.surfaces, vec!["WallSurface", "RoofSurface"]);
        assert!(surfaces_emittable(&s));
    }

    #[test]
    fn extension_type_is_not_emittable() {
        let s = Semantics { surfaces: vec!["+Custom".into()], values: json!([[0]]) };
        assert!(!surfaces_emittable(&s));
    }

    #[test]
    fn solid_values_map_faces_to_surfaces_with_null() {
        let shells = vec![vec![f(), f()], vec![f()]];
        let out = solid_face_surfaces(&json!([[0, null], [1]]), &shells, 2).unwrap();
        assert_eq!(out, vec![Some(0), None, Some(1)]);
    }

    #[test]
    fn solid_nesting_mismatch_errors() {
        let shells = vec![vec![f(), f()]];
        assert!(solid_face_surfaces(&json!([[0]]), &shells, 2).is_err()); // 1 value, 2 faces
    }

    #[test]
    fn solid_index_out_of_range_errors() {
        let shells = vec![vec![f()]];
        assert!(solid_face_surfaces(&json!([[5]]), &shells, 2).is_err());
    }

    #[test]
    fn composite_values_per_member() {
        let m = vec![vec![f()]];
        let members = vec![m.clone(), m];
        let out = composite_face_surfaces(&json!([[[0]], [[null]]]), &members, 1).unwrap();
        assert_eq!(out, vec![vec![Some(0)], vec![None]]);
    }

    #[test]
    fn multisurface_flat_values() {
        let out = multisurface_face_surfaces(&json!([0, 1, 0]), 3, 2).unwrap();
        assert_eq!(out, vec![Some(0), Some(1), Some(0)]);
    }
}
```

- [ ] **Step 2: Run to verify failure** — `cargo test -p cityparquet --lib citygml::writer::semantics` → FAIL (undefined).

- [ ] **Step 3: Implement**

```rust
pub struct Semantics {
    pub surfaces: Vec<String>,
    pub values: Value,
}

fn err(m: impl Into<String>) -> CityParquetError { CityParquetError::Geometry(m.into()) }

pub fn parse_semantics(props: Option<&Value>) -> Option<Semantics> {
    let sem = props?.get("semantics")?;
    let surfaces = sem.get("surfaces")?.as_array()?;
    let surfaces: Vec<String> = surfaces
        .iter()
        .map(|s| s.get("type").and_then(Value::as_str).unwrap_or("").to_string())
        .collect();
    let values = sem.get("values")?.clone();
    Some(Semantics { surfaces, values })
}

pub fn surfaces_emittable(s: &Semantics) -> bool {
    !s.surfaces.is_empty() && s.surfaces.iter().all(|t| is_ncname(t))
}

/// One face's value: null -> None, integer in range -> Some(idx), else Err.
fn face_index(v: &Value, nsurfaces: usize) -> Result<Option<usize>> {
    match v {
        Value::Null => Ok(None),
        Value::Number(n) => {
            let i = n.as_u64().ok_or_else(|| err("semantics value is not a non-negative integer"))? as usize;
            if i >= nsurfaces { return Err(err(format!("semantics value {i} >= surfaces len {nsurfaces}"))); }
            Ok(Some(i))
        }
        _ => Err(err("semantics value is neither null nor an integer")),
    }
}

pub fn solid_face_surfaces(values: &Value, shells: &[Vec<Face>], nsurfaces: usize) -> Result<Vec<Option<usize>>> {
    let vshells = values.as_array().ok_or_else(|| err("solid semantics values must be an array of shells"))?;
    if vshells.len() != shells.len() {
        return Err(err(format!("semantics has {} shells but geometry has {}", vshells.len(), shells.len())));
    }
    let mut out = Vec::new();
    for (vs, shell) in vshells.iter().zip(shells) {
        let vfaces = vs.as_array().ok_or_else(|| err("shell values must be an array"))?;
        if vfaces.len() != shell.len() {
            return Err(err(format!("semantics shell has {} faces but geometry has {}", vfaces.len(), shell.len())));
        }
        for v in vfaces {
            out.push(face_index(v, nsurfaces)?);
        }
    }
    Ok(out)
}

pub fn composite_face_surfaces(values: &Value, members: &[Vec<Vec<Face>>], nsurfaces: usize) -> Result<Vec<Vec<Option<usize>>>> {
    let vmembers = values.as_array().ok_or_else(|| err("composite semantics values must be an array of solids"))?;
    if vmembers.len() != members.len() {
        return Err(err(format!("semantics has {} solids but geometry has {}", vmembers.len(), members.len())));
    }
    let mut out = Vec::with_capacity(members.len());
    for (vm, shells) in vmembers.iter().zip(members) {
        out.push(solid_face_surfaces(vm, shells, nsurfaces)?);
    }
    Ok(out)
}

pub fn multisurface_face_surfaces(values: &Value, nfaces: usize, nsurfaces: usize) -> Result<Vec<Option<usize>>> {
    let vs = values.as_array().ok_or_else(|| err("multisurface semantics values must be an array"))?;
    if vs.len() != nfaces {
        return Err(err(format!("semantics has {} values but geometry has {} faces", vs.len(), nfaces)));
    }
    vs.iter().map(|v| face_index(v, nsurfaces)).collect()
}
```

Add `pub mod semantics;` to `mod.rs`. Note `is_ncname` must be `pub` in `building.rs` (it already is).

- [ ] **Step 4: Run to verify pass** — same command → PASS.
- [ ] **Step 5: `just check` scoped** — `cargo clippy -p cityparquet --all-targets -- -D warnings` + `cargo fmt --all --check` → clean.
- [ ] **Step 6: Commit** — `feat(citygml): W-M3 semantics model + face->surface resolution`.

---

### Task 2: `gml:id` allocator + per-face surfaceMember writer (`writer/semantics.rs`, `writer/geometry.rs`)

**Files:**
- Modify: `crates/cityparquet/src/citygml/writer/semantics.rs` (allocator)
- Modify: `crates/cityparquet/src/citygml/writer/geometry.rs` (xlink/inline member writer)

**Interfaces:**
- Produces:
  - `pub struct IdAlloc<'a> { next: usize, seen: &'a mut std::collections::HashSet<String> }` with `pub fn new(seen) -> Self` and `pub fn alloc(&mut self) -> String` (returns a `_cpq_p<N>` not in `seen`, inserting it).
  - `pub fn write_xlink_member<W>(w, id: &str) -> Result<()>` — `<gml:surfaceMember xlink:href="#id"/>`.
  - `pub fn write_inline_member<W>(w, coords, face) -> Result<()>` — `<gml:surfaceMember><gml:Polygon>…` (reuses `write_polygon`).

- [ ] **Step 1: Failing tests** — allocator produces distinct `_cpq_p0/_cpq_p1`, skips an id already in `seen`; `write_xlink_member` emits `<gml:surfaceMember xlink:href="#_cpq_p0"/>`; `write_inline_member` wraps a `gml:Polygon`.

```rust
#[test]
fn id_alloc_is_unique_and_avoids_seen() {
    let mut seen = std::collections::HashSet::from(["_cpq_p0".to_string()]);
    let mut a = IdAlloc::new(&mut seen);
    assert_eq!(a.alloc(), "_cpq_p1"); // p0 taken
    assert_eq!(a.alloc(), "_cpq_p2");
}
```

- [ ] **Step 2: Run → FAIL.**
- [ ] **Step 3: Implement**

```rust
use std::collections::HashSet;

pub struct IdAlloc<'a> { next: usize, seen: &'a mut HashSet<String> }
impl<'a> IdAlloc<'a> {
    pub fn new(seen: &'a mut HashSet<String>) -> Self { Self { next: 0, seen } }
    pub fn alloc(&mut self) -> String {
        loop {
            let id = format!("_cpq_p{}", self.next);
            self.next += 1;
            if self.seen.insert(id.clone()) { return id; }
        }
    }
}
```

In `geometry.rs` (reusing `write_polygon`):

```rust
pub fn write_xlink_member<W: Write>(w: &mut Writer<W>, id: &str) -> Result<()> {
    let mut m = BytesStart::new("gml:surfaceMember");
    m.push_attribute(("xlink:href", format!("#{id}").as_str()));
    w.write_event(Event::Empty(m)).map_err(io_err)?;
    Ok(())
}

pub fn write_inline_member<W: Write>(w: &mut Writer<W>, coords: &[[f64; 3]], face: &[Vec<usize>]) -> Result<()> {
    w.write_event(Event::Start(BytesStart::new("gml:surfaceMember"))).map_err(io_err)?;
    write_polygon(w, coords, face)?;
    w.write_event(Event::End(BytesEnd::new("gml:surfaceMember"))).map_err(io_err)?;
    Ok(())
}
```

`write_polygon` needs an optional `gml:id` on its `gml:Polygon` for the boundedBy inline polygons (Task 3). Add a variant `write_polygon_with_id<W>(w, coords, face, id: Option<&str>)` and make `write_polygon` call it with `None`; put the `gml:id` attribute on the `gml:Polygon` start when `Some`.

- [ ] **Step 4: Run → PASS. Step 5: clippy+fmt. Step 6: Commit** — `feat(citygml): W-M3 gml:id allocator + xlink/inline surfaceMember writers`.

---

### Task 3: Solid/CompositeSolid + semantics emission (`writer/semantics.rs`)

Emit the xlinked solid + `boundedBy` from a decoded geometry + its `Semantics`. This is the core of case §B.

**Interfaces:**
- Consumes: Task 1 (resolution), Task 2 (allocator, members), `export::{shell_faces_flat, shell_faces_nested, partition_shells}`, `geometry::{write_gml_solid-like, write_polygon_with_id, write_composite_surface}`.
- Produces: `pub fn write_solid_with_semantics<W>(w, coords, kind: &DecodedKind, props, sem: &Semantics, ids: &mut IdAlloc, major: u8) -> Result<()>` — emits `<bldg:lod{major}Solid>` (xlink/inline members) followed by the `boundedBy` block. Errors bubble to the caller for geometry-only fallback.

**Algorithm (spelled out — the tricky part):**
1. Build the per-member shell partition: for a `PolyhedralSurface(faces)` → one member `[partition_shells(faces, shell_faces_flat)]`; for `GeometryCollection(members)` → `partition_shells` per member with `shell_faces_nested[m]`.
2. Resolve surfaces: `solid_face_surfaces` per member (or `composite_face_surfaces`), giving `Vec<Vec<Option<usize>>>` (member → flat face → surface).
3. Allocate: for every non-null face, `ids.alloc()`; build `face_ids: Vec<Vec<Option<String>>>` aligned to (2).
4. Emit `<bldg:lod{major}Solid>` → `gml:Solid` (single member) or `gml:CompositeSolid`/`solidMember`/`gml:Solid` (per member). Each `gml:Solid` = exterior shell 0 + interior shells, each shell a `gml:CompositeSurface` whose `surfaceMember`s are, per face in shell order, `write_xlink_member(id)` if the face has an id else `write_inline_member(coords, face)`.
5. Emit the `boundedBy` block: for each surface index `i` in `0..sem.surfaces.len()`, `<bldg:{surfaces[i]}><bldg:lod{major}MultiSurface><gml:surfaceMember><gml:Polygon gml:id="{id}">…</gml:surfaceMember>…</bldg:lod{major}MultiSurface></bldg:{surfaces[i]}>` — one member per (member,shell,face) whose surface==i, using its allocated id; if none, emit `<bldg:{surfaces[i]}/>` (empty).
6. Accumulate bounds over every emitted coordinate (done by the caller over the geometry's coord pool, unchanged).

**Failing tests** (in-code single Solid + CompositeSolid with semantics):
- 1 shell, 3 faces, values `[[0,null,1]]`, surfaces `[Wall, Roof]`: solid has 3 `surfaceMember`s (2 xlink, 1 inline); `<bldg:WallSurface>` xlinks face 0's id; `<bldg:RoofSurface>` xlinks face 2's id; the xlinked ids in the solid MATCH the `gml:Polygon gml:id`s in boundedBy.
- A zero-face surface index → `<bldg:{type}/>` empty and index alignment preserved.
- 2-member CompositeSolid with `[solid][shell][face]` values → 2 `solidMember`s, boundedBy spans both.

Assert by string containment + id-cross-reference (extract `xlink:href="#X"` and assert a `gml:Polygon gml:id="X"` exists).

- [ ] Steps: failing tests → FAIL → implement per algorithm → PASS → clippy+fmt → commit `feat(citygml): W-M3 solid+semantics emission (xlink solid + boundedBy)`.

---

### Task 4: MultiSurface + semantics emission (`writer/semantics.rs`)

Case §C: inline `boundedBy`, no solid.

**Interfaces:**
- Produces: `pub fn write_multisurface_with_semantics<W>(w, coords, faces: &[Face], props, sem: &Semantics, major: u8, report: &mut WriteReport) -> Result<()>` — emits one `bldg:boundedBy` per surface (array order, empty when zero-face) with inline `gml:Polygon`s; a null-value face increments `report.multisurface_null_faces_dropped` and is not emitted.

**Failing tests** (in-code MultiPolygon + flat values):
- faces `[a,b,c]`, values `[0,0,1]`, surfaces `[Wall, Roof]` → `<bldg:WallSurface>` has 2 polygons, `<bldg:RoofSurface>` has 1; no `gml:Solid`.
- a `null` value → that face dropped, `multisurface_null_faces_dropped == 1`.
- zero-face surface → empty element.

- [ ] Steps: failing tests → FAIL → implement → PASS → clippy+fmt → commit `feat(citygml): W-M3 multisurface+semantics emission`.

---

### Task 5: Wire into `write_building` + driver + counters

**Files:**
- Modify: `writer/mod.rs` (`WriteReport` counters; `route_geometry` — carry MultiSurface/CompositeSurface **with semantics** to the building instead of `lod_columns_skipped`).
- Modify: `writer/building.rs` (dispatch: a geometry with emittable semantics → semantics path; else W-M2 path; semantics on a MultiSolid → `semantic_surfaces_dropped`; geometry-only fallback on resolution error).

**`WriteReport` additions:** `semantic_surfaces_written`, `multisurface_null_faces_dropped` (keep `semantic_surfaces_dropped`).

**`write_building` emit dispatch** (replacing the current per-major match): for each kept `(major, geom, props)`:
- `let sem = parse_semantics(props).filter(surfaces_emittable);`
- If `sem` is `Some` and `geom.kind` is `PolyhedralSurface`/`GeometryCollection`: `write_solid_with_semantics(...)`; on `Err`, fall back to the W-M2 `write_solid`/`write_composite_solid` and `semantic_surfaces_dropped += sem.surfaces.len()`; on `Ok`, `semantic_surfaces_written += sem.surfaces.len()`.
- If `sem` is `Some` and `geom.kind` is `MultiPolygon`: `write_multisurface_with_semantics(...)` (wrap in its own `<bldg:lod{major}…>`? No — boundedBy is top-level on the Building, so emit the boundedBy block directly, no `lodNSolid`). `semantic_surfaces_written += ...`.
- Else (no semantics): W-M2 path unchanged (`PolyhedralSurface`→`write_solid` in `lodNSolid`; `GeometryCollection`→`write_composite_solid`; `MultiPolygon` without semantics → was never carried here, still skipped in the driver).
- A `MultiPolygon` (MultiSurface) is now representable **only when it has semantics**; the driver must carry it (currently `route_geometry` sends non-solid non-collection lodded geometry to `Emit`, and `write_building`'s `representable` check rejects `MultiPolygon`). Update `write_building`'s `representable` to also accept `MultiPolygon` **when it has emittable semantics**, else `lod_columns_skipped`.

**Driver:** `route_geometry` already `Emit`s any lodded geometry (only MultiSolid is special-cased). A MultiSurface reaches `write_building`; the `representable` gate there decides. Add: a `MultiSolid` (skipped) whose props carry semantics → `semantic_surfaces_dropped += count` in the driver before skipping.

**Failing tests:** extend `building.rs` unit tests — a Solid with in-code semantics emits `bldg:boundedBy` and sets `semantic_surfaces_written`; a MultiPolygon with semantics emits boundedBy and no `lodNSolid`; a MultiPolygon WITHOUT semantics returns `lod_columns_skipped`; a resolution-error geometry falls back to a plain solid + `semantic_surfaces_dropped`.

- [ ] Steps: failing tests → FAIL → implement wiring → PASS → `just check` → commit `feat(citygml): W-M3 wire semantics into write_building + driver`.

---

### Task 6: Round-trip oracles (b1 solid+semantics, railway multisurface+semantics)

**Files:**
- Modify: `crates/cityparquet/tests/citygml_writer_composite.rs` (b1 now PRESERVES semantics).
- Create: `crates/cityparquet/tests/citygml_writer_semantics_ms.rs` (railway).

**b1 oracle rewrite:** replace the W-M2 "semantics dropped/absent-after" assertions with:
- `report.semantic_surfaces_written == 9`, `report.semantic_surfaces_dropped == 0`.
- A `semantics(pkg)` extractor returning per `(id, major)` the geometry's `geometry_properties.semantics` (`surfaces` types + `values`, normalised) AND the structural boundaries (reuse Task-less `composite_structure`); assert `before == after`.
- `any_semantics(&pkg2)` is now `true` (semantics preserved).

**railway oracle (new):** `railway_lod3_fragment.gml` → package → `.gml` → package; assert the MultiSurface geometry's `semantics.surfaces` type histogram and `values` are equal across the round trip, and `semantic_surfaces_written > 0`, `multisurface_null_faces_dropped == 0`.

To compare `values` robustly, extract the decoded geometry's stored `geometry_properties.semantics` from each package (via `decode_batch`'s `props`) keyed by `(id, major)`, and assert equal `serde_json::Value`. (Reader-produced packages are canonical, so exact JSON equality holds.)

- [ ] Steps: write both oracle tests (they FAIL until Tasks 1–5 land — run last) → run → PASS → `just check` → commit `feat(citygml): W-M3 semantics round-trip oracles (b1 + railway)`.

---

## Final milestone steps

- [ ] Whole-branch review (final reviewer, most capable model).
- [ ] Codex external review: `codex exec --cd "$(pwd)" --sandbox read-only "Review W-M3 semantic-surface writer on git diff main..HEAD …"` — triage + fix Critical/Important.
- [ ] Finish the branch (verify `just check`, merge to `main`, delete branch).
- [ ] Update milestone memory (`cityparquet-rs-milestones.md`): W-M3 done; W-M4 = BuildingParts (paired reader+writer), W-M5 = appearance.

## Self-Review notes

- **Spec coverage:** dispatch-by-type + semantics (Task 5); resolution (1); id + members (2); solid+semantics (3); multisurface+semantics (4); oracles (6). All §A–§F covered.
- **Type consistency:** `Face`, `Semantics`, `IdAlloc`, `write_solid_with_semantics`/`write_multisurface_with_semantics` names consistent across tasks; counters (`semantic_surfaces_written`, `multisurface_null_faces_dropped`, kept `semantic_surfaces_dropped`) defined in Task 5, asserted in Task 6.
- **Edge cases as counters, not errors:** extension types + shape mismatch → geometry-only fallback + `semantic_surfaces_dropped`; MS null faces → `multisurface_null_faces_dropped`.
