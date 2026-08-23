# Patches

**Base:** `cjseq` [0.4.1](https://crates.io/crates/cjseq/0.4.1) from crates.io,
copied verbatim from the local Cargo registry cache
(`cjseq-0.4.1/src/lib.rs` et al.), minus the registry's own bookkeeping
files (`.cargo-ok`, `.cargo_vcs_info.json`).

## Why this crate is vendored

`cjseq::CityJSON::add_cjfeature` (the routine `cjseq_to_cj` uses to merge a
CityJSONSeq header + features into one CityJSON document — the only path
`cityparquet-rs` calls it on is `OutputFormat::Doc`, i.e. exporting to
`.city.json`; the `.city.jsonl` export path never calls it) corrupts
texture UV-vertex indices when a dataset's features are merged. Two
independent defects in `src/lib.rs` compound:

1. **`t_offset` computed from the wrong length.** `add_cjfeature` re-bases
   each feature's local `vertices-texture` indices onto the merged
   document's accumulated pool, the same way `g_offset` re-bases 3D vertex
   indices. But where `g_offset` correctly reads the accumulated length
   *before* appending (`let g_offset = self.vertices.len();`), the texture
   equivalent read the current feature's own (about-to-be-appended) local
   UV-vertex count instead:

   ```rust
   // upstream 0.4.1, add_cjfeature:
   let g_offset = self.vertices.len();   // correct: accumulated length
   let mut t_offset = 0;
   ...
   if let Some(cjf_v_tex) = &cjf_app.vertices_texture {
       t_offset = cjf_v_tex.len();       // WRONG: this feature's own count
       self.add_vertices_texture(cjf_v_tex.clone());
   }
   ```

2. **`update_texture`'s rebased index is a compacted counter, not the
   local index plus offset.** `update_texture(&mut self, t_oldnew,
   t_v_oldnew, offset)` is a single routine shared by two structurally
   different callers:

   - `get_metadata`/`get_cjfeature` (the CityJSON→CityJSONSeq direction):
     **slices** a subset of the global `vertices_texture` array down to a
     compact, feature-local array holding only the entries that feature's
     boundaries actually reference, always called with `offset = 0`. Here
     assigning each newly-seen UV index the next value of
     `t_v_oldnew.len()` (a first-encounter-order counter) is exactly
     right: the caller then builds a freshly `resize`d array and gathers
     just the referenced entries at that same compacted position (e.g.
     `t_new_vertices.resize(t_v_oldnew.len(), vec![]); ... t_new_vertices[*new] = atv[*old]`).
   - `add_cjfeature` (the CityJSONSeq→CityJSON direction, `cjseq_to_cj`):
     **appends** a feature's *whole* local `vertices_texture` array
     unchanged (`self.add_vertices_texture(cjf_v_tex.clone())`, a plain
     `Vec::append`, no compaction, no dedup) at a real, non-zero `offset`.
     For this caller the correct rebased index is `local_index + offset`
     — a direct, uncompacted arithmetic rebase, exactly like
     `offset_geometry_boundaries` does for ordinary 3D vertex indices.

   Upstream 0.4.1 used the *same* compacted-counter logic for both
   callers and, on top of that, wrote the WRONG value even by its own
   (slicing-only-correct) logic on first occurrence — the memo map got
   the counter position, the output array got a stale, unrebased value:

   ```rust
   // upstream 0.4.1, update_texture, MultiSurface/CompositeSurface branch:
   let l = t_v_oldnew.len();
   t_v_oldnew.insert(thevalue, l + offset);  // memo: compacted counter + offset
   a2[i][j][k] = Some(l);                    // output: compacted counter, missing "+ offset" too
   ```

   (Mirrored in the `Solid` branch with `l2`/`a2[i][j][k][l]`.)

   For the *slicing* callers (`offset` always `0`) this bug happened to be
   invisible in the common case (the memo value and the output value only
   differ by `offset`, which is `0` there) — the field patched here is
   specific to `add_cjfeature`'s non-zero `offset` case. But even fixing
   only the missing `+ offset` is **insufficient** for `add_cjfeature`:
   using the compacted counter `l`/`l2` in place of the true local index
   is only correct when a feature's UV indices happen to be referenced in
   ascending, gapless order starting at 0 — which single-object features
   satisfy by construction, but multi-object CityJSONSeq features (a
   principal object plus its children sharing one feature-local
   `vertices_texture` pool — common in real CityGML data, e.g. railway
   furniture objects grouped under a parent) do not, once more than one
   object's boundaries are visited (`cjf.city_objects` is a `HashMap`,
   iterated in process-randomised order). This was confirmed empirically:
   patching only the `+ offset` omission left texture corruption on
   several — a different, HashMap-iteration-order-dependent set of —
   railway objects, each run.

   The fix therefore gives `update_texture` an explicit `compact: bool`
   parameter distinguishing the two call shapes, rather than special-casing
   inside the shared indexing logic: `true` for the three slicing call
   sites (`get_metadata`, and both call sites inside `get_cjfeature`, all
   already passing `offset = 0`, behaviour unchanged), `false` for
   `add_cjfeature`'s merge call site (uses `local_index + offset`
   directly, bypassing the counter entirely).

Both defects are entirely inside `cjseq`; nothing in `cityparquet-rs`'s own
encode/export/appearance/compare code is at fault (its `.city.jsonl`
writer never calls this merge and round-trips textures losslessly). See
`crates/cityparquet/tests/doc_export_textures.rs` for the regression test
against the real `lod3_railway.city.json` fixture (a multi-object feature
with 53 CityObjects sharing one `vertices_texture` pool, run repeatedly in
CI to guard against the HashMap-order-dependent recurrence described
above).

An upstream issue/PR is intended (draft at
`/tmp/.../scratchpad/cjseq-upstream-issue.md` at the time of writing); once
a fixed release exists on crates.io, drop this vendor directory and the
`[patch.crates-io]` entry in the workspace `Cargo.toml`, and bump the
`cjseq` version requirement instead.

## Exact hunks patched (relative to upstream 0.4.1's `src/lib.rs`)

### 1. `add_cjfeature`: `t_offset` from the accumulated pool, not the local count

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

### 2. `update_texture`: new `compact: bool` parameter

```diff
     fn update_texture(
         &mut self,
         t_oldnew: &mut HashMap<usize, usize>,
         t_v_oldnew: &mut HashMap<usize, usize>,
         offset: usize,
+        compact: bool,
     ) {
```

### 3. `update_texture`, `MultiSurface`/`CompositeSurface` branch: rebase by call shape

```diff
                                                 let y2 = t_v_oldnew.get(&thevalue);
                                                 if y2.is_none() {
-                                                    let l = t_v_oldnew.len();
-                                                    t_v_oldnew.insert(thevalue, l + offset);
-                                                    a2[i][j][k] = Some(l);
+                                                    let new_index = if compact {
+                                                        t_v_oldnew.len() + offset
+                                                    } else {
+                                                        thevalue + offset
+                                                    };
+                                                    t_v_oldnew.insert(thevalue, new_index);
+                                                    a2[i][j][k] = Some(new_index);
                                                 } else {
```

### 4. `update_texture`, `Solid` branch: same fix

```diff
                                                     let y2 = t_v_oldnew.get(&thevalue);
                                                     if y2.is_none() {
-                                                        let l2 = t_v_oldnew.len();
-                                                        t_v_oldnew.insert(thevalue, l2 + offset);
-                                                        a2[i][j][k][l] = Some(l2);
+                                                        let new_index = if compact {
+                                                            t_v_oldnew.len() + offset
+                                                        } else {
+                                                            thevalue + offset
+                                                        };
+                                                        t_v_oldnew.insert(thevalue, new_index);
+                                                        a2[i][j][k][l] = Some(new_index);
                                                     } else {
```

### 5. The four call sites of `update_texture`: pass the new argument

```diff
 // get_metadata (slicing geometry-template textures for the Seq header line):
-                    g.update_texture(&mut t_oldnew, &mut t_v_oldnew, 0);
+                    g.update_texture(&mut t_oldnew, &mut t_v_oldnew, 0, true);

 // get_cjfeature, principal object:
-                    g.update_texture(&mut t_oldnew, &mut t_v_oldnew, 0);
+                    g.update_texture(&mut t_oldnew, &mut t_v_oldnew, 0, true);

 // get_cjfeature, each child object:
-                        g.update_texture(&mut t_oldnew, &mut t_v_oldnew, 0);
+                        g.update_texture(&mut t_oldnew, &mut t_v_oldnew, 0, true);

 // add_cjfeature (the merge path this vendoring exists to fix):
-                    g.update_texture(&mut t_oldnew, &mut t_v_oldnew, t_offset);
+                    g.update_texture(&mut t_oldnew, &mut t_v_oldnew, t_offset, false);
```

No other lines in `src/lib.rs` are changed; `main.rs`/`wasm.rs`/tests/data
are carried unmodified for reference and are not built by
`cityparquet-rs` (only the `cjseq` library target is depended on, via
`[patch.crates-io]` in the workspace `Cargo.toml`).
