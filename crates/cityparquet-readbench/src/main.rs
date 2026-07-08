mod alloc;
mod formats;
mod scenario;

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::Parser;

use scenario::{AttrPred, QueryParams, Scenario};

/// Cross-format read benchmark for CityParquet (FlatCityBuf, GeoParquet, etc.).
///
/// This binary has two entry points sharing one CLI surface: the
/// coordinator (default; drives a whole benchmark matrix, Task 11 — not
/// yet implemented) and `--child` (this task) — a single-scenario worker
/// the coordinator spawns once per (format, scenario, dataset, repeat)
/// measurement. `--child` resets the heap allocator, times exactly one
/// `FormatRunner::run` call, and prints one line to stdout:
/// `time_s peak_heap_bytes ru_maxrss_bytes result_count`.
#[derive(Parser, Debug)]
#[command(name = "cityparquet-readbench", version, about)]
struct Cli {
    /// Run as the single-scenario child process (see the module doc
    /// comment). All of `--format`/`--scenario`/`--input` are required in
    /// this mode.
    #[arg(long)]
    child: bool,

    /// Format backend: `cityparquet` (implemented), or
    /// `cityjsonseq`/`cityjsonseq-gz`/`flatcitybuf`/`duckdb-parquet`
    /// (reserved for later tasks).
    #[arg(long)]
    format: Option<String>,

    /// Scenario to run: full-read, count, bbox-query, attr-filter,
    /// attr-stats, id-lookup, project.
    #[arg(long)]
    scenario: Option<String>,

    /// Format-specific input path: a CityParquet package directory (or its
    /// main table file directly), a `.city.jsonl`/`.jsonl.gz` file, or a
    /// `.fcb` file.
    #[arg(long)]
    input: Option<PathBuf>,

    /// Query window for `bbox-query`, as six comma-separated numbers:
    /// `minx,miny,minz,maxx,maxy,maxz`.
    #[arg(long, value_delimiter = ',')]
    bbox: Option<Vec<f64>>,

    /// Attribute column for `attr-filter` / `attr-stats` / `project`.
    #[arg(long)]
    attr_column: Option<String>,

    /// Equality predicate value for `attr-filter`: ALWAYS a string
    /// comparison (see `build_attr_pred`'s own doc comment) — numeric
    /// equality is not supported via this flag; use `--attr-ge`/`--attr-le`
    /// (together, a closed range) for numeric predicates.
    #[arg(long)]
    attr_eq: Option<String>,

    /// `>=` bound for `attr-filter` (combine with `--attr-le` for a closed
    /// range).
    #[arg(long)]
    attr_ge: Option<f64>,

    /// `<=` bound for `attr-filter` (combine with `--attr-ge` for a closed
    /// range).
    #[arg(long)]
    attr_le: Option<f64>,

    /// Target object id for `id-lookup`.
    #[arg(long)]
    target_id: Option<String>,

    /// Free-text selectivity label the (Task 11) coordinator threads
    /// through to the results CSV's `notes` column; no scenario reads this
    /// itself.
    #[arg(long)]
    selectivity_tag: Option<String>,
}

fn main() {
    let cli = Cli::parse();
    if let Err(err) = run(cli) {
        eprintln!("cityparquet-readbench: {err:#}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    if !cli.child {
        bail!(
            "the coordinator entry point is not implemented yet (Task 11); run with \
             --child --format <f> --scenario <s> --input <path> to execute a single \
             scenario directly"
        );
    }

    let format = cli.format.context("--child requires --format")?;
    let scenario_str = cli.scenario.context("--child requires --scenario")?;
    let input = cli.input.context("--child requires --input")?;
    let scenario: Scenario = scenario_str
        .parse()
        .map_err(|e: String| anyhow::anyhow!(e))?;

    let bbox = match cli.bbox {
        Some(values) => Some(<[f64; 6]>::try_from(values.clone()).map_err(|_| {
            anyhow::anyhow!(
                "--bbox needs exactly 6 comma-separated numbers, got {}",
                values.len()
            )
        })?),
        None => None,
    };
    let attr_pred = build_attr_pred(cli.attr_eq.as_deref(), cli.attr_ge, cli.attr_le)?;

    let params = QueryParams {
        bbox,
        attr_column: cli.attr_column,
        attr_pred,
        target_id: cli.target_id,
        selectivity_tag: cli.selectivity_tag,
    };

    let runner = formats::resolve(&format)?;

    alloc::reset();
    let start = Instant::now();
    let result_count = runner.run(&input, scenario, &params)?;
    let time_s = start.elapsed().as_secs_f64();
    let peak_heap_bytes = alloc::peak_heap_bytes();
    let ru_maxrss_bytes = max_rss_bytes()?;

    println!("{time_s:.6} {peak_heap_bytes} {ru_maxrss_bytes} {result_count}");
    Ok(())
}

/// Builds a single [`AttrPred`] from the `--attr-eq`/`--attr-ge`/`--attr-le`
/// flags: `--attr-eq` alone always becomes [`AttrPred::Eq`] of a JSON
/// STRING — never coerced to a number, even when `raw` looks numeric (e.g.
/// `--attr-eq 1070`) — because numeric equality is not what `--attr-eq`
/// means; `--attr-ge`+`--attr-le` together become a closed
/// [`AttrPred::Range`]; either alone becomes [`AttrPred::Ge`]/
/// [`AttrPred::Le`]; none of the three yields `None` (scenarios that need a
/// predicate report their own missing-flag error later, in the runner).
///
/// **Why string-only, never numeric, equality.** Before this fix, a
/// numeric-looking `--attr-eq` value was eagerly parsed into a JSON number,
/// which silently broke equality against STRING-typed numeric attribute
/// codes — common in real CityJSON data (e.g. `lod3_railway.city.json`'s
/// `function` attribute stores values like `"1070"` as strings, not
/// numbers). The CityParquet runner's `query::attr_filter` (via
/// `evaluate_attr_predicate`) requires `Eq(Value::String(_))` for its
/// `Utf8`/`Dictionary<_, Utf8>` columns and rejects a numeric `Eq` outright;
/// the CityJSONSeq runner's own `matches_predicate` silently returned zero
/// matches instead of erroring (a JSON-number `want` compared via
/// `value.as_f64()` against a JSON-string cell, which is always `None`); the
/// FlatCityBuf runner's `eq_key` masked the bug entirely with its own local
/// number-to-string recovery. Making `--attr-eq` always a string value
/// removes the ambiguity at its one source: every format runner's `Eq`
/// dispatch already picks string-vs-numeric comparison from the *column's*
/// own type (see `query::evaluate_attr_predicate`'s `Utf8`/`Dictionary`
/// arms, `formats::cityjsonseq::matches_predicate`'s `want.as_str()` arm,
/// and `formats::flatcitybuf::eq_key`'s `ColumnType::String` arm), so a
/// string `want` still compares correctly against a JSON-string cell
/// whether that cell's own content looks numeric or not — the numeric
/// comparisons this benchmark actually needs (`AttrStats`, `Ge`/`Le`/
/// `Range` filters) go through `--attr-ge`/`--attr-le`/both instead, never
/// `--attr-eq`.
fn build_attr_pred(eq: Option<&str>, ge: Option<f64>, le: Option<f64>) -> Result<Option<AttrPred>> {
    if let Some(raw) = eq {
        return Ok(Some(AttrPred::Eq(serde_json::Value::String(
            raw.to_string(),
        ))));
    }
    Ok(match (ge, le) {
        (Some(g), Some(l)) => Some(AttrPred::Range(g, l)),
        (Some(g), None) => Some(AttrPred::Ge(g)),
        (None, Some(l)) => Some(AttrPred::Le(l)),
        (None, None) => None,
    })
}

/// `getrusage(RUSAGE_SELF).ru_maxrss`, in BYTES. On macOS `ru_maxrss` is
/// natively reported in bytes (a real cross-platform gotcha: Linux reports
/// it in KiB instead — the milestone's methodology doc discloses this;
/// this crate targets macOS development machines, so no `cfg`-gated
/// conversion is applied here).
fn max_rss_bytes() -> Result<u64> {
    // SAFETY: `usage` is zero-initialized and only read after `getrusage`
    // returns success; `RUSAGE_SELF` and a valid `&mut rusage` are exactly
    // what this libc binding requires.
    let usage = unsafe {
        let mut usage: libc::rusage = std::mem::zeroed();
        let rc = libc::getrusage(libc::RUSAGE_SELF, &mut usage);
        if rc != 0 {
            bail!("getrusage failed: {}", std::io::Error::last_os_error());
        }
        usage
    };
    Ok(usage.ru_maxrss as u64)
}
