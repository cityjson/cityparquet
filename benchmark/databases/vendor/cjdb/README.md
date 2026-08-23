# Patched cjdb

`citybench.systems.cjdb.CjdbSystem` drives a **patched** build of
[cjdb](https://github.com/tudelft3d/cjdb) `2.2.0`, not stock cjdb from
PyPI. This directory holds the patch and explains why.

## The defect

`cjdb/modules/geometric.py`'s `get_ground_surfaces()` derives an
object's 2D footprint (`ground_geometry`) by collecting every non-vertical
face of its lowest-LoD solid, then keeping only the ones below the mean
height of those faces (a "ground vs roof" split). Stock cjdb 2.2.0
accumulates the candidate faces into a `dict` keyed by each face's own
mean Z height:

```python
ground_surfaces = {}
for polygon in polygons:
    ...
    z = mean([point[2] for point in polygon.exterior.coords])
    ground_surfaces[z] = force_2d(polygon)   # a tie silently overwrites
```

Any two non-vertical faces that happen to share a mean Z overwrite one
another — only the last one processed survives. This is an entirely
ordinary shape for a building with a flat-planed roof or ground: nothing
contrived about it. Measured directly against the delft fixture this
benchmark ships (`benchmarking/data/delft.city.jsonl`, 2231 CityObjects):
**9 of the 1116 BuildingParts** that reach this code path have two or more
tied faces, silently losing up to 8 of them in one case. The practical
effect is a `ground_geometry` polygon that is smaller — sometimes much
smaller — than the object's true footprint, which then makes cjdb
undercount spatial queries (`bbox-query`) for reasons that have nothing to
do with cjdb's actual row-oriented/JSONB architecture.

Full investigation, the exact affected-object measurement, and the
methodology behind it are in this project's Task 12 fix report
(`.superpowers/sdd/2026-07-31-database-benchmark-harness/task-12-report.md`,
not distributed with cjdb itself). A minimal upstream bug report is drafted
there too.

## Why patch it rather than benchmark it as-is

This benchmark's claim is about **architecture** — cjdb's row-oriented
PostgreSQL/JSONB design against CityParquet's columnar one — not about one
release's importer having a footprint-derivation bug. Benchmarking the
bug instead of the architecture would be attacking a strawman, unfair to
cjdb, and would weaken rather than strengthen the comparison. So this
project benchmarks a **corrected** cjdb, and says so, everywhere a number
derived from it appears:

- `results/<dataset>.manifest.json`'s `patches.cjdb` (built by
  `citybench.systems.cjdb.patch_disclosure()`) and `versions.cjdb`.
- `benchmarking/README.md`.
- This file.

## The patch

`ground-surfaces-tie.patch` — a minimal, unified diff against
`cjdb/modules/geometric.py`, applying with `patch -p1` from the extracted
sdist root. It changes exactly one thing: `ground_surfaces` becomes a
`list` of `(z, polygon)` pairs instead of a `dict` keyed by `z`, so a tied
face is appended, never overwritten.

**Deliberately NOT changed**: the split threshold. Stock cjdb computes
`z_mean = mean(ground_surfaces.keys())` — the mean of the *distinct* Z
values, because a dict's keys are unique by construction. The patch
preserves this exactly (`z_mean = mean({z for z, _ in ground_surfaces})`),
rather than taking the mean over every retained `(z, polygon)` pair, which
would silently reweight the threshold by how many faces happen to share
each Z — a second, separate semantic change that has nothing to do with
the face-dropping bug and is not bundled into this fix. (Whether a
count-weighted mean would in fact be a *better* split rule is a separate
question, noted but not acted on — see the Task 12 fix report.)

## Building it

```
just patch-cjdb
# or directly:
./scripts/patch_cjdb.sh
```

Downloads the pinned `cjdb-2.2.0.tar.gz` sdist from PyPI (checksum-verified
against a pinned SHA-256 — see the script), extracts it, and applies
`ground-surfaces-tie.patch` with `patch -p1`. The result is written to
`benchmarking/.cjdb-patched/cjdb-2.2.0+<patch-hash>/` — a directory whose
name embeds the patch file's own sha256 prefix, **not** committed
(git-ignored, like `data/`): it is a deterministic build artefact,
reproducible from this committed patch by anyone who runs the command
above, not source.

The content-addressed naming is deliberate, not decorative: `uv run --with
<local-dir>`'s own build cache does not reliably notice an in-place source
change at a *fixed* path (confirmed while building this mechanism — even
`uv cache clean cjdb --force` served a stale, pre-patch build from a path
that had been rebuilt with new content; only a full `uv cache clean` or a
genuinely new path picked up the change). A fixed output path would risk
silently continuing to run an old patched build after `ground-surfaces-tie.patch`
itself was edited. `CjdbSystem`'s `patched_cjdb_source()` additionally
checks the resolved path's name against the *current* patch file's hash on
every call and raises if they disagree, rather than trusting a stale
pointer file.

## Licence

cjdb is MIT-licensed, copyright TU Delft — the same group as this paper's
work — see `LICENSE` inside the built source tree (downloaded, not
committed here) or the [upstream repository](https://github.com/tudelft3d/cjdb).
