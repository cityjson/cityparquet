Title: `CityJSON::add_cjfeature` corrupts texture UV-vertex indices when merging CityJSONSeq features into a document

## Summary

`CityJSON::add_cjfeature` (used by `cjseq_to_cj`, the merge that turns a
CityJSONSeq header + feature stream back into a single CityJSON document)
silently corrupts every geometry's texture (`material`/`texture` map's
`vertices-texture`) references whenever a feature contributes any
`appearance.vertices-texture` entries. Two independent bugs in
`src/lib.rs` compound. Both are in code paths exercised by the ordinary
`cjseq seq2cj`-style merge, on any input that has textures.

Version: 0.4.1 (crates.io / the version current at the time of filing).

## Bug 1 — `t_offset` is computed from the wrong length

`add_cjfeature` re-bases each feature's local `vertices-texture` indices
onto the merged document's accumulated pool, the same way `g_offset`
re-bases 3D vertex indices. `g_offset` correctly reads the accumulated
length *before* appending; the texture equivalent reads the current
feature's own (about-to-be-appended) local UV-vertex count instead:

```rust
// src/lib.rs, add_cjfeature, lines 245-260
let g_offset = self.vertices.len();   // correct: accumulated length so far
let mut t_offset = 0;
...
if let Some(cjf_v_tex) = &cjf_app.vertices_texture {
    t_offset = cjf_v_tex.len();       // WRONG: this feature's OWN local count,
                                       // not the accumulated pool length
    self.add_vertices_texture(cjf_v_tex.clone());
}
```

For every feature after the first, `t_offset` under-counts (or otherwise
mismatches) the true insertion point, so every UV index rebased against it
lands at an unrelated position in the merged `vertices-texture` array —
typically inside an earlier, unrelated feature's block.

## Bug 2 — `update_texture`'s rebasing conflates two incompatible use cases

`update_texture(&mut self, t_oldnew, t_v_oldnew, offset)` (`src/lib.rs`,
around line 888) is shared by two structurally different call shapes:

- **Slicing** (`get_metadata`, and both call sites inside
  `get_cjfeature`): carves a compact, feature-local array out of the
  document's global `vertices_texture`, containing only the entries that
  feature's boundaries actually reference, always called with
  `offset = 0`. Assigning each newly-seen index the next value of
  `t_v_oldnew.len()` (a first-encounter-order counter) is correct here —
  the caller immediately builds a freshly `resize`d array and gathers just
  the referenced entries at that same compacted position, e.g.:
  ```rust
  t_new_vertices.resize(t_v_oldnew.len(), vec![]);
  for (old, new) in &t_v_oldnew {
      t_new_vertices[*new] = atv[*old].clone();
  }
  ```
- **Merging** (`add_cjfeature`): appends a feature's *whole* local
  `vertices_texture` array unchanged (`self.add_vertices_texture(...)`, a
  plain `Vec::append`, no compaction) at a real, non-zero `offset`. Here
  the correct rebased index is `local_index + offset` — a direct,
  uncompacted arithmetic rebase, exactly the way `offset_geometry_boundaries`
  handles ordinary 3D vertex indices elsewhere in the same function.

`update_texture` currently uses the *slicing* (compacted-counter) logic
unconditionally for both callers, and even gets that logic internally
inconsistent on first occurrence — the memo map records one value, the
output array a different, stale one:

```rust
// src/lib.rs, update_texture, MultiSurface/CompositeSurface branch,
// approx. lines 918-926 (mirrored in the Solid branch, lines ~955-964,
// with l2/a2[i][j][k][l])
let y2 = t_v_oldnew.get(&thevalue);
if y2.is_none() {
    let l = t_v_oldnew.len();
    t_v_oldnew.insert(thevalue, l + offset);  // memo: compacted counter + offset
    a2[i][j][k] = Some(l);                    // output: compacted counter, missing "+ offset"
} else {
    let y2 = y2.unwrap();
    a2[i][j][k] = Some(*y2);
}
```

For the slicing callers (`offset` is always `0`) the missing `+ offset` on
the output is invisible (the two values coincide when `offset == 0`). For
`add_cjfeature`'s non-zero `offset`, even patching only that omission is
**still wrong**: using the compacted counter in place of the true local
index is only correct when a feature's UV indices happen to be referenced
in ascending, gapless order starting at 0 — true for many single-object
features by construction, but not for a CityJSONSeq feature that groups a
principal object with children sharing one feature-local
`vertices_texture` pool (a common shape in real CityGML data — e.g.
several sibling city objects, such as railway furniture grouped under a
parent, referencing disjoint or interleaved slices of one shared pool).
Since `city_objects` is iterated as a `HashMap` (process-randomised
order), the visible symptom is also non-deterministic across runs of the
same input.

## Minimal reproduction

Any CityJSONSeq stream where **more than one feature contributes
`appearance.vertices-texture` entries**, merged back to a single document
via `CityJSON::add_cjfeature` (i.e. `cjseq seq2cj`, or the equivalent
`cjseq_to_cj` library call), will place the second and any later feature's
texture UV coordinates at the wrong pool position. Concretely:

```rust
use cjseq::{CityJSON, CityJSONFeature};

// `header.jsonl` + `features.jsonl` are two ordinary CityJSONSeq lines
// (or however cjseq's own CLI reads them) where AT LEAST TWO features
// each carry their own `appearance.vertices-texture` array and boundary
// texture indices.
let mut doc = CityJSON::from_metadata(&header_line)?;   // however cjseq builds this
for feature_line in feature_lines {
    let mut cjf: CityJSONFeature = serde_json::from_str(&feature_line)?;
    doc.add_cjfeature(&mut cjf);
}
// doc.appearance.vertices-texture now holds every feature's UV
// coordinates, but any feature after the first has its geometry's
// texture indices pointing at the wrong entries.
```

Concretely, dereferencing the merged document's own
`appearance.vertices-texture[]` through the (now-wrong) indices recorded
in a later feature's boundaries yields UV pairs that belong to an
earlier, unrelated feature — not the values that feature's own source
line/original document carried at that position. This is reproducible
with the standard `cjseq` CLI (`cjseq seq2cj` on any multi-feature,
multi-texture CityJSONSeq stream) or with any consumer library — including
`cityparquet-rs` — that calls `cjseq_to_cj` on CityJSONSeq output covering
more than one texture-bearing feature.

## Suggested fix

### 1. `add_cjfeature`: compute `t_offset` from the accumulated pool

```diff
             if let Some(cjf_v_tex) = &cjf_app.vertices_texture {
-                t_offset = cjf_v_tex.len();
+                t_offset = self
+                    .appearance
+                    .as_ref()
+                    .and_then(|a| a.vertices_texture.as_ref())
+                    .map_or(0, |v| v.len());
                 self.add_vertices_texture(cjf_v_tex.clone());
             }
```

### 2. `update_texture`: distinguish the slicing and merging call shapes

Add a `compact: bool` parameter so the two callers get the arithmetic each
actually needs, rather than sharing one (only-sometimes-correct) scheme:

```diff
     fn update_texture(
         &mut self,
         t_oldnew: &mut HashMap<usize, usize>,
         t_v_oldnew: &mut HashMap<usize, usize>,
         offset: usize,
+        compact: bool,
     ) {
```

MultiSurface/CompositeSurface branch:

```diff
                 let y2 = t_v_oldnew.get(&thevalue);
                 if y2.is_none() {
-                    let l = t_v_oldnew.len();
-                    t_v_oldnew.insert(thevalue, l + offset);
-                    a2[i][j][k] = Some(l);
+                    let new_index = if compact {
+                        t_v_oldnew.len() + offset
+                    } else {
+                        thevalue + offset
+                    };
+                    t_v_oldnew.insert(thevalue, new_index);
+                    a2[i][j][k] = Some(new_index);
                 } else {
```

Solid branch: the identical change, on `l2`/`a2[i][j][k][l]`.

And update the four call sites: `get_metadata` and both call sites inside
`get_cjfeature` pass `true` (unchanged slicing behaviour, all already use
`offset = 0`); `add_cjfeature`'s call site passes `false`.

A full patch against 0.4.1, plus the reasoning above, is available at
<https://github.com/HideBa/cityparquet-rs> under `vendor/cjseq/` (that
project vendors a patched copy of this crate as a stopgap while this
issue is open — `vendor/cjseq/PATCHES.md` has the complete before/after
and a regression test against a real multi-texture CityJSONSeq fixture).
Happy to open a PR with this change if useful.

## Impact

Any tool that merges a CityJSONSeq stream with more than one
texture-bearing feature back into a single CityJSON document via
`cjseq_to_cj`/`add_cjfeature` silently produces a document whose texture
UV coordinates are wrong for every feature after the first that
contributes `vertices-texture` entries — a correctness bug (not a crash),
so it is easy to miss without directly comparing dereferenced UV values
against the original source.
