//! The read-benchmark COORDINATOR: `cityparquet-readbench run ...`.
//!
//! Everything Tasks 8-10 built is a `--child` process that runs exactly one
//! (format, scenario) measurement and prints one line to stdout. This module
//! is the piece that actually drives a whole benchmark matrix: for every
//! requested (format, scenario) pair it derives real [`QueryParams`] from the
//! data itself (never a hardcoded id/attribute/bbox), spawns the `--child`
//! process `repeat` times (plus one discarded warmup) via
//! [`std::env::current_exe`], medians the timings (+ MAD), takes the MAX
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
//! by `benchmark/scripts/readbench_duckdb.sh` rather than re-derived there.
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
//!
//! **`--variants`: the configuration run.** Given variant ids rather than
//! formats, this module measures ONE format's writer axes instead of
//! comparing formats: per variant it spawns `--child --write` (a warmup plus
//! `write_repeat` warm repeats, each converting the prepared CityJSONSeq
//! artefact into a scratch directory inside `prepared_dir`), keeps the last
//! repeat's package as `<prepared_dir>/<base>.<id>.parquet`, and then runs
//! the ordinary read children against it. The CSV keeps its shape — the
//! variant id goes in the `format` column, and the conversion is a `write`
//! row — and the packages' sizes go to `sizes.csv` beside it (see
//! [`write_sizes`]). Every write runs before any read, so the two kinds of
//! load never interleave; the rows are sorted back into per-variant groups
//! before they are written.
//!
//! **Self-consistency (disclosed, never a hard failure).** After the
//! `AttrFilter` scenario has run for every resolved format, this module
//! compares their `result_count`s: `object_type` equality is CityObject-level
//! for every format (CityParquet's own row grain; CityJSONSeq/FlatCityBuf
//! deliberately flatten to the same grain for this scenario — see their own
//! module docs), so a healthy run should see them agree exactly. A mismatch
//! is not a fatal error — this is a diagnostic, not a correctness gate on
//! the coordinator itself — but it is not stderr-only either: it is recorded
//! in the `notes` column of every `AttrFilter` row (see
//! [`ATTR_FILTER_MISMATCH`]), because the CSV is what gets published, and a
//! spoiled run must not be byte-indistinguishable from a clean one. The same
//! applies to a runner that fell back from an index to a full scan (see
//! [`ChildLine::notes`]).

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use cityparquet::variant::Variant;
use cityparquet_readbench::format::{Artefact, Format};
use cityparquet_readbench::naming::strip_known_extension;

use crate::formats::{IoStats, Source};
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
    /// into (default `benchmark/formats/data/readbench`).
    pub prepared_dir: PathBuf,
    /// Result CSV path; this run OWNS the file (fresh truncate + write).
    pub out: PathBuf,
    /// Warm repeats per measurement (a further, discarded warmup precedes
    /// every one). Must be >= 1.
    pub repeat: usize,
    /// Requested formats; `None`/empty selects [`Format::DEFAULT_SET`], the
    /// format-comparison set.
    pub formats: Option<Vec<Format>>,
    /// Requested variant ids (`cityparquet::variant`'s grammar). When set,
    /// this is a CONFIGURATION run rather than a format comparison: every id
    /// is converted by a write child into
    /// `<prepared_dir>/<base>.<id>.parquet` and then read by the CityParquet
    /// runner. Exclusive with `formats`; local transport only.
    pub variants: Option<Vec<String>>,
    /// Warm write repeats per variant (a discarded warmup precedes them);
    /// read only on a `variants` run, where it must be >= 1.
    pub write_repeat: usize,
    /// Requested scenario names (canonical [`Scenario::as_str`] spelling,
    /// case-insensitive); `None`/empty selects every [`Scenario::ALL`].
    pub scenarios: Option<Vec<String>>,
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
const CSV_HEADER: &str = "dataset,format,scenario,selectivity,result_count,time_s,time_mad_s,\
peak_heap_bytes,peak_rss_bytes,repeat,notes,bytes_read,http_requests";

/// The resolved-parameters sidecar for a results CSV: the CSV's own path with
/// `.params.json` appended, so the two travel together and a run cannot leave
/// a stale sidecar behind for a different CSV.
pub fn params_sidecar_path(out: &Path) -> PathBuf {
    let mut name = out.as_os_str().to_os_string();
    name.push(".params.json");
    PathBuf::from(name)
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
    if variants.is_some() && opts.transport == Transport::Http {
        bail!("--variants runs on --transport local only: there is no server-side write");
    }
    if variants.is_some() && opts.write_repeat == 0 {
        bail!("--write-repeat must be >= 1");
    }

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
    // packages its own write children have yet to produce, so it is filled
    // in after the CSV header is written, below.
    let mut resolved_formats: Vec<(Format, Source, String)> = Vec::new();
    if variants.is_none() {
        // Who chose the format list matters to how a skip below is reported: an
        // operator who NAMED a format is told about it once, in passing; a
        // default-set run that quietly measured a subset would otherwise be
        // published as "the format comparison" (see the summary after this loop).
        let (requested_formats, chosen_by_default): (Vec<Format>, bool) = match &opts.formats {
            Some(v) if !v.is_empty() => (v.clone(), false),
            _ => (Format::DEFAULT_SET.to_vec(), true),
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
                ArtefactResolution::NotCoordinated => {
                    skipped_formats.push(format);
                    eprintln!(
                        "cityparquet-readbench: skipping format '{format}': driven by \
                         benchmark/scripts/readbench_duckdb.sh, not this coordinator"
                    )
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
    let seq_path = match Format::CityJsonSeq.artefact(base) {
        Artefact::Prepared(name) => {
            let path = opts.prepared_dir.join(name);
            path.exists().then_some(path)
        }
        Artefact::NotCoordinated => None,
    };
    let gml_path = match Format::CityGml.artefact(base) {
        Artefact::Prepared(name) => {
            let path = opts.prepared_dir.join(name);
            path.exists().then_some(path)
        }
        Artefact::NotCoordinated => None,
    };
    let resolved = params::resolve(
        &dataset,
        &cp_table,
        seq_path.as_deref(),
        gml_path.as_deref(),
    )?;

    eprintln!(
        "cityparquet-readbench: derived params for '{dataset}': windows={:?}, object_type \
         most-frequent='{}' (n={}), numeric attribute={:?}, id probes={:?}, CityObject \
         total={}",
        resolved
            .windows
            .iter()
            .map(|w| (w.tag.as_str(), w.achieved, w.approx))
            .collect::<Vec<_>>(),
        resolved.object_type,
        resolved.object_type_count,
        resolved.numeric_attr,
        resolved
            .id_probes
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
    // description of which windows, ids and attributes this run measured —
    // `benchmark/scripts/readbench_duckdb.sh` reads it rather than
    // re-deriving the same choices in bash, so the two cannot drift.
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

    // A configuration run's own sources: one write child per variant, timed
    // into `rows` and measured into `sizes`, each leaving the package the
    // read children below then run against. It happens here, after the
    // sidecar and the header, so a run that dies in a write still leaves a
    // clean, empty CSV and the parameters it was going to measure with.
    //
    // Order: every write first, then every read. Writing and reading one
    // variant at a time would interleave two different kinds of load on the
    // page cache; the CSV is sorted back into per-variant groups at the end
    // (see the sort before the rows are written).
    let mut sizes: Vec<SizeRow> = Vec::new();
    let variant_seq: Option<PathBuf> = match &variants {
        None => None,
        Some(_) => Some(seq_path.clone().ok_or_else(|| {
            anyhow::anyhow!(
                "--variants needs the prepared CityJSONSeq artefact {}.city.jsonl to convert \
                 from (run `just readbench-prepare {}` first)",
                base,
                opts.input.display()
            )
        })?),
    };
    if let Some(list) = &variants {
        let seq = variant_seq
            .as_deref()
            .expect("set together with `variants`");
        for (id, _) in list {
            let package = run_write(
                &mut rows,
                &dataset,
                base,
                id,
                &opts.prepared_dir,
                seq,
                opts.write_repeat,
            )?;
            sizes.push(SizeRow {
                dataset: base.to_string(),
                label: id.clone(),
                bytes: dir_bytes(&package)?,
            });
            resolved_formats.push((Format::CityParquet, Source::Local(package), id.clone()));
        }
    }

    // AttrFilter's result_count per measured label, collected for the
    // self-consistency check below. Keyed by the LABEL rather than the
    // format so a configuration run compares its variants against each other
    // — on a format run every label is that format's own name, so nothing
    // changes there.
    let mut attr_filter_counts: HashMap<String, u64> = HashMap::new();

    for (format, source, label) in &resolved_formats {
        let format = *format;
        let total = total_count_for(format, source)
            .with_context(|| format!("deriving total count for format '{format}'"))?;

        for scenario in &scenarios {
            match scenario {
                Scenario::Count | Scenario::FullRead => {
                    run_measurement(
                        &mut rows,
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
                            bbox: Some(window.window),
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
                Scenario::AttrFilter => {
                    let params = QueryParams {
                        attr_column: Some("object_type".to_string()),
                        attr_pred: Some(AttrPred::Eq(serde_json::Value::String(
                            resolved.object_type.clone(),
                        ))),
                        ..Default::default()
                    };
                    let notes = format!("object_type={}", resolved.object_type);
                    let count = run_measurement(
                        &mut rows,
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
                    attr_filter_counts.insert(label.clone(), count);
                }
                Scenario::AttrStats | Scenario::Project => match &resolved.numeric_attr {
                    Some(column) => {
                        let params = QueryParams {
                            attr_column: Some(column.clone()),
                            ..Default::default()
                        };
                        let notes = format!("attr={column}");
                        run_measurement(
                            &mut rows,
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
                measure: Measure::Read(Scenario::FullRead),
                selectivity: None,
                result_count: line.result_count,
                time_s: line.time_s,
                time_mad_s: 0.0,
                peak_heap_bytes: line.peak_heap_bytes,
                peak_rss_bytes: line.ru_maxrss_bytes,
                repeat: 1,
                // Exactly `cold`, and nothing else ever appended:
                // `benchmark/plot/readbench_plot/plot.py` drops a cold row with
                // an EXACT `notes == "cold"` test, so a tag alongside it
                // would put the purged-cache measurement into the warm
                // charts. Nothing is lost by it — the only disclosures a
                // child makes today are FlatCityBuf's attribute-index
                // fallbacks (see [`ChildLine::notes`]), and this row is
                // always `FullRead`, which has no index path to fall back
                // from.
                notes: vec!["cold".to_string()],
                io: line.io,
            };
            debug_assert!(
                line.notes.is_empty(),
                "a cold FullRead row cannot carry a disclosure tag: {:?}",
                line.notes
            );
            rows.push(row);
        }
    }

    // Self-consistency check: never fatal, but never invisible either (see
    // this module's own doc comment).
    if attr_filter_counts.len() > 1 {
        let mut values = attr_filter_counts.values();
        let first = *values.next().expect("len > 1 implies at least one value");
        if values.all(|v| *v == first) {
            eprintln!(
                "cityparquet-readbench: self-consistency OK: every resolved format's \
                 AttrFilter(object_type) result_count == {first}"
            );
        } else {
            eprintln!(
                "cityparquet-readbench: WARNING: formats disagree on \
                 AttrFilter(object_type) result_count: {attr_filter_counts:?}"
            );
            tag_attr_filter_mismatch(&mut rows);
        }
    }

    // A configuration run groups its rows per variant, write first, so a CSV
    // reads top to bottom the way the recipe listed the variants (the writes
    // all happened before the reads; see the variants block above).
    // `sort_by_key` is stable, so the read rows keep their scenario order
    // within each group.
    if let Some(list) = &variants {
        let position = |label: &str| {
            list.iter()
                .position(|(id, _)| id == label)
                .unwrap_or(usize::MAX)
        };
        rows.sort_by_key(|r| (position(&r.label), r.measure != Measure::Write));
    }

    for row in &rows {
        writeln!(csv, "{}", row.render()).context("writing a CSV row")?;
    }

    if variants.is_some() {
        let seq = variant_seq
            .as_deref()
            .expect("set together with `variants`");
        write_sizes(&opts.out, base, seq, &sizes)?;
    }

    Ok(())
}

/// The variant list, parsed, de-duplicated by canonical id, and required to
/// carry the bare `cityparquet` baseline every ratio is taken against.
fn parse_variant_list(ids: &[String]) -> Result<Vec<(String, Variant)>> {
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::with_capacity(ids.len());
    for raw in ids {
        let variant = Variant::parse(raw).map_err(|e| anyhow::anyhow!("--variants: {e}"))?;
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

/// The `notes` tag a spoiled cross-format comparison carries into the CSV.
///
/// The stderr WARNING above is for whoever watched the run; this is for
/// everyone who only ever sees the artefact. Without it a run whose formats
/// disagreed on `AttrFilter(object_type)` — i.e. one whose object-level rows
/// are not measuring the same query — is byte-indistinguishable from a clean
/// one.
const ATTR_FILTER_MISMATCH: &str = "attr-filter-count-mismatch";

/// Records [`ATTR_FILTER_MISMATCH`] on every [`Scenario::AttrFilter`] row —
/// the rows whose `result_count`s are the ones that disagreed.
fn tag_attr_filter_mismatch(rows: &mut [Row]) {
    for row in rows
        .iter_mut()
        .filter(|r| r.measure == Measure::Read(Scenario::AttrFilter))
    {
        row.notes.push(ATTR_FILTER_MISMATCH.to_string());
    }
}

/// One requested format's artefact [`Source`], or how it is out of this
/// coordinator's scope.
enum ArtefactResolution {
    Source(Source),
    /// [`Artefact::NotCoordinated`] — `duckdb-parquet`, a separate
    /// SQL-engine baseline (Task 12).
    NotCoordinated,
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
/// only turns the resulting [`Artefact`] into a path or an HTTP key.
///
/// EVERY format reads a PREPARED artefact: `input` itself is never measured,
/// so this function does not touch it. (`Format::CityJsonSeq` used to read
/// it, which was correct only while every `--input` was a `.city.jsonl` —
/// see [`Artefact`]'s own doc comment.)
fn resolve_format_artefact(
    format: Format,
    prepared_dir: &Path,
    base: &str,
    transport: Transport,
    base_url: Option<&str>,
) -> ArtefactResolution {
    let local_path = match format.artefact(base) {
        Artefact::Prepared(name) => prepared_dir.join(name),
        Artefact::NotCoordinated => return ArtefactResolution::NotCoordinated,
    };

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

    let output = cmd.output().with_context(|| {
        format!("spawning child process (format={format}, scenario={scenario})")
    })?;
    if !output.status.success() {
        bail!(
            "child process failed (format={format}, scenario={scenario}); stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let notes = child_disclosures(&String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).context("child stdout was not valid UTF-8")?;
    let line = stdout.trim();
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
/// `Project`/`IdLookup`), [`run`] instead uses the dataset-global CityObject
/// total — this same function called once against the `cityparquet` package
/// — as a SHARED denominator across every format, so those scenarios'
/// selectivity is directly comparable and always in `(0, 1]` (see this
/// module's own doc comment).
fn total_count_for(format: Format, source: &Source) -> Result<u64> {
    let line = spawn_child(format, Scenario::Count, source, &QueryParams::default())?;
    Ok(line.result_count)
}

/// Runs one (format, scenario, params) measurement: `repeat + 1` fresh child
/// processes (the first discarded as a warmup), then the MEDIAN `time_s` (+
/// MAD), the MAX `peak_heap_bytes`/`ru_maxrss_bytes` across the `repeat` warm
/// samples, and `result_count` from the first warm sample (every warm sample
/// measures the identical scenario against the identical unmodified input,
/// so they always agree on `result_count`; only the timing/memory varies).
/// Buffers one CSV row (see [`run`] on why rows are held to the end) and
/// returns the `result_count` for callers that need it (the self-consistency
/// check in [`run`]).
#[allow(clippy::too_many_arguments)]
fn run_measurement(
    rows: &mut Vec<Row>,
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
    // Likewise the first warm sample's own disclosures (see
    // [`ChildLine::notes`]): which mechanism a runner took is a property of
    // the artefact, identical in every repeat.
    let mut child_notes: Vec<String> = Vec::new();

    for i in 0..=repeat {
        let line = spawn_child(format, scenario, source, params)?;
        if i == 0 {
            // Warmup: discarded entirely (never contributes to the median,
            // the MAX peak metrics, or `result_count`).
            continue;
        }
        times.push(line.time_s);
        peak_heap_max = peak_heap_max.max(line.peak_heap_bytes);
        peak_rss_max = peak_rss_max.max(line.ru_maxrss_bytes);
        if result_count.is_none() {
            result_count = Some(line.result_count);
            io = line.io;
            child_notes = line.notes;
        }
    }

    let result_count = result_count.expect("repeat >= 1 guarantees at least one warm sample");
    let time_s = median(&times);
    let time_mad_s = mad(&times, time_s);

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
        measure: Measure::Read(scenario),
        selectivity,
        result_count,
        time_s,
        time_mad_s,
        peak_heap_bytes: peak_heap_max,
        peak_rss_bytes: peak_rss_max,
        repeat,
        notes: tags,
        io,
    });

    Ok(result_count)
}

/// One variant's write: a discarded warmup and `write_repeat` warm repeats,
/// each a child process converting into a fresh directory INSIDE
/// `prepared_dir` (a rename across filesystems would fail, and a 1M-object
/// package does not belong in /tmp). The last repeat's package is kept as
/// `<prepared_dir>/<base>.<id>.parquet`; returns that path.
fn run_write(
    rows: &mut Vec<Row>,
    dataset: &str,
    base: &str,
    id: &str,
    prepared_dir: &Path,
    seq: &Path,
    write_repeat: usize,
) -> Result<PathBuf> {
    let self_exe = std::env::current_exe().context("cannot determine own executable path")?;
    let mut times = Vec::with_capacity(write_repeat);
    let mut peak_heap_max = 0u64;
    let mut peak_rss_max = 0u64;
    let mut object_count: Option<u64> = None;
    let mut kept: Option<tempfile::TempDir> = None;

    for i in 0..=write_repeat {
        let scratch = tempfile::Builder::new()
            .prefix(&format!(".{base}.{id}.repeat."))
            .tempdir_in(prepared_dir)
            .with_context(|| {
                format!("creating a scratch directory in {}", prepared_dir.display())
            })?;
        let out = scratch.path().join("pkg");
        let output = Command::new(&self_exe)
            .arg("--child")
            .arg("--write")
            .arg("--variant")
            .arg(id)
            .arg("--input")
            .arg(seq)
            .arg("--out")
            .arg(&out)
            .output()
            .with_context(|| format!("spawning the write child (variant={id})"))?;
        if !output.status.success() {
            bail!(
                "write child failed (variant={id}); stderr:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let stdout =
            String::from_utf8(output.stdout).context("write child stdout was not valid UTF-8")?;
        let fields: Vec<&str> = stdout.split_whitespace().collect();
        if fields.len() != 4 {
            bail!(
                "expected 4 fields from the write child, got {} in '{}'",
                fields.len(),
                stdout.trim()
            );
        }
        let time_s: f64 = fields[0]
            .parse()
            .with_context(|| format!("parsing time_s from '{}'", fields[0]))?;
        let heap: u64 = fields[1]
            .parse()
            .with_context(|| format!("parsing peak_heap_bytes from '{}'", fields[1]))?;
        let rss: u64 = fields[2]
            .parse()
            .with_context(|| format!("parsing ru_maxrss_bytes from '{}'", fields[2]))?;
        let count: u64 = fields[3]
            .parse()
            .with_context(|| format!("parsing object_count from '{}'", fields[3]))?;
        if i == 0 {
            continue; // warmup: the directory drops here and is deleted
        }
        times.push(time_s);
        peak_heap_max = peak_heap_max.max(heap);
        peak_rss_max = peak_rss_max.max(rss);
        object_count = Some(count);
        kept = Some(scratch); // an earlier warm repeat's TempDir drops and is deleted
    }

    let kept = kept.expect("write_repeat >= 1 guarantees a kept repeat");
    let target = prepared_dir.join(format!("{base}.{id}.parquet"));
    if target.exists() {
        fs::remove_dir_all(&target)
            .with_context(|| format!("removing the previous {}", target.display()))?;
    }
    let scratch = kept.keep();
    fs::rename(scratch.join("pkg"), &target)
        .with_context(|| format!("moving the kept package to {}", target.display()))?;
    fs::remove_dir(&scratch).with_context(|| format!("removing {}", scratch.display()))?;

    let time_s = median(&times);
    rows.push(Row {
        dataset: dataset.to_string(),
        label: id.to_string(),
        measure: Measure::Write,
        selectivity: None,
        result_count: object_count.expect("at least one warm repeat"),
        time_s,
        time_mad_s: mad(&times, time_s),
        peak_heap_bytes: peak_heap_max,
        peak_rss_bytes: peak_rss_max,
        repeat: write_repeat,
        notes: Vec::new(),
        io: None,
    });
    Ok(target)
}

/// The median of `values` (must be non-empty).
fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("time_s is always finite"));
    let n = sorted.len();
    let mid = n / 2;
    if n.is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

/// The median absolute deviation of `values` from `med` (must be non-empty).
fn mad(values: &[f64], med: f64) -> f64 {
    let deviations: Vec<f64> = values.iter().map(|v| (v - med).abs()).collect();
    median(&deviations)
}

/// What one CSV row measured: a conversion or one read scenario. `write`
/// never enters [`Scenario::ALL`], so `--scenarios write` is rejected by the
/// scenario parser and a write row can only come from the `--variants` path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Measure {
    Write,
    Read(Scenario),
}

impl std::fmt::Display for Measure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Measure::Write => f.write_str("write"),
            Measure::Read(s) => f.write_str(s.as_str()),
        }
    }
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
    measure: Measure,
    selectivity: Option<f64>,
    result_count: u64,
    time_s: f64,
    time_mad_s: f64,
    peak_heap_bytes: u64,
    peak_rss_bytes: u64,
    repeat: usize,
    notes: Vec<String>,
    io: Option<IoStats>,
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
        let notes = self.notes.join(";");
        let (dataset, format, scenario, result_count, time_s, time_mad_s) = (
            &self.dataset,
            &self.label,
            self.measure,
            self.result_count,
            self.time_s,
            self.time_mad_s,
        );
        let (peak_heap_bytes, peak_rss_bytes, repeat) =
            (self.peak_heap_bytes, self.peak_rss_bytes, self.repeat);
        format!(
            "{dataset},{format},{scenario},{selectivity_field},{result_count},{time_s:.6},\
             {time_mad_s:.6},{peak_heap_bytes},{peak_rss_bytes},{repeat},{notes},\
             {bytes_field},{requests_field}"
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
            measure: Measure::Read(scenario),
            selectivity: None,
            result_count: 1116,
            time_s: 0.5,
            time_mad_s: 0.0,
            peak_heap_bytes: 1,
            peak_rss_bytes: 2,
            repeat: 1,
            notes: notes.iter().map(|n| (*n).to_string()).collect(),
            io: None,
        }
    }

    /// The `notes` column is one CSV field, so several tags on one row must
    /// never introduce a comma — that would shift `bytes_read` and
    /// `http_requests` one column left for that row alone.
    #[test]
    fn several_notes_tags_stay_inside_one_csv_field() {
        let rendered = row(
            Scenario::AttrFilter,
            &["object_type=Building", "no-attr-index"],
        )
        .render();
        assert_eq!(
            rendered.split(',').count(),
            CSV_HEADER.split(',').count(),
            "a multi-tag row must still have exactly {} fields: {rendered}",
            CSV_HEADER.split(',').count()
        );
        assert!(
            rendered.contains("object_type=Building;no-attr-index"),
            "tags must be joined with ';': {rendered}"
        );
    }

    /// A run whose formats disagreed on `AttrFilter(object_type)` is not
    /// measuring the same query in every row of that scenario. Warning about
    /// it on stderr alone leaves the CSV — the thing that gets published and
    /// plotted — indistinguishable from a clean run's.
    #[test]
    fn a_spoiled_run_is_visible_in_the_csv_not_only_on_stderr() {
        let mut rows = vec![
            row(Scenario::AttrFilter, &["object_type=Building"]),
            row(Scenario::Count, &[]),
        ];
        tag_attr_filter_mismatch(&mut rows);
        assert!(
            rows[0].render().contains(ATTR_FILTER_MISMATCH),
            "the attr-filter row must carry the disclosure: {}",
            rows[0].render()
        );
        assert!(
            !rows[1].render().contains(ATTR_FILTER_MISMATCH),
            "only the rows whose counts disagreed are tagged: {}",
            rows[1].render()
        );
    }

    /// A `cold` row is excluded from the charts by an EXACT `notes == "cold"`
    /// test in `benchmark/plot/readbench_plot/plot.py`, so its `notes` field must
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
}
