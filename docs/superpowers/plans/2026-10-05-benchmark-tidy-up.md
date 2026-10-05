# Plan: tidy up the benchmark for the paper (§5)

A high-level overview of the work that brings the benchmark in line with the paper's evaluation chapter. It complements the handout (`2026-10-05-benchmark-paper-handout.md`), which lists what the paper expects; this document says what we do and in which order. Each task gets its own short design before it starts.

## How we work

- The commander session plans, briefs and verifies; an Opus subagent implements.
- Small commits on `develop` in this repository, nothing pushed until the author has looked. The parent paper repository's submodule pointer is bumped once, when a task is finished.
- Code only. The author edits the paper text separately.
- Citable numbers come from the benchmark host. On a laptop we change code, run the test suites and smoke runs, and re-render from committed evidence.

## Tasks, in order

### 0. Remove the codec and row-group axes — done

Recipes, evidence, figures, tests and documentation are gone (`22fc32b`..`5c71188`). The bloom axis is the only configuration axis left.

### 1. Remove the write benchmark

Drop the measurement of how fast each format is written: the timed-write script and recipe, the write rows in the heatmaps, their evidence, tests and documentation. The reason is fairness: not every format has a native writer, so the files are converted through a common format and the timings do not compare like with like.

Decided:

- The bloom axis loses its write time as well; it keeps its read scenarios and its file sizes.
- The database comparison keeps its write tier (add, update, delete, append): there it is one of the queries.

### 2. Make CityGML the baseline

Every relative value answers "CityParquet is x times faster, or smaller, than CityGML". Today the size plot and the heatmaps divide by CityJSONSeq.

- Switch the baseline in the size plot and the heatmaps, with their titles and captions.
- Emit the ratios as data, not only as colours: the size table of the handout (factor against CityGML, best and worst dataset) and the per-query ratios against CityGML. CSV only; the author converts it to Typst.
- One figure per dataset and metric, as SVG and PNG, in a folder per dataset: `formats/<dataset>/time.{svg,png}` and `formats/<dataset>/rss.{svg,png}`.
- A missing, skipped or failed CityGML cell leaves the ratio explicit as unavailable, never zero.
- The database figure keeps its own baseline (3DCityDB).

Two small fixes ride along: the specification site's benchmark page says timings are medians where the evidence reports means, and `just bench-summary` calls bare `python3`, which fails on Python older than 3.11, so it runs through `uv`.

### 3. Interactive review of the benchmark code

After tasks 1 and 2, the author reviews what remains, one stage at a time: corpus and preparation, the read harness and its queries, size measurement, aggregation, plots. The commander explains each stage, shows the evidence it produced and flags doubts; the author confirms or corrects before the next stage. Findings become small fixes or entries in this plan.

Already noted for the review:

- The handout's smaller items: the "1,000,001 objects" title, the repetition counts, the memory actually available on the host, the table of tool versions, the dataset list with URLs and attribute predicates, the geometry share from `parquet_metadata`.

### 4. Add the benchmark over the network

Run the format read benchmark over HTTP against object storage and report time, bytes read and request count per query (paper §5.4). The harness already has an HTTP transport (`--transport http`), used so far only by `bloom-bench-http`.

- A recipe that runs the format benchmark over HTTP for the corpus, and an upload step for the prepared artefacts.
- Bytes read and request count in the results, and a figure or table for them.
- The run records the storage location and the network path, and states that it is a snapshot of one path at one time.

The author supplies the URLs where the whole corpus is hosted when this task starts. To decide then: which formats take part.

## Afterwards

A full re-run on the host with the final harness, then the figures and tables are regenerated and the paper's placeholders filled.
