# Handout: benchmark and visualisation changes for the paper (§5)

The paper (`paper/chapters/05-evaluation.typ` in the parent repo) has been restructured. The benchmark and plot scripts must produce what it now expects. Evidence lives under `benchmark/runs/`; do not edit the paper itself.

## 1. Size table (§5.1, `tab:eval-sizes`)

- Per dataset: size on disk in CityGML 2.0, CityJSON, CityJSONSeq, FlatCityBuf, CityParquet.
- Compression factor per format **against CityGML** (CityGML size ÷ format size; larger = smaller file).
- Output as CSV **and** as Typst table rows, matching the paper's columns: Dataset, City objects, CityGML size, then size and × for each other format.
- Generic over the corpus: datasets may be added (for example a PLATEAU Tokyo subset).
- Also report the best and worst datasets by CityParquet factor.

## 2. Geometry share on the 1M 3DBAG slice

- From `parquet_metadata` on the CityParquet file: the share of the file taken by the `geometry_lod*` columns, and their compression ratio (uncompressed ÷ compressed).
- The paper has placeholders for both (`x %`, `x-fold`). For reference, the 3DBAG Delft file gives about 90 % and about 5×.

## 3. Plots

- Size plot: switch the baseline from CityJSONSeq to **CityGML**, to match the table. The author may drop the plot later.
- The 3DBAG title says "1,000,001 objects", but the paper says 1,000,000. Round the title, or cut the slice at exactly 1,000,000.
- Unsupported, skipped or failed cells must stay explicit in every figure, never zero.

## 4. Run protocol and provenance

- Repetitions: the last run used 7 read repetitions. Confirm with the author (7/7 was mentioned) and record the value used.
- Record the memory actually available on the shared host, not only the installed 540 GB.
- Emit a table of tool versions and the benchmark commit (cityparquet-rs, cjseq, fcb, citygml-tools, DuckDB, PostgreSQL/PostGIS).

## 5. Pending measurements

- Network (§5.4): the format benchmark over HTTP against object storage, reporting time, bytes read and request count.
- Configuration (§5.3): file size with and without Bloom filters.

## 6. Repository

- `benchmark/formats/` (linked from the paper at `tree/main/benchmark/formats`) must list every dataset with its download URL **and** its attribute predicate. Check the branch.

## Not in scope, but be aware

- Format renames are pending (`geometry_templates` → `implicit_geometries`, `template` → `implicit_geometry`, and an extension-namespace prefix instead of `ex_`). See `docs/superpowers/specs/2026-10-03-citygml-cm-naming-audit.md`. Two benchmark files mention `geometry_templates`.
- Optional: convert a CityJSON cube with a cubic cavity and check the paper's §3.3 listing (face order, `shells: [[6, 6]]`, orientation of the interior faces).
