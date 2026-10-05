//! The read-benchmark COORDINATOR: `cityparquet-readbench run ...`.
//!
//! Everything Tasks 8-10 built is a `--child` process that runs exactly one
//! (format, scenario) measurement and prints one line to stdout. This module
//! is the piece that actually drives a whole benchmark matrix: for every
//! requested (format, scenario) pair it derives real [`QueryParams`] from the
//! data itself (never a hardcoded id/attribute/bbox), spawns the `--child`
//! process `repeat` times (plus one discarded warmup) via
//! [`std::env::current_exe`], means the timings (+ the population standard
//! deviation), takes the MAX
//! peak heap/RSS across the repeats, computes `selectivity`, and holds one
//! row for the results CSV — which this module owns outright (a fresh
//! truncate-and-write per run, never an append, so a re-run is always
//! clean). The rows are written once the whole matrix has run, so that a
//! run-level finding can still reach the rows it concerns (see
//! "Self-consistency" below).
//!
//! **Where `QueryParams` come from.** Every one of them is derived by
//! [`cityparquet_readbench::params::resolve`], which this module calls once
//! per dataset and whose result it also writes beside the CSV as
//! `<out>.params.json` — the single description of what a run measured, read
//! back by the renderer (`benchmark/plot`) and hashed into the run manifest.
//!
//! That derivation REQUIRES the `cityparquet` package (`<x>.parquet`)
//! regardless of which `--formats` were requested: it is the source of the
//! row bboxes, the attribute choices and the shared denominator. The
//! `cityjsonseq` stream (`<x>.city.jsonl`) is the canonical order the id
//! deciles are cut from, and the `citygml` artefact is checked so an id
//! probe it does not contain is substituted rather than timed as a silent
//! miss; each is used when present, and a run whose prepared directory has
//! no seq artefact simply gets no id probes and a logged skip — the same
//! "never fabricated" treatment a dataset with no numeric attribute gets.
//!
//! Two scenarios emit MORE THAN ONE ROW per format as a result:
//! [`Scenario::BBoxQuery`] one per searched window (`bbox-1pct`,
//! `bbox-5pct`, `bbox-25pct`, each `;approx` when the target row fraction
//! was not reachable), and [`Scenario::IdLookup`] one per probe
//! (`id-10pct`, `id-50pct`, `id-90pct`, `id-miss`).
//! [`Scenario::FeatureLookup`] (CityParquet only, and only when named) emits
//! two: `feature-50pct` and `feature-miss`. Every CityParquet lookup row
//! carries its [`LookupCounters`] in the three trailing CSV columns.
//!
//! **`--variants`: the configuration run.** Given variant ids rather than
//! formats, this module compares ONE format's writer configurations instead
//! of comparing formats: per variant it converts the prepared CityJSONSeq
//! artefact with that variant's recipe (untimed — building a package is not
//! a measurement here), keeps the package as
//! `<prepared_dir>/<base>.<id>.parquet`, and then runs the ordinary read
//! children against it. The CSV keeps its shape — the variant id goes in
//! the `format` column — and the packages' sizes go to `sizes.csv` beside it
//! (see [`write_sizes`]). Every package is built before any read runs; the
//! rows are sorted back into per-variant groups before they are written.
//! Over `--transport http` the run is read-only: it reads the
//! `<base>.<id>.parquet` packages a local run built, uploaded beside the
//! prepared artefacts, and writes no `sizes.csv`.
//!
//! **Cross-format consistency (a hard failure).** Once the matrix has run,
//! every row's `result_count` is checked ([`check_consistency`]): `count`,
//! `full-read`, each `bbox-*` window and `attr-filter` against the reference
//! the parameters were derived with, at the row's own counting level
//! (CityObjects or features); `attr-stats` and each `id-*` probe for
//! equality across formats. A disagreeing row is tagged
//! [`COUNT_MISMATCH`] in `notes`, the CSV is still written so the evidence
//! survives, and the run then exits non-zero naming the scenario, the
//! formats and the counts. A runner that fell back from an index to a full
//! scan discloses it in `notes` too (see [`ChildLine::notes`]).

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use cityparquet::package::RowOrder;
use cityparquet::variant::Variant;
use cityparquet_readbench::format::Format;
use cityparquet_readbench::naming::strip_known_extension;

use crate::formats::{IoStats, LOOKUP_STATS_MARKER, LookupCounters, Source};
use crate::scenario::{AttrPred, QueryParams, Scenario};
use cityparquet_readbench::params;

/// The `run` subcommand's own options — the parsed form of `main.rs`'s
/// `RunArgs` clap struct, kept independent of clap so this module has no
/// CLI-parsing concerns of its own.
#[derive(Debug, Clone)]
pub struct RunOptions {
    /// The ORIGINAL CityGML/CityJSON/CityJSONSeq input. It is NEVER measured
    /// — it names the dataset (and, stripped of its extension, every
    /// artefact in `prepared_dir`), and every measured byte comes from an
    /// artefact `benchmark/scripts/readbench_prepare.sh` built there.
    pub input: PathBuf,
    /// Directory `just readbench-prepare` wrote the per-format artefacts
    /// into (default `benchmark/runs/data/readbench`).
    pub prepared_dir: PathBuf,
    /// Result CSV path; this run OWNS the file (fresh truncate + write).
    pub out: PathBuf,
    /// Warm repeats per measurement (a further, discarded warmup precedes
    /// every one). Must be >= 1.
    pub repeat: usize,
    /// Requested formats; `None`/empty selects [`Format::ALL`], the
    /// format comparison.
    pub formats: Option<Vec<Format>>,
    /// Requested variant ids (`cityparquet::variant`'s grammar). When set,
    /// this is a CONFIGURATION run rather than a format comparison: every id
    /// is converted into `<prepared_dir>/<base>.<id>.parquet` and then read
    /// by the CityParquet runner. Exclusive with `formats`. Over HTTP the
    /// packages are read, never built.
    pub variants: Option<Vec<String>>,
    /// Requested scenario names (canonical [`Scenario::as_str`] spelling,
    /// case-insensitive); `None`/empty selects every [`Scenario::ALL`].
    pub scenarios: Option<Vec<String>>,
    /// Optional subset of resolved `id-lookup` probe tags. Configuration runs
    /// use this to hold lookup position at 50%; format comparisons retain all.
    pub id_probes: Option<Vec<String>>,
    /// Optional subset of resolved `feature-lookup` probe tags.
    pub feature_probes: Option<Vec<String>>,
    /// After the warm matrix, run one additional `FullRead` per format,
    /// tagged `cold` in `notes` (see [`run`]'s own doc comment on the
    /// `sudo purge` protocol this does NOT automate).
    pub cold: bool,
    /// Transport for every measurement this run drives (see [`Transport`]).
    pub transport: Transport,
    /// HTTP base URL; required when `transport` is [`Transport::Http`].
    pub base_url: Option<String>,
}

/// `run`'s own transport selector — the CLI-facing mirror of
/// `formats::Source`'s two variants (that enum instead carries a resolved
/// per-artefact base_url+key/path).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Local,
    Http,
}

/// The exact CSV header this coordinator writes. `bytes_read`/`http_requests`
/// are empty for a local-transport row (no HTTP concept, and this keeps
/// every existing local row's own field values unchanged; see [`Row::render`]'s
/// own `io: None` handling) and populated for an
/// http-transport row from the wrapped `ObjectStore`/range-client tally each
/// `FormatRunner`'s `Source::Http` arm reports (see `formats::IoStats`).
/// The last three are a CityParquet lookup's [`LookupCounters`], empty on every other row.
const CSV_HEADER: &str = "dataset,format,scenario,selectivity,result_count,time_s,time_std_s,\
peak_heap_bytes,peak_rss_bytes,repeat,notes,bytes_read,http_requests,row_groups_total,\
bloom_pruned,filter_bytes";

/// The resolved-parameters sidecar for a results CSV: the CSV's own path with
/// `.params.json` appended, so the two travel together and a run cannot leave
/// a stale sidecar behind for a different CSV.
pub fn params_sidecar_path(out: &Path) -> PathBuf {
    let mut name = out.as_os_str().to_os_string();
    name.push(".params.json");
    PathBuf::from(name)
}

/// Raw child-process samples beside the aggregate CSV. Warmups are retained
/// and labelled, so a published aggregate can always be recomputed.
#[derive(Debug, serde::Serialize)]
struct Sample {
    dataset: String,
    format: String,
    scenario: String,
    query_tag: String,
    sample_index: usize,
    warmup: bool,
    time_s: f64,
    peak_rss_bytes: u64,
    peak_heap_bytes: u64,
    result_count: u64,
}

fn samples_sidecar_path(out: &Path) -> PathBuf {
    let mut name = out.as_os_str().to_os_string();
    name.push(".samples.json");
    PathBuf::from(name)
}

fn write_samples(out: &Path, samples: &[Sample]) -> Result<()> {
    let target = samples_sidecar_path(out);
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("creating sample sidecar beside {}", target.display()))?;
    serde_json::to_writer_pretty(&mut temporary, samples).context("serialising raw samples")?;
    temporary
        .persist(&target)
        .map_err(|error| error.error)
        .with_context(|| format!("atomically writing {}", target.display()))?;
    Ok(())
}

/// Runs `opts`'s whole (format x scenario) matrix, writing `opts.out` fresh.
pub fn run(opts: &RunOptions) -> Result<()> {
    if opts.repeat == 0 {
        bail!("--repeat must be >= 1");
    }
    if opts.transport == Transport::Http && opts.base_url.is_none() {
        bail!("--transport http requires --base-url");
    }
    // A CSV is either a format comparison or a configuration run; the two
    // answer different questions and share one `format` column.
    let variants = match (&opts.formats, &opts.variants) {
        (Some(f), Some(v)) if !f.is_empty() && !v.is_empty() => bail!(
            "--formats and --variants are exclusive: a CSV is either a format comparison or a \
             configuration run, never both"
        ),
        (_, Some(v)) if !v.is_empty() => Some(parse_variant_list(v)?),
        _ => None,
    };

    let dataset = opts
        .input
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "cannot derive a dataset name from input path {}",
                opts.input.display()
            )
        })?;
    let base = strip_known_extension(&dataset);

    // The cityparquet package is REQUIRED regardless of `--formats`: every
    // QueryParams derivation below reads from it (see this module's own doc
    // comment).
    let cp_table = locate_cityparquet_table(&opts.prepared_dir, base)?;

    // The measured sources, each with the LABEL its `format` column carries
    // (a format name here; a variant id on a `--variants` run). A
    // configuration run resolves nothing in this block: its sources are the
    // packages it has yet to build, so it is filled in after the CSV header
    // is written, below.
    let mut resolved_formats: Vec<(Format, Source, String)> = Vec::new();
    if variants.is_none() {
        // Who chose the format list matters to how a skip below is reported: an
        // operator who NAMED a format is told about it once, in passing; a
        // default-set run that quietly measured a subset would otherwise be
        // published as "the format comparison" (see the summary after this loop).
        let (requested_formats, chosen_by_default): (Vec<Format>, bool) = match &opts.formats {
            Some(v) if !v.is_empty() => (v.clone(), false),
            _ => (Format::ALL.to_vec(), true),
        };

        let mut skipped_formats: Vec<Format> = Vec::new();
        for &format in &requested_formats {
            match resolve_format_artefact(
                format,
                &opts.prepared_dir,
                base,
                opts.transport,
                opts.base_url.as_deref(),
            ) {
                ArtefactResolution::Source(Source::Local(path)) if path.exists() => {
                    resolved_formats.push((
                        format,
                        Source::Local(path),
                        format.as_str().to_string(),
                    ));
                }
                ArtefactResolution::Source(Source::Local(path)) => {
                    skipped_formats.push(format);
                    eprintln!(
                        "cityparquet-readbench: skipping format '{format}': missing artefact {} \
                         (run `just readbench-prepare {}` first)",
                        path.display(),
                        opts.input.display()
                    )
                }
                // Optimistic, no existence check: a missing remote object
                // surfaces as a natural child-process error, not a preflight
                // HEAD request this coordinator would otherwise need to make.
                ArtefactResolution::Source(source @ Source::Http { .. }) => {
                    resolved_formats.push((format, source, format.as_str().to_string()));
                }
                ArtefactResolution::NonUtf8Key => {
                    skipped_formats.push(format);
                    eprintln!(
                        "cityparquet-readbench: skipping format '{format}': its artefact path is \
                         not valid UTF-8, so no HTTP key can be derived from it"
                    )
                }
            }
        }
        if resolved_formats.is_empty() {
            bail!(
                "no requested format has a present artefact for dataset '{dataset}'; nothing to \
                 run (run `just readbench-prepare {}` first)",
                opts.input.display()
            );
        }
        // A skip is one line of noise when the operator NAMED the format — they
        // know what they asked for. When the coordinator itself chose the
        // format-comparison set, an incomplete run produces a CSV that LOOKS like
        // the comparison but silently omits formats, so it is reported as the
        // spoiled result it is (loudly, and only for the default set).
        if chosen_by_default && !skipped_formats.is_empty() {
            let missing: Vec<&str> = skipped_formats.iter().map(|f| f.as_str()).collect();
            eprintln!(
                "cityparquet-readbench: WARNING: this run measured {} of the {} formats in the \
                 default format-comparison set, so its results are not a complete format \
                 comparison; missing: {}. Run `just readbench-prepare {}` to build every \
                 artefact, or pass an explicit --formats to measure a subset deliberately.",
                resolved_formats.len(),
                requested_formats.len(),
                missing.join(", "),
                opts.input.display()
            );
        }
    }

    let scenarios: Vec<Scenario> = match &opts.scenarios {
        Some(v) if !v.is_empty() => v
            .iter()
            .map(|s| s.parse().map_err(|e: String| anyhow::anyhow!(e)))
            .collect::<Result<Vec<_>>>()?,
        _ => Scenario::ALL.to_vec(),
    };

    // --- Every QueryParams for this dataset, derived once from the prepared
    // artefacts (see `cityparquet_readbench::params`) — no hardcoded ids,
    // attributes or windows anywhere in this function.
    //
    // TWO artefacts are required regardless of `--formats`: the CityParquet
    // package (the row bboxes, the attribute choices, the denominator) and
    // the CityJSONSeq stream (the canonical order the id deciles are cut
    // from). The CityGML artefact is checked when present, so an id probe it
    // does not contain is substituted rather than timed as a silent miss.
    let seq_path = Some(opts.prepared_dir.join(Format::CityJsonSeq.artefact(base)))
        .filter(|path| path.exists());
    let gml_path =
        Some(opts.prepared_dir.join(Format::CityGml.artefact(base))).filter(|path| path.exists());
    let mut resolved = params::resolve(
        &dataset,
        &cp_table,
        seq_path.as_deref(),
        gml_path.as_deref(),
    )?;

    retain_requested_probes(
        "id-probes",
        &mut resolved.id_probes,
        opts.id_probes.as_ref(),
    )?;
    retain_requested_probes(
        "feature-probes",
        &mut resolved.feature_probes,
        opts.feature_probes.as_ref(),
    )?;

    eprintln!(
        "cityparquet-readbench: derived params for '{dataset}': windows={:?}, attr-filter={}, \
         numeric attribute={:?}, id probes={:?}, feature probes={:?}, CityObject total={}",
        resolved
            .windows
            .iter()
            .map(|w| (w.tag.as_str(), w.achieved, w.approx))
            .collect::<Vec<_>>(),
        match &resolved.attr_filter {
            Some(spec) => format!(
                "{} (n={}, {:.1}% of rows, {})",
                spec.notes_tag(),
                spec.matched,
                spec.share * 100.0,
                if spec.hand_picked {
                    "hand-picked"
                } else {
                    "derived"
                }
            ),
            None => "none (skipped)".to_string(),
        },
        resolved.numeric_attr,
        resolved
            .id_probes
            .iter()
            .map(|p| (p.tag.as_str(), p.id.as_str()))
            .collect::<Vec<_>>(),
        resolved
            .feature_probes
            .iter()
            .map(|p| (p.tag.as_str(), p.id.as_str()))
            .collect::<Vec<_>>(),
        resolved.cp_object_total
    );

    if let Some(parent) = opts.out.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }

    // The resolved parameters, beside the CSV this run owns. It is the ONE
    // description of which windows, ids and attributes this run measured,
    // read back by the renderer and hashed into the run manifest.
    let sidecar = params_sidecar_path(&opts.out);
    fs::write(
        &sidecar,
        serde_json::to_string_pretty(&resolved)
            .context("serialising the resolved query parameters")?,
    )
    .with_context(|| format!("writing {}", sidecar.display()))?;
    let mut csv = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&opts.out)
        .with_context(|| format!("creating {}", opts.out.display()))?;
    writeln!(csv, "{CSV_HEADER}").context("writing CSV header")?;

    // Every row is HELD until the whole matrix has run, then written in one
    // go (the header above is written immediately, so a run that dies midway
    // still leaves a clean, empty CSV rather than the previous run's rows).
    // Holding them is what lets a RUN-LEVEL disclosure — the cross-format
    // self-consistency check below — reach the `notes` column of the rows it
    // concerns instead of living only on stderr, where a spoiled run looks
    // exactly like a clean one to anyone reading the artefact.
    let mut rows: Vec<Row> = Vec::new();
    let mut samples: Vec<Sample> = Vec::new();

    // A configuration run's own sources: one package per variant, measured
    // into `sizes`, each the package the read children below then run
    // against. It happens here, after the sidecar and the header, so a run
    // that dies in a conversion still leaves a clean, empty CSV and the
    // parameters it was going to measure with.
    //
    // Order: every package is built first, then every read runs, so no read
    // shares the page cache with a conversion in flight; the CSV is sorted
    // back into per-variant groups at the end (see the sort before the rows
    // are written).
    let mut sizes: Vec<SizeRow> = Vec::new();
    let variant_seq: Option<PathBuf> = match (&variants, opts.transport) {
        (Some(_), Transport::Local) => Some(seq_path.clone().ok_or_else(|| {
            anyhow::anyhow!(
                "--variants needs the prepared CityJSONSeq artefact {}.city.jsonl to convert \
                 from (run `just readbench-prepare {}` first)",
                base,
                opts.input.display()
            )
        })?),
        _ => None,
    };
    if let Some(list) = &variants {
        for (id, variant) in list {
            match opts.transport {
                Transport::Local => {
                    let seq = variant_seq
                        .as_deref()
                        .expect("set for every local `variants` run");
                    let package = build_variant(base, id, variant, &opts.prepared_dir, seq)?;
                    sizes.push(SizeRow {
                        dataset: base.to_string(),
                        label: id.clone(),
                        bytes: dir_bytes(&package)?,
                    });
                    resolved_formats.push((
                        Format::CityParquet,
                        Source::Local(package),
                        id.clone(),
                    ));
                }
                // Read-only: the package a local run built under this same
                // name, uploaded beside the prepared artefacts.
                Transport::Http => resolved_formats.push((
                    Format::CityParquet,
                    Source::Http {
                        base_url: opts
                            .base_url
                            .clone()
                            .expect("run validated --base-url for --transport http"),
                        key: format!("{base}.{id}.parquet"),
                    },
                    id.clone(),
                )),
            }
        }
    }

    for (format, source, label) in &resolved_formats {
        let format = *format;
        let total = total_count_for(format, source)
            .with_context(|| format!("deriving total count for format '{format}'"))?;

        for scenario in &scenarios {
            match scenario {
                Scenario::Count | Scenario::FullRead => {
                    run_measurement(
                        &mut rows,
                        &mut samples,
                        &dataset,
                        format,
                        label,
                        source,
                        *scenario,
                        &QueryParams::default(),
                        opts.repeat,
                        None,
                        "",
                    )?;
                }
                Scenario::BBoxQuery => {
                    for window in &resolved.windows {
                        let params = QueryParams {
                            bbox: Some(window_in_artefact_order(
                                window.window,
                                format,
                                resolved.swap_xy,
                            )),
                            ..Default::default()
                        };
                        // `approx` means the target row fraction was not
                        // reachable on this data (1% of 379 rows is 3.79).
                        // The CSV says so rather than presenting a missed
                        // target as a met one.
                        let notes = if window.approx {
                            format!("{};approx", window.tag)
                        } else {
                            window.tag.clone()
                        };
                        run_measurement(
                            &mut rows,
                            &mut samples,
                            &dataset,
                            format,
                            label,
                            source,
                            *scenario,
                            &params,
                            opts.repeat,
                            Some(total),
                            &notes,
                        )?;
                    }
                }
                Scenario::AttrFilter => match &resolved.attr_filter {
                    Some(spec) => {
                        let params = QueryParams {
                            attr_column: Some(spec.column.clone()),
                            attr_pred: Some(match &spec.pred {
                                params::AttrFilterPred::Eq(value) => {
                                    AttrPred::Eq(serde_json::Value::String(value.clone()))
                                }
                                params::AttrFilterPred::Ge(bound) => AttrPred::Ge(*bound),
                            }),
                            ..Default::default()
                        };
                        let notes = spec.notes_tag();
                        run_measurement(
                            &mut rows,
                            &mut samples,
                            &dataset,
                            format,
                            label,
                            source,
                            *scenario,
                            &params,
                            opts.repeat,
                            Some(resolved.cp_object_total),
                            &notes,
                        )?;
                    }
                    None => eprintln!(
                        "cityparquet-readbench: skipping scenario '{scenario}' for format \
                         '{format}': dataset '{dataset}' has no attribute column a selective \
                         predicate can be derived from (never fabricated)"
                    ),
                },
                Scenario::AttrStats => match &resolved.numeric_attr {
                    Some(column) => {
                        let params = QueryParams {
                            attr_column: Some(column.clone()),
                            ..Default::default()
                        };
                        let notes = format!("attr={column}");
                        run_measurement(
                            &mut rows,
                            &mut samples,
                            &dataset,
                            format,
                            label,
                            source,
                            *scenario,
                            &params,
                            opts.repeat,
                            Some(resolved.cp_object_total),
                            &notes,
                        )?;
                    }
                    None => eprintln!(
                        "cityparquet-readbench: skipping scenario '{scenario}' for format \
                         '{format}': dataset '{dataset}' has no Int64/Float64 attribute column \
                         (never fabricated)"
                    ),
                },
                // Four rows: three positioned hits plus a verified miss. One
                // target would make the published time a function of where
                // that id happened to sit in the stream, which is a property
                // of the sample rather than of the format.
                Scenario::FeatureLookup if format != Format::CityParquet => {
                    eprintln!(
                        "cityparquet-readbench: skipping scenario '{scenario}' for format \
                         '{format}': {}",
                        crate::formats::FEATURE_LOOKUP_CITYPARQUET_ONLY
                    )
                }
                Scenario::FeatureLookup if resolved.feature_probes.is_empty() => eprintln!(
                    "cityparquet-readbench: skipping scenario '{scenario}' for format \
                     '{format}': dataset '{dataset}' has no prepared cityjsonseq artefact to \
                     take the feature probes from (never fabricated)"
                ),
                Scenario::FeatureLookup => {
                    for probe in &resolved.feature_probes {
                        let params = QueryParams {
                            target_feature_id: Some(probe.id.clone()),
                            ..Default::default()
                        };
                        let mut notes = probe.tag.clone();
                        if probe.substituted {
                            notes.push_str(";id-substituted");
                        }
                        run_measurement(
                            &mut rows,
                            &mut samples,
                            &dataset,
                            format,
                            label,
                            source,
                            *scenario,
                            &params,
                            opts.repeat,
                            Some(resolved.cp_object_total),
                            &notes,
                        )?;
                    }
                }
                Scenario::IdLookup if resolved.id_probes.is_empty() => eprintln!(
                    "cityparquet-readbench: skipping scenario '{scenario}' for format \
                     '{format}': dataset '{dataset}' has no prepared cityjsonseq artefact to \
                     cut the id deciles from (never fabricated)"
                ),
                Scenario::IdLookup => {
                    for probe in &resolved.id_probes {
                        let params = QueryParams {
                            target_id: Some(probe.id.clone()),
                            ..Default::default()
                        };
                        let mut notes = probe.tag.clone();
                        if probe.substituted {
                            notes.push_str(";id-substituted");
                        }
                        run_measurement(
                            &mut rows,
                            &mut samples,
                            &dataset,
                            format,
                            label,
                            source,
                            *scenario,
                            &params,
                            opts.repeat,
                            Some(resolved.cp_object_total),
                            &notes,
                        )?;
                    }
                }
            }
        }

        if opts.cold {
            eprintln!(
                "cityparquet-readbench: --cold for format '{format}': run `sudo purge` NOW to \
                 drop the OS disk/page cache before this FullRead measurement, if you have not \
                 already (this coordinator cannot invoke `sudo` itself)"
            );
            let line = spawn_child(format, Scenario::FullRead, source, &QueryParams::default())?;
            let row = Row {
                dataset: dataset.clone(),
                label: label.clone(),
                scenario: Scenario::FullRead,
                selectivity: None,
                result_count: line.result_count,
                time_s: line.time_s,
                time_std_s: 0.0,
                peak_heap_bytes: line.peak_heap_bytes,
                peak_rss_bytes: line.ru_maxrss_bytes,
                repeat: 1,
                // Exactly `cold`, and nothing else ever appended:
                // `the benchmark renderer` drops a cold row with
                // an EXACT `notes == "cold"` test, so a tag alongside it
                // would put the purged-cache measurement into the warm
                // charts. Nothing is lost by it — the only disclosures a
                // child makes today are FlatCityBuf's attribute-index
                // fallbacks (see [`ChildLine::notes`]), and this row is
                // always `FullRead`, which has no index path to fall back
                // from.
                notes: vec!["cold".to_string()],
                io: line.io,
                lookup: None,
            };
            debug_assert!(
                line.notes.is_empty(),
                "a cold FullRead row cannot carry a disclosure tag: {:?}",
                line.notes
            );
            rows.push(row);
        }
    }

    // Cross-format consistency: every count is checked against the reference
    // the parameters were derived with, or against the other formats, and a
    // disagreement fails the run — after the CSV is written, with the
    // disagreeing rows tagged, so the evidence of what went wrong survives.
    let formats_by_label: HashMap<String, Format> = resolved_formats
        .iter()
        .map(|(format, _, label)| (label.clone(), *format))
        .collect();
    let failures = check_consistency(&mut rows, &formats_by_label, &resolved);
    if failures.is_empty() {
        eprintln!("cityparquet-readbench: cross-format consistency OK");
    }

    // A configuration run groups its rows per variant, so a CSV reads top to
    // bottom the way the recipe listed the variants. `sort_by_key` is
    // stable, so the rows keep their scenario order within each group.
    if let Some(list) = &variants {
        let position = |label: &str| {
            list.iter()
                .position(|(id, _)| id == label)
                .unwrap_or(usize::MAX)
        };
        rows.sort_by_key(|r| position(&r.label));
    }

    for row in &rows {
        writeln!(csv, "{}", row.render()).context("writing a CSV row")?;
    }

    write_samples(&opts.out, &samples)?;

    if variants.is_some() && opts.transport == Transport::Local {
        let seq = variant_seq
            .as_deref()
            .expect("set for every local variants run");
        write_sizes(&opts.out, base, seq, &sizes)?;
    }

    if !failures.is_empty() {
        bail!(
            "cross-format consistency check failed for '{dataset}' ({} disagreement(s); the \
             rows are tagged `{COUNT_MISMATCH}` in {}):\n  {}",
            failures.len(),
            opts.out.display(),
            failures.join("\n  ")
        );
    }
    Ok(())
}

/// `window` (in the CityParquet package's longitude-first order) in the axis
/// order `format`'s artefact stores: swapped in `x`/`y` for a latitude-first
/// dataset read through an artefact that keeps the source's order.
fn window_in_artefact_order(window: [f64; 6], format: Format, swap_xy: bool) -> [f64; 6] {
    if swap_xy && !format.stores_longitude_first() {
        [
            window[1], window[0], window[2], window[4], window[3], window[5],
        ]
    } else {
        window
    }
}

/// Keeps only the `requested` tags of `probes`; an empty request, or a tag
/// that did not resolve, is an error naming what is available.
fn retain_requested_probes(
    flag: &str,
    probes: &mut Vec<params::IdProbe>,
    requested: Option<&Vec<String>>,
) -> Result<()> {
    let Some(requested) = requested else {
        return Ok(());
    };
    if requested.is_empty() {
        bail!("--{flag} must name at least one resolved probe tag");
    }
    let available: Vec<&str> = probes.iter().map(|probe| probe.tag.as_str()).collect();
    let unknown: Vec<&str> = requested
        .iter()
        .map(String::as_str)
        .filter(|tag| !available.contains(tag))
        .collect();
    if !unknown.is_empty() {
        bail!(
            "--{flag} requested unavailable tag(s) {}; available: {}",
            unknown.join(", "),
            available.join(", ")
        );
    }
    probes.retain(|probe| requested.iter().any(|tag| tag == &probe.tag));
    Ok(())
}

/// The variant list, parsed, de-duplicated by canonical id, and required to
/// carry the bare `cityparquet` baseline every ratio is taken against.
///
/// A `+hilbert` suffix is refused: every package this benchmark builds is
/// written in Hilbert order ([`build_variant`]), so the suffix would name
/// the same package as the id without it.
fn parse_variant_list(ids: &[String]) -> Result<Vec<(String, Variant)>> {
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::with_capacity(ids.len());
    for raw in ids {
        let variant = Variant::parse(raw).map_err(|e| anyhow::anyhow!("--variants: {e}"))?;
        if variant.ordering() == RowOrder::Hilbert {
            bail!(
                "--variants: '{raw}' carries +hilbert, but every benchmark package is written in \
                 Hilbert order already; drop the suffix"
            );
        }
        let id = variant.id();
        if seen.contains(&id) {
            bail!("--variants: duplicate variant '{id}'");
        }
        seen.push(id.clone());
        out.push((id, variant));
    }
    if !seen.iter().any(|id| id == VARIANT_BASELINE) {
        bail!(
            "--variants must include the bare '{VARIANT_BASELINE}' baseline: every ratio on a \
             configuration axis is taken against it"
        );
    }
    Ok(out)
}

/// The variant every configuration axis is measured against — the recipe
/// CityParquet ships with, spelled bare.
const VARIANT_BASELINE: &str = "cityparquet";

/// The `notes` tag a row carries when its `result_count` disagrees with the
/// reference or with the other formats. Without it a spoiled run would be
/// byte-indistinguishable from a clean one to anyone who only sees the CSV.
const COUNT_MISMATCH: &str = "count-mismatch";

/// Checks every row's `result_count` and tags the ones that disagree with
/// [`COUNT_MISMATCH`]; returns one line per disagreement, naming the
/// scenario, the formats and the counts.
///
/// Formats count at two levels (READ_BENCHMARK.md, Caveat 1):
/// [`Format::counts_features`] formats count features, the others
/// CityObjects. Where the parameters carry a reference for both levels the
/// row is checked against its own level's reference:
///
/// - `count` / `full-read`: [`params::ResolvedParams::cp_object_total`] or
///   `cp_feature_total`;
/// - each `bbox-*` window: its `objects` or `features` — the CityObjects, or
///   the features' root objects, whose bbox intersects the window;
/// - `attr-filter`: the predicate's `matched` (CityObject-level in every
///   format).
///
/// `attr-stats` and each `id-*` probe have no reference and are checked for
/// equality across every format instead. A configuration run's variants are
/// all CityParquet, so the same rules apply to them.
fn check_consistency(
    rows: &mut [Row],
    formats: &HashMap<String, Format>,
    resolved: &params::ResolvedParams,
) -> Vec<String> {
    let mut failures = Vec::new();
    let mut tagged = vec![false; rows.len()];
    let primary = |row: &Row| -> String {
        row.notes
            .first()
            .map(|n| n.split(';').next().unwrap_or("").to_string())
            .unwrap_or_default()
    };
    let is_cold = |row: &Row| row.notes.iter().any(|n| n == "cold");

    // Against a reference, per level.
    for (i, row) in rows.iter().enumerate() {
        if is_cold(row) {
            continue;
        }
        let Some(format) = formats.get(&row.label) else {
            continue;
        };
        let features = format.counts_features();
        let level = if features { "features" } else { "CityObjects" };
        let expected = match row.scenario {
            Scenario::Count | Scenario::FullRead => Some(if features {
                resolved.cp_feature_total
            } else {
                resolved.cp_object_total
            }),
            Scenario::BBoxQuery => resolved
                .windows
                .iter()
                .find(|w| w.tag == primary(row))
                .map(|w| if features { w.features } else { w.objects }),
            Scenario::AttrFilter => resolved.attr_filter.as_ref().map(|spec| spec.matched),
            _ => None,
        };
        if let Some(expected) = expected
            && row.result_count != expected
        {
            let what = match row.scenario {
                Scenario::BBoxQuery => primary(row),
                other => other.as_str().to_string(),
            };
            failures.push(format!(
                "{what}: {} reports {}, expected {expected} {level}",
                row.label, row.result_count
            ));
            tagged[i] = true;
        }
    }

    // Equality across formats where there is no reference.
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        if is_cold(row) || !matches!(row.scenario, Scenario::AttrStats | Scenario::IdLookup) {
            continue;
        }
        let key = match row.scenario {
            Scenario::IdLookup => primary(row),
            other => other.as_str().to_string(),
        };
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, members)) => members.push(i),
            None => groups.push((key, vec![i])),
        }
    }
    for (key, members) in groups {
        let first = rows[members[0]].result_count;
        if members.iter().any(|&i| rows[i].result_count != first) {
            let counts: Vec<String> = members
                .iter()
                .map(|&i| format!("{}={}", rows[i].label, rows[i].result_count))
                .collect();
            failures.push(format!("{key}: formats disagree: {}", counts.join(", ")));
            for i in members {
                tagged[i] = true;
            }
        }
    }

    for (row, tag) in rows.iter_mut().zip(tagged) {
        if tag {
            row.notes.push(COUNT_MISMATCH.to_string());
        }
    }
    failures
}

/// One requested format's artefact [`Source`], or why it has none.
enum ArtefactResolution {
    Source(Source),
    /// The artefact has no valid-UTF-8 relative key, so no HTTP URL can be
    /// built from it (only reachable under [`Transport::Http`]).
    NonUtf8Key,
}

/// Maps `format` onto its artefact [`Source`] — for [`Transport::Local`], a
/// local path under `prepared_dir`, the exact naming convention
/// `benchmark/scripts/readbench_prepare.sh` produces; for [`Transport::Http`], the
/// same artefact's relative key under `base_url` (`prepared_dir` uploaded
/// wholesale — see `benchmark/scripts/readbench_upload.md`).
///
/// The per-format NAMING itself lives on [`Format::artefact`]; this function
/// only turns the resulting name into a path or an HTTP key.
///
/// EVERY format reads a PREPARED artefact: `input` itself is never measured,
/// so this function does not touch it.
fn resolve_format_artefact(
    format: Format,
    prepared_dir: &Path,
    base: &str,
    transport: Transport,
    base_url: Option<&str>,
) -> ArtefactResolution {
    let local_path = prepared_dir.join(format.artefact(base));

    match transport {
        Transport::Local => ArtefactResolution::Source(Source::Local(local_path)),
        Transport::Http => {
            let key = local_path
                .strip_prefix(prepared_dir)
                .ok()
                .and_then(|p| p.to_str());
            match key {
                Some(key) => ArtefactResolution::Source(Source::Http {
                    base_url: base_url
                        .expect("caller (run) already validated Transport::Http requires base_url")
                        .to_string(),
                    key: key.to_string(),
                }),
                None => ArtefactResolution::NonUtf8Key,
            }
        }
    }
}

/// The cityparquet package's main table — required for QueryParams
/// derivation regardless of `--formats` (see this module's own doc
/// comment). Errors clearly, pointing at `just readbench-prepare`, rather
/// than failing deep inside a later scan.
///
/// Reads `metadata.json` and requires exactly one listed table: every
/// by-type package from a single-family dataset (e.g. delft) lists exactly
/// one, which this uses regardless of its derived name; a multi-family
/// by-type package (several family tables, no single file holding the whole
/// dataset) is rejected outright, since this coordinator's `QueryParams`
/// derivation (bbox, attribute predicate, target id — see this module's own
/// doc comment) is single-file — out of scope here rather than silently
/// deriving params from only one family's rows.
fn locate_cityparquet_table(prepared_dir: &Path, base: &str) -> Result<PathBuf> {
    let package_dir = prepared_dir.join(format!("{base}.parquet"));
    if !package_dir.is_dir() {
        bail!(
            "cannot derive QueryParams: no CityParquet package at {} — run \
             `just readbench-prepare <input>` first",
            package_dir.display()
        );
    }
    // `PackageTables::open` is the sole reader of `metadata.json` here; it
    // already rejects an empty or duplicate-naming manifest. The
    // "exactly one object table" requirement below is this coordinator's
    // own — its `QueryParams` derivation is single-file, out of scope for a
    // multi-family by-type package (see this fn's doc comment).
    let tables =
        cityparquet::stac::properties::PackageTables::open(&package_dir).with_context(|| {
            format!(
                "cannot derive QueryParams: reading {}",
                package_dir.display()
            )
        })?;
    match tables.tables.as_slice() {
        [only] => Ok(only.clone()),
        many => {
            let names: Vec<&str> = many
                .iter()
                .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
                .collect();
            bail!(
                "cannot derive QueryParams: {} has {} tables ({names:?}); only single-table \
                 (single-family) packages are supported here, not multi-table by-type packages",
                package_dir.display(),
                many.len(),
            )
        }
    }
}

/// A child's own protocol line: the LAST non-empty line of its stdout.
///
/// Every `--child` invocation prints its protocol line last, so nothing
/// before it belongs to this coordinator. Splitting the WHOLE capture on
/// whitespace instead — which is what this used to do — makes the parse
/// hostage to any library that writes to stdout behind a runner's back:
/// `fcb_core`'s indexed numeric-range `select_attr_query` prints
/// `index_start: …` / `start_position: …` / `query condition: …` on some
/// paths, which would turn a perfectly good `--attr-ge` measurement into an
/// "expected 4 or 6 fields" failure rather than a number.
fn protocol_line(stdout: &str) -> &str {
    stdout
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .unwrap_or("")
}

/// One parsed `--child` protocol stdout line. `io` is `Some` only for the 6-
/// field http-transport protocol shape (see [`spawn_child`]'s own parsing).
struct ChildLine {
    time_s: f64,
    peak_heap_bytes: u64,
    ru_maxrss_bytes: u64,
    result_count: u64,
    io: Option<IoStats>,
    /// Disclosure tags the child announced on stderr — today the FlatCityBuf
    /// runner's index fallbacks (see [`child_disclosures`]). They go into the
    /// row's `notes`, because a full scan published as an indexed query is
    /// the kind of thing a reader of the CSV must not have to have been
    /// watching the terminal to learn.
    notes: Vec<String>,
    /// The child's [`LookupCounters`], from its [`LOOKUP_STATS_MARKER`] line.
    lookup: Option<LookupCounters>,
}

/// The disclosure tags `stderr` announces, in
/// [`crate::formats::flatcitybuf::FALLBACK_MARKERS`] order, each at most
/// once.
///
/// The markers are owned by the runner that prints them, so a renamed
/// marker is a compile error here rather than a note that silently stops
/// being recorded.
fn child_disclosures(stderr: &str) -> Vec<String> {
    crate::formats::flatcitybuf::FALLBACK_MARKERS
        .iter()
        .filter(|marker| stderr.contains(**marker))
        .map(|marker| (*marker).to_string())
        .collect()
}

/// The [`LookupCounters`] a child reported after [`LOOKUP_STATS_MARKER`],
/// if it reported any.
fn child_lookup_counters(stderr: &str) -> Result<Option<LookupCounters>> {
    let Some(line) = stderr
        .lines()
        .find_map(|line| line.strip_prefix(LOOKUP_STATS_MARKER))
    else {
        return Ok(None);
    };
    let fields: Vec<u64> = line
        .split_whitespace()
        .map(|field| {
            field
                .parse::<u64>()
                .with_context(|| format!("parsing lookup counter '{field}'"))
        })
        .collect::<Result<_>>()?;
    let [row_groups_total, bloom_pruned, filter_bytes] = fields[..] else {
        bail!("expected three lookup counters after '{LOOKUP_STATS_MARKER}', got '{line}'");
    };
    Ok(Some(LookupCounters {
        row_groups_total,
        bloom_pruned,
        filter_bytes,
    }))
}

/// Spawns a FRESH `--child` process (this binary's own executable, found via
/// [`std::env::current_exe`] — never `env!("CARGO_BIN_EXE_...")`, which is
/// only set for `cargo test`/`cargo bench` targets, not a normal build) for
/// one (format, scenario) measurement, and parses its one stdout line. A
/// fresh process per call is the point (see `formats::FormatRunner`'s own
/// doc comment): independent cache state and an independent `peak_alloc`
/// high-water mark for every single sample.
fn spawn_child(
    format: Format,
    scenario: Scenario,
    source: &Source,
    params: &QueryParams,
) -> Result<ChildLine> {
    let self_exe = std::env::current_exe().context("cannot determine own executable path")?;

    let mut cmd = Command::new(&self_exe);
    cmd.arg("--child")
        .arg("--format")
        .arg(format.as_str())
        .arg("--scenario")
        .arg(scenario.as_str());
    match source {
        Source::Local(path) => {
            cmd.arg("--input").arg(path);
        }
        Source::Http { base_url, key } => {
            cmd.arg("--transport")
                .arg("http")
                .arg("--base-url")
                .arg(base_url)
                .arg("--input")
                .arg(key);
        }
    }

    if let Some(bbox) = params.bbox {
        let joined = bbox
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(",");
        cmd.arg("--bbox").arg(joined);
    }
    if let Some(column) = &params.attr_column {
        cmd.arg("--attr-column").arg(column);
    }
    match &params.attr_pred {
        Some(AttrPred::Eq(value)) => {
            let raw = match value {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            cmd.arg("--attr-eq").arg(raw);
        }
        Some(AttrPred::Ge(bound)) => {
            cmd.arg("--attr-ge").arg(bound.to_string());
        }
        Some(AttrPred::Le(bound)) => {
            cmd.arg("--attr-le").arg(bound.to_string());
        }
        Some(AttrPred::Range(lo, hi)) => {
            cmd.arg("--attr-ge").arg(lo.to_string());
            cmd.arg("--attr-le").arg(hi.to_string());
        }
        None => {}
    }
    if let Some(id) = &params.target_id {
        cmd.arg("--target-id").arg(id);
    }
    if let Some(feature_id) = &params.target_feature_id {
        cmd.arg("--target-feature-id").arg(feature_id);
    }

    let output = cmd.output().with_context(|| {
        format!("spawning child process (format={format}, scenario={scenario})")
    })?;
    if !output.status.success() {
        bail!(
            "child process failed (format={format}, scenario={scenario}); stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let notes = child_disclosures(&stderr);
    let lookup = child_lookup_counters(&stderr)?;
    let stdout = String::from_utf8(output.stdout).context("child stdout was not valid UTF-8")?;
    let line = protocol_line(&stdout);
    let fields: Vec<&str> = line.split_whitespace().collect();
    // 4 fields: local transport (unchanged since Task 8). 6 fields: http
    // transport, `bytes_read`/`http_requests` appended (see
    // `main.rs`'s own child-protocol doc comment).
    let io = match fields.len() {
        4 => None,
        6 => Some(IoStats {
            bytes: fields[4]
                .parse()
                .with_context(|| format!("parsing bytes_read from '{}'", fields[4]))?,
            requests: fields[5]
                .parse()
                .with_context(|| format!("parsing http_requests from '{}'", fields[5]))?,
        }),
        n => bail!(
            "expected 4 or 6 whitespace-separated fields from the child protocol, got {n} in \
             '{line}' (format={format}, scenario={scenario})"
        ),
    };
    Ok(ChildLine {
        time_s: fields[0]
            .parse()
            .with_context(|| format!("parsing time_s from '{}'", fields[0]))?,
        peak_heap_bytes: fields[1]
            .parse()
            .with_context(|| format!("parsing peak_heap_bytes from '{}'", fields[1]))?,
        ru_maxrss_bytes: fields[2]
            .parse()
            .with_context(|| format!("parsing ru_maxrss_bytes from '{}'", fields[2]))?,
        result_count: fields[3]
            .parse()
            .with_context(|| format!("parsing result_count from '{}'", fields[3]))?,
        io,
        notes,
        lookup,
    })
}

/// A single `Count`-scenario child call (untimed — its own `time_s` is
/// discarded), used to establish `format`'s own total object/feature count.
/// Each format counts at its own natural grain (see
/// `formats::cityparquet`/`formats::cityjsonseq`/`formats::flatcitybuf`'s own
/// module docs on CityObject-vs-feature granularity), so this per-format
/// total is the correct SELECTIVITY denominator only for [`Scenario::BBoxQuery`]
/// (feature-level numerator over a feature-level denominator, for every
/// format). For the CityObject-level scenarios (`AttrFilter`/`AttrStats`/
/// `IdLookup`), [`run`] instead uses the dataset-global CityObject
/// total — this same function called once against the `cityparquet` package
/// — as a SHARED denominator across every format, so those scenarios'
/// selectivity is directly comparable and always in `(0, 1]` (see this
/// module's own doc comment).
fn total_count_for(format: Format, source: &Source) -> Result<u64> {
    let line = spawn_child(format, Scenario::Count, source, &QueryParams::default())?;
    Ok(line.result_count)
}

/// Runs one (format, scenario, params) measurement: `repeat + 1` fresh child
/// processes (the first discarded as a warmup), then the MEAN `time_s` (+ the
/// population standard deviation), the MAX `peak_heap_bytes`/`ru_maxrss_bytes`
/// across the `repeat` warm
/// samples, and `result_count` from the first warm sample (every warm sample
/// measures the identical scenario against the identical unmodified input,
/// so they always agree on `result_count`; only the timing/memory varies).
/// Buffers one CSV row (see [`run`] on why rows are held to the end) and
/// returns the `result_count` for callers that need it (the self-consistency
/// check in [`run`]).
#[allow(clippy::too_many_arguments)]
fn run_measurement(
    rows: &mut Vec<Row>,
    samples: &mut Vec<Sample>,
    dataset: &str,
    format: Format,
    label: &str,
    source: &Source,
    scenario: Scenario,
    params: &QueryParams,
    repeat: usize,
    total_for_selectivity: Option<u64>,
    notes: &str,
) -> Result<u64> {
    let mut times = Vec::with_capacity(repeat);
    let mut peak_heap_max = 0u64;
    let mut peak_rss_max = 0u64;
    let mut result_count: Option<u64> = None;
    // The first warm sample's io, same convention as `result_count`: every
    // warm sample measures the identical scenario against the identical
    // unmodified remote object, so bytes/requests are deterministic across
    // repeats (unlike timing) — no need to aggregate beyond "the first one".
    let mut io: Option<IoStats> = None;
    let mut lookup: Option<LookupCounters> = None;
    // Likewise the first warm sample's own disclosures (see
    // [`ChildLine::notes`]): which mechanism a runner took is a property of
    // the artefact, identical in every repeat.
    let mut child_notes: Vec<String> = Vec::new();

    for i in 0..=repeat {
        let line = spawn_child(format, scenario, source, params)?;
        samples.push(Sample {
            dataset: dataset.to_string(),
            format: label.to_string(),
            scenario: scenario.to_string(),
            query_tag: notes.to_string(),
            sample_index: i,
            warmup: i == 0,
            time_s: line.time_s,
            peak_rss_bytes: line.ru_maxrss_bytes,
            peak_heap_bytes: line.peak_heap_bytes,
            result_count: line.result_count,
        });
        if i == 0 {
            // Warmup: discarded entirely (never contributes to the mean,
            // the MAX peak metrics, or `result_count`).
            continue;
        }
        times.push(line.time_s);
        peak_heap_max = peak_heap_max.max(line.peak_heap_bytes);
        peak_rss_max = peak_rss_max.max(line.ru_maxrss_bytes);
        if result_count.is_none() {
            result_count = Some(line.result_count);
            io = line.io;
            lookup = line.lookup;
            child_notes = line.notes;
        }
    }

    let result_count = result_count.expect("repeat >= 1 guarantees at least one warm sample");
    let time_s = mean(&times);
    let time_std_s = std_dev(&times, time_s);

    let selectivity = match scenario {
        Scenario::Count | Scenario::FullRead => None,
        _ => total_for_selectivity
            .and_then(|total| (total > 0).then_some(result_count as f64 / total as f64)),
    };

    let mut tags: Vec<String> = Vec::new();
    if !notes.is_empty() {
        tags.push(notes.to_string());
    }
    tags.extend(child_notes);

    rows.push(Row {
        dataset: dataset.to_string(),
        label: label.to_string(),
        scenario,
        selectivity,
        result_count,
        time_s,
        time_std_s,
        peak_heap_bytes: peak_heap_max,
        peak_rss_bytes: peak_rss_max,
        repeat,
        notes: tags,
        io,
        lookup,
    });

    Ok(result_count)
}

/// Builds one variant's package: the prepared CityJSONSeq converted with the
/// variant's recipe into a scratch directory INSIDE `prepared_dir` (a rename
/// across filesystems would fail, and a 1M-object package does not belong in
/// /tmp), then moved to `<prepared_dir>/<base>.<id>.parquet`; returns that
/// path. Untimed: the configuration run measures the reads of the package
/// and its size, not how long it took to build.
///
/// `ConvertOptions` is filled the way the prepare script's `cityparquet
/// convert --ordering hilbert` fills it (Hilbert row order,
/// `generate_lod0: true`, the default batch size), so a variant package has
/// the same content as `<base>.parquet` and differs from it only in the
/// recipe under test. A library-default `ConvertOptions::new`
/// would leave LoD0 generation OFF and the row counts would not line up.
fn build_variant(
    base: &str,
    id: &str,
    variant: &Variant,
    prepared_dir: &Path,
    seq: &Path,
) -> Result<PathBuf> {
    let scratch = tempfile::Builder::new()
        .prefix(&format!(".{base}.{id}.build."))
        .tempdir_in(prepared_dir)
        .with_context(|| format!("creating a scratch directory in {}", prepared_dir.display()))?;
    let built = scratch.path().join("pkg");
    let mut opts = cityparquet::package::ConvertOptions::new(seq.to_path_buf(), built.clone());
    opts.recipe = variant.recipe();
    opts.ordering = RowOrder::Hilbert;
    opts.generate_lod0 = true;
    cityparquet::package::convert(&opts)
        .with_context(|| format!("converting with variant '{id}'"))?;

    let target = prepared_dir.join(format!("{base}.{id}.parquet"));
    if target.exists() {
        fs::remove_dir_all(&target)
            .with_context(|| format!("removing the previous {}", target.display()))?;
    }
    fs::rename(&built, &target)
        .with_context(|| format!("moving the built package to {}", target.display()))?;
    Ok(target)
}

/// The arithmetic mean of `values` (must be non-empty).
///
/// `time_s` is the mean in both this harness and `benchmark/databases`, so a
/// timing quoted from either CSV is the same statistic.
fn mean(values: &[f64]) -> f64 {
    debug_assert!(!values.is_empty(), "mean needs at least one sample");
    values.iter().sum::<f64>() / values.len() as f64
}

/// The population standard deviation of `values` about `centre` (must be
/// non-empty). Population, not sample: the warm repeats are the whole
/// measured set, not a draw used to infer a wider one. This is the
/// dispersion column beside the mean `time_s`.
fn std_dev(values: &[f64], centre: f64) -> f64 {
    debug_assert!(!values.is_empty(), "std_dev needs at least one sample");
    let variance = values
        .iter()
        .map(|v| {
            let d = v - centre;
            d * d
        })
        .sum::<f64>()
        / values.len() as f64;
    variance.sqrt()
}

/// One results-CSV row, held until the whole matrix has run.
///
/// Everything but `notes` is fixed the moment its measurement finishes;
/// `notes` is a LIST of tags because a row can carry more than one — the
/// scenario's own tag (`bbox-5pct`, `id=…`), a disclosure the child made
/// about which mechanism it actually took (see [`ChildLine::notes`]), and a
/// run-level one only known once every format has run (see
/// [`ATTR_FILTER_MISMATCH`]).
struct Row {
    dataset: String,
    /// What the `format` column prints: `format.as_str()` on a format run,
    /// the variant id on a `--variants` run.
    label: String,
    scenario: Scenario,
    selectivity: Option<f64>,
    result_count: u64,
    time_s: f64,
    time_std_s: f64,
    peak_heap_bytes: u64,
    peak_rss_bytes: u64,
    repeat: usize,
    notes: Vec<String>,
    io: Option<IoStats>,
    lookup: Option<LookupCounters>,
}

impl Row {
    /// This row in [`CSV_HEADER`]'s exact column order.
    ///
    /// Tags are joined with `;`, never `,`: `notes` is one CSV field, and a
    /// comma inside it would silently shift every column after it.
    fn render(&self) -> String {
        let selectivity_field = match self.selectivity {
            Some(value) => format!("{value:.6}"),
            None => String::new(),
        };
        let (bytes_field, requests_field) = match self.io {
            Some(io) => (io.bytes.to_string(), io.requests.to_string()),
            None => (String::new(), String::new()),
        };
        let lookup_fields = match self.lookup {
            Some(l) => format!(
                "{},{},{}",
                l.row_groups_total, l.bloom_pruned, l.filter_bytes
            ),
            None => ",,".to_string(),
        };
        let notes = self.notes.join(";");
        let (dataset, format, scenario, result_count, time_s, time_std_s) = (
            &self.dataset,
            &self.label,
            self.scenario.as_str(),
            self.result_count,
            self.time_s,
            self.time_std_s,
        );
        let (peak_heap_bytes, peak_rss_bytes, repeat) =
            (self.peak_heap_bytes, self.peak_rss_bytes, self.repeat);
        format!(
            "{dataset},{format},{scenario},{selectivity_field},{result_count},{time_s:.6},\
             {time_std_s:.6},{peak_heap_bytes},{peak_rss_bytes},{repeat},{notes},\
             {bytes_field},{requests_field},{lookup_fields}"
        )
    }
}

/// One `sizes.csv` row: what one variant's package weighs.
struct SizeRow {
    dataset: String,
    label: String,
    bytes: u64,
}

/// Bytes of every regular file directly inside a package directory — the
/// rule `cityparquet bench` uses for `total_bytes`.
fn dir_bytes(dir: &Path) -> Result<u64> {
    let mut total = 0u64;
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let meta = entry?.metadata()?;
        if meta.is_file() {
            total += meta.len();
        }
    }
    Ok(total)
}

/// The size table's own header, in the read run's columns.
const SIZES_HEADER: &str =
    "dataset,format,bytes,mb,ratio_vs_cityjsonseq,baseline_format,ratio_vs_baseline";

/// `sizes.csv` beside `out`, in the read run's columns. Rows for THIS dataset
/// are replaced; other datasets' rows are kept, so one recipe walking many
/// slices builds up one file and a re-run of one slice never duplicates.
fn write_sizes(out: &Path, base: &str, seq: &Path, sizes: &[SizeRow]) -> Result<()> {
    let path = out
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("sizes.csv");
    let mut kept: Vec<String> = Vec::new();
    if path.exists() {
        let existing =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let mut lines = existing.lines();
        match lines.next() {
            Some(header) if header == SIZES_HEADER => {}
            Some(header) => bail!(
                "{} has a foreign header; expected `{SIZES_HEADER}`, found `{header}`",
                path.display()
            ),
            None => {}
        }
        kept.extend(
            lines
                .filter(|l| l.split(',').next() != Some(base))
                .map(str::to_string),
        );
    }
    let seq_bytes = fs::metadata(seq)
        .with_context(|| format!("stat {}", seq.display()))?
        .len() as f64;
    let baseline_bytes = sizes
        .iter()
        .find(|s| s.label == VARIANT_BASELINE)
        .map(|s| s.bytes as f64)
        .expect("parse_variant_list guarantees the baseline");
    for s in sizes {
        let bytes = s.bytes as f64;
        kept.push(format!(
            "{},{},{},{:.6},{:.6},{VARIANT_BASELINE},{:.6}",
            s.dataset,
            s.label,
            s.bytes,
            bytes / (1024.0 * 1024.0),
            seq_bytes / bytes,
            baseline_bytes / bytes,
        ));
    }
    let mut text = String::from(SIZES_HEADER);
    text.push('\n');
    for line in kept {
        text.push_str(&line);
        text.push('\n');
    }
    fs::write(&path, text).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(scenario: Scenario, notes: &[&str]) -> Row {
        Row {
            dataset: "delft.city.jsonl".to_string(),
            label: Format::FlatCityBuf.as_str().to_string(),
            scenario,
            selectivity: None,
            result_count: 1116,
            time_s: 0.5,
            time_std_s: 0.0,
            peak_heap_bytes: 1,
            peak_rss_bytes: 2,
            repeat: 1,
            notes: notes.iter().map(|n| (*n).to_string()).collect(),
            io: None,
            lookup: None,
        }
    }

    /// A library that prints to stdout behind a runner's back must not be
    /// able to break the child protocol. `fcb_core`'s indexed numeric-range
    /// query does exactly that, so an `--attr-ge` measurement would
    /// otherwise fail to parse rather than return a number.
    #[test]
    fn the_protocol_line_is_the_last_non_empty_stdout_line() {
        assert_eq!(
            protocol_line("0.001 10 20 30\n"),
            "0.001 10 20 30",
            "a clean child is unchanged"
        );
        assert_eq!(
            protocol_line(
                "index_start: 1234\nstart_position: 8\nquery condition: TerrainHeight Ge \
                 2.45\n0.000467 19132 5255168 217\n"
            ),
            "0.000467 19132 5255168 217",
            "library chatter before the protocol line must be ignored"
        );
        assert_eq!(
            protocol_line("0.001 10 20 30\n\n  \n"),
            "0.001 10 20 30",
            "trailing blank lines are not the protocol line"
        );
        assert_eq!(
            protocol_line(""),
            "",
            "an empty capture yields an empty line"
        );
    }

    /// The `notes` column is one CSV field, so several tags on one row must
    /// never introduce a comma — that would shift `bytes_read` and
    /// `http_requests` one column left for that row alone.
    #[test]
    fn several_notes_tags_stay_inside_one_csv_field() {
        let rendered = row(
            Scenario::AttrFilter,
            &["attr=b3_dak_type=slanted", "no-attr-index"],
        )
        .render();
        assert_eq!(
            rendered.split(',').count(),
            CSV_HEADER.split(',').count(),
            "a multi-tag row must still have exactly {} fields: {rendered}",
            CSV_HEADER.split(',').count()
        );
        assert!(
            rendered.contains("attr=b3_dak_type=slanted;no-attr-index"),
            "tags must be joined with ';': {rendered}"
        );
    }

    fn labelled(label: &str, scenario: Scenario, notes: &[&str], count: u64) -> Row {
        let mut r = row(scenario, notes);
        r.label = label.to_string();
        r.result_count = count;
        r
    }

    /// Tokyo's shape: 49,915 CityObjects in 38,743 features, and one window
    /// selecting 499 objects in 262 features.
    fn tokyo_like() -> params::ResolvedParams {
        params::ResolvedParams {
            dataset: "tokyo.city.json".to_string(),
            windows: vec![params::BboxWindow {
                tag: "bbox-1pct".to_string(),
                target: 0.01,
                achieved: 0.01,
                window: [0.0; 6],
                approx: false,
                objects: 499,
                features: 262,
            }],
            id_probes: Vec::new(),
            attr_filter: None,
            feature_probes: Vec::new(),
            numeric_attr: None,
            cp_object_total: 49_915,
            cp_feature_total: 38_743,
            swap_xy: true,
        }
    }

    fn formats() -> HashMap<String, Format> {
        [Format::CityJson, Format::CityJsonSeq, Format::CityParquet]
            .into_iter()
            .map(|f| (f.as_str().to_string(), f))
            .collect()
    }

    #[test]
    fn counts_that_agree_at_their_own_level_pass() {
        let mut rows = vec![
            labelled("cityjson", Scenario::BBoxQuery, &["bbox-1pct"], 499),
            labelled(
                "cityjsonseq",
                Scenario::BBoxQuery,
                &["bbox-1pct;approx"],
                262,
            ),
            labelled("cityparquet", Scenario::BBoxQuery, &["bbox-1pct"], 499),
            labelled("cityjson", Scenario::Count, &[], 49_915),
            labelled("cityjsonseq", Scenario::Count, &[], 38_743),
            labelled("cityjson", Scenario::IdLookup, &["id-50pct"], 1),
            labelled("cityjsonseq", Scenario::IdLookup, &["id-50pct"], 1),
        ];
        assert!(check_consistency(&mut rows, &formats(), &tokyo_like()).is_empty());
        assert!(rows.iter().all(|r| !r.render().contains(COUNT_MISMATCH)));
    }

    /// The defect this check exists for: every source-order format returned
    /// 0 on a latitude-first dataset while CityParquet returned 499, and the
    /// run still reported itself consistent.
    #[test]
    fn the_tokyo_zeros_fail_and_name_the_scenario_format_and_counts() {
        let mut rows = vec![
            labelled("cityjson", Scenario::BBoxQuery, &["bbox-1pct"], 0),
            labelled("cityjsonseq", Scenario::BBoxQuery, &["bbox-1pct"], 0),
            labelled("cityparquet", Scenario::BBoxQuery, &["bbox-1pct"], 499),
        ];
        let failures = check_consistency(&mut rows, &formats(), &tokyo_like());
        assert_eq!(
            failures,
            vec![
                "bbox-1pct: cityjson reports 0, expected 499 CityObjects".to_string(),
                "bbox-1pct: cityjsonseq reports 0, expected 262 features".to_string(),
            ]
        );
        assert!(rows[0].render().contains(COUNT_MISMATCH));
        assert!(!rows[2].render().contains(COUNT_MISMATCH));
    }

    #[test]
    fn a_seeded_disagreement_without_a_reference_fails_too() {
        let mut rows = vec![
            labelled(
                "cityjson",
                Scenario::AttrStats,
                &["attr=measuredHeight"],
                38_743,
            ),
            labelled(
                "cityjsonseq",
                Scenario::AttrStats,
                &["attr=measuredHeight"],
                38_742,
            ),
            labelled("cityjson", Scenario::IdLookup, &["id-miss"], 0),
            labelled("cityjsonseq", Scenario::IdLookup, &["id-miss"], 0),
        ];
        let failures = check_consistency(&mut rows, &formats(), &tokyo_like());
        assert_eq!(
            failures,
            vec!["attr-stats: formats disagree: cityjson=38743, cityjsonseq=38742".to_string()]
        );
        assert!(
            rows[0].render().contains(COUNT_MISMATCH) && rows[1].render().contains(COUNT_MISMATCH)
        );
        assert!(!rows[2].render().contains(COUNT_MISMATCH));
    }

    /// A `cold` row is excluded from the charts by an EXACT `notes == "cold"`
    /// test in `the benchmark renderer`, so its `notes` field must
    /// render as exactly that — a purged-cache measurement plotted among the
    /// warm ones would be read as a warm number.
    #[test]
    fn a_cold_rows_notes_field_is_exactly_cold() {
        let rendered = row(Scenario::FullRead, &["cold"]).render();
        let notes = rendered.split(',').nth(10).unwrap();
        assert_eq!(notes, "cold");
    }

    /// Each FlatCityBuf index fallback the child announces becomes exactly
    /// one `notes` tag; a child that took the indexed path announces none.
    #[test]
    fn a_flatcitybuf_index_fallback_becomes_a_notes_tag() {
        assert_eq!(
            child_disclosures(
                "cityparquet-readbench: flatcitybuf: attribute 'object_type' has no B+-tree \
                 index (no-attr-index); falling back to a full scan\n"
            ),
            vec!["no-attr-index".to_string()]
        );
        assert_eq!(
            child_disclosures(
                "cityparquet-readbench: flatcitybuf: indexed attr-filter query on 'x' failed \
                 (boom) (attr-index-failed); falling back to a full scan\n"
            ),
            vec!["attr-index-failed".to_string()]
        );
        assert!(child_disclosures("").is_empty());
    }

    #[test]
    fn lookup_counters_fill_the_three_trailing_columns_and_are_empty_otherwise() {
        let plain = row(Scenario::IdLookup, &["id-miss"]);
        let rendered = plain.render();
        assert!(rendered.ends_with("id-miss,,,,,"), "{rendered}");
        assert_eq!(rendered.split(',').count(), CSV_HEADER.split(',').count());

        let mut counted = row(Scenario::IdLookup, &["id-miss"]);
        counted.lookup = Some(LookupCounters {
            row_groups_total: 16,
            bloom_pruned: 15,
            filter_bytes: 4096,
        });
        let rendered = counted.render();
        assert!(rendered.ends_with("id-miss,,,16,15,4096"), "{rendered}");
        assert_eq!(rendered.split(',').count(), CSV_HEADER.split(',').count());
    }

    #[test]
    fn a_childs_lookup_counters_are_read_from_its_marker_line() {
        let stderr = format!("some log\n{LOOKUP_STATS_MARKER} 16 15 4096\n");
        assert_eq!(
            child_lookup_counters(&stderr).unwrap(),
            Some(LookupCounters {
                row_groups_total: 16,
                bloom_pruned: 15,
                filter_bytes: 4096,
            })
        );
        assert_eq!(child_lookup_counters("some log\n").unwrap(), None);
        assert!(child_lookup_counters(&format!("{LOOKUP_STATS_MARKER} 1 2\n")).is_err());
        assert!(child_lookup_counters(&format!("{LOOKUP_STATS_MARKER} 1 2 3 4\n")).is_err());
    }
}
