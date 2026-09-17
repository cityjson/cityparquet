# Benchmarks

The suite compares file formats, CityParquet configurations and database query
systems. Run the three public commands from the repository root:

```sh
just bench-prep
just bench-run
just bench-summary
```

Preparation downloads and prepares inputs; running measures them; summarising
renders existing results. Rendering never starts a benchmark.

On this machine the data and output root is
`benchmark/runs/`. Its layout is:

| Path | Contents |
| --- | --- |
| `data/benchmark/`, `data/scaling/` | Source corpus and nested 3DBAG slices |
| `data/readbench/` | Prepared format artefacts |
| `formats/results/`, `formats/scaling_{codec,rowgroup}_results/` | Full format and configuration measurements |
| `formats/smoke/` | Isolated smoke measurements |
| `databases/{prepared,results,smoke}/` | Database lifecycle inputs and measurements |
| `summary/{full,smoke}/` | Rendered figures and combined HTML |

Benchmark inputs, derived artefacts, results and rendered summaries are generated
beneath this ignored directory. The paper checkout may explicitly export figures
to `paper/assets/bench/`.

## Selecting work

```sh
just bench-prep --families formats
just bench-run --families codec,rowgroup
just bench-run --datasets 3dbag --smoke
just bench-summary --data-root benchmark/runs
```

The family names are `sizes`, `formats`, `codec`, `rowgroup` and `databases`.
With no selection, the suite includes all five. Use each command's `--help`
for its selection and output options. Smoke runs validate the pipeline with
small inputs and fewer repetitions; their results are not publication runs.

## Full run

Run the full matrix only after the smoke run succeeds:

```sh
just bench-prep --data-root benchmark/runs
just bench-run --data-root benchmark/runs
just bench-summary --data-root benchmark/runs
```

Preparation fetches the six-file corpus (about 423 MB), uses the pinned 7.6 GB
3DBAG FlatCityBuf source to make seven nested slices through the nominal
one-million-object prefix, prepares all required format artefacts, builds the
release CityParquet CLI, and prepares the database environment. It needs Rust
and Cargo, Java 17 or later, `fcb`, `cjseq`, the pinned citygml-tools archive,
Python with `uv`, and rootless Podman plus the database images for the database
family. The full run writes substantial prepared packages and result files
under the data root and takes hours; the database family also needs its own
container storage and available local ports. `bench-summary` only reads those
results.

Use `--families` to run one family after preparing it. `--datasets` accepts
manifest IDs, `3dbag` for the scaling series, and `largest` for the largest
slice. A selected database run requires `largest`; its normal dataset is the
largest 3DBAG slice. The selector rejects data roots outside `benchmark/runs/`.

## Experimental matrix

| Family | Data | Measurements | Read queries |
| --- | --- | --- | --- |
| `sizes` | Corpus with the largest 3DBAG scaling slice | Complete file or package size | None |
| `formats` | Same corpus | Write time and peak memory; read time and peak memory | All format queries |
| `codec` | Nested 3DBAG scaling slices | Size and the four performance metrics | Full read, bbox windows, middle-position ID |
| `rowgroup` | Same slices | Same metrics | Same queries |
| `databases` | Largest 3DBAG slice | Storage including indexes; mean query time and peak memory | Database query suite |

The corpus retains Rotterdam, Ingolstadt, Vienna, New York and Zurich, and
uses the largest scaling slice for 3DBAG. Dataset IDs identify artefacts;
figures use readable display names. The scaling generator takes whole features
from a pinned FlatCityBuf source in source order. Slices are nested prefixes,
not replicated objects. Actual CityObject counts can exceed the nominal target
because a feature is indivisible; the recorded counts determine plot positions.

The format comparison displays the Hilbert-ordered configuration as
**CityParquet**. Its internal configuration ID remains distinct from source
order. Codec experiments hold ordering and row-group size fixed; row-group
experiments hold ordering and codec fixed. Codec levels are not matched for
compression effort across codec families.

## Figures

`just bench-summary` produces individual SVG and PNG files and a self-contained
`index.html` collecting the same figures and their conditions.

| Figure | Content |
| --- | --- |
| `sizes` | Vertical size bars, one subplot per dataset; actual sizes and ratios to CityJSONSeq |
| `heatmap` | One panel per dataset, with write time, write memory, read time and read memory heatmaps |
| `codec`, `rowgroup` | Five metric panels for the largest measured scaling dataset |
| `codec-scaling`, `rowgroup-scaling` | Absolute metrics against actual CityObject counts |
| `databases` | Storage bars and query time/memory heatmaps |

Heatmap colours encode measurement divided by baseline: **lower is better**,
with 1× neutral. Cell labels show actual values and units. Format comparisons
use CityJSONSeq; configuration comparisons use their fixed default; database
comparisons use **3DCityDB**. A missing baseline is not replaced by another
system. Missing, unsupported or failed measurements remain explicit rather
than becoming zeros. The combined HTML reports incomplete coverage.

## Evidence and interpretation

Keep source identity, query parameters, software revisions, machine information
and repetition settings with measurements. Do not combine old write runs and
new read runs as if they were one experiment. Database memory must cover the
execution system, not only its client process; the database methodology defines
the measurement boundary and sampling limitations.

Read the detailed methodology before citing a result:

- [Format queries and fairness caveats](formats/READ_BENCHMARK.md)
- [Writing and configuration experiments](formats/README.md)
- [Database measurements](databases/README.md)

Relevant qualifications include CityGML synthesis and possible information
loss, feature-versus-CityObject counting grain, source-position-dependent ID
lookups, and small timing differences relative to repetition spread. Preserve
these qualifications in captions and reports. Database ingest time is separate
from the query comparison.

## Layout and checks

`readbench/` is a separate Rust workspace for the format harness. `scripts/`
holds preparation and orchestration. `formats/` and `databases/` own their
measurement artefacts. `plot/` renders them; `summary/` holds generated output.
Large source files and prepared packages are fetched or generated, not committed.

Run `just plot-test`, `just scripts-test`, the readbench Rust tests and the
database unit tests when changing the corresponding components. Benchmarks are
separate from CI checks: complete runs require the corpus, external converters
and database containers, and can take hours.
