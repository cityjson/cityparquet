mod alloc;
mod coordinator;
mod formats;
mod scenario;

use std::path::PathBuf;
use std::str::FromStr as _;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use cityparquet_readbench::format::Format;
use clap::{Args, Parser, Subcommand};

use scenario::{AttrPred, QueryParams, Scenario};

/// Cross-format read benchmark for CityParquet (FlatCityBuf, GeoParquet, etc.).
///
/// This binary has two entry points sharing one CLI surface: the `run`
/// subcommand (the coordinator — drives a whole (format x scenario) matrix,
/// medians the repeats, and writes the results CSV; see [`coordinator`]) and
/// `--child` (a plain top-level flag, no subcommand keyword) — a
/// single-scenario worker the coordinator spawns once per (format, scenario,
/// dataset, repeat) measurement. `--child` resets the heap allocator, times
/// exactly one `FormatRunner::run` call, and prints one line to stdout:
/// `time_s peak_heap_bytes ru_maxrss_bytes result_count`.
#[derive(Parser, Debug)]
#[command(name = "cityparquet-readbench", version, about)]
struct Cli {
    /// The coordinator's own subcommand (`run`). Left unset when invoking
    /// `--child` directly (no positional subcommand keyword appears in that
    /// case, so clap never attempts to match one).
    #[command(subcommand)]
    command: Option<Command>,

    /// Run as the single-scenario child process (see the module doc
    /// comment). All of `--format`/`--scenario`/`--input` are required in
    /// this mode.
    #[arg(long)]
    child: bool,

    /// Format backend — one of `Format::ALL`'s canonical names, which
    /// `Format::from_str` validates (and whose error lists them all), so no
    /// list is repeated here to drift out of date.
    #[arg(long, value_parser = parse_format)]
    format: Option<Format>,

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

    /// Free-text selectivity label the coordinator threads through to the
    /// results CSV's `notes` column; no scenario reads this itself.
    #[arg(long)]
    selectivity_tag: Option<String>,

    /// Transport for `--child`'s own `--input`: `local` (a filesystem path,
    /// default) or `http` (an HTTP base URL + relative key, combined with
    /// `--base-url`).
    #[arg(long, default_value = "local")]
    transport: String,

    /// HTTP base URL (required when `--transport http`); `--input` becomes
    /// the relative key under it.
    #[arg(long)]
    base_url: Option<String>,
}

/// The coordinator's own subcommand.
#[derive(Subcommand, Debug)]
enum Command {
    /// Drive a whole (format x scenario) benchmark matrix and write the
    /// results CSV (see [`coordinator::run`]).
    Run(RunArgs),
}

/// `cityparquet-readbench run`'s own flags — the CLI-facing mirror of
/// [`coordinator::RunOptions`].
#[derive(Args, Debug)]
struct RunArgs {
    /// The original CityJSON/CityJSONSeq input (also the `cityjsonseq`
    /// format's own artefact, read directly — never converted or copied).
    #[arg(long)]
    input: PathBuf,

    /// Directory `just readbench-prepare` wrote the per-format artefacts
    /// into.
    #[arg(long, default_value = "bench/data/readbench")]
    prepared_dir: PathBuf,

    /// Result CSV path. This run OWNS the file: a fresh truncate + write, so
    /// a re-run is always clean (never an append).
    #[arg(long)]
    out: PathBuf,

    /// Warm repeats per measurement; a further, discarded warmup precedes
    /// every one. Must be >= 1.
    #[arg(long, default_value_t = 7)]
    repeat: usize,

    /// Comma-separated format names — one of `Format::ALL`'s canonical
    /// names each, validated by `Format::from_str` (an unknown name is
    /// rejected here, never silently skipped); omit for
    /// `Format::DEFAULT_SET`, the format-comparison set. `Format::ORDERING_SET`
    /// names the other measured set (`just ordering-bench` passes it).
    #[arg(long, value_delimiter = ',', value_parser = parse_format)]
    formats: Option<Vec<Format>>,

    /// Comma-separated scenario names (`full-read`, `count`, `bbox-query`,
    /// `attr-filter`, `attr-stats`, `id-lookup`, `project`, or their
    /// [`Scenario::from_str`] aliases); omit for every scenario.
    #[arg(long, value_delimiter = ',')]
    scenarios: Option<Vec<String>>,

    /// After the warm matrix, run one additional `FullRead` per format,
    /// tagged `cold` in `notes` (see [`coordinator::run`]'s own doc comment
    /// on the `sudo purge` protocol this does NOT automate).
    #[arg(long)]
    cold: bool,

    /// Transport for every measurement in this run: `local` (default) or
    /// `http` (requires `--base-url`).
    #[arg(long, default_value = "local")]
    transport: String,

    /// HTTP base URL when `--transport http`.
    #[arg(long)]
    base_url: Option<String>,
}

fn main() {
    let cli = Cli::parse();
    if let Err(err) = run(cli) {
        eprintln!("cityparquet-readbench: {err:#}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    if let Some(Command::Run(run_args)) = cli.command {
        let transport = match run_args.transport.as_str() {
            "local" => coordinator::Transport::Local,
            "http" => coordinator::Transport::Http,
            other => bail!("unknown --transport '{other}'; expected 'local' or 'http'"),
        };
        return coordinator::run(&coordinator::RunOptions {
            input: run_args.input,
            prepared_dir: run_args.prepared_dir,
            out: run_args.out,
            repeat: run_args.repeat,
            formats: run_args.formats,
            scenarios: run_args.scenarios,
            cold: run_args.cold,
            transport,
            base_url: run_args.base_url,
        });
    }

    if !cli.child {
        bail!(
            "run either `run --input <path> --prepared-dir <dir> --out <csv>` (the \
             coordinator) or `--child --format <f> --scenario <s> --input <path>` (a single \
             measurement)"
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

    let source = match cli.transport.as_str() {
        "local" => formats::Source::Local(input),
        "http" => {
            let base_url = cli
                .base_url
                .context("--transport http requires --base-url")?;
            let key = input
                .to_str()
                .context("--input must be valid UTF-8 for --transport http")?
                .to_string();
            formats::Source::Http { base_url, key }
        }
        other => bail!("unknown --transport '{other}'; expected 'local' or 'http'"),
    };
    let is_http = matches!(source, formats::Source::Http { .. });
    let runner = formats::resolve(format)?;

    alloc::reset();
    let start = Instant::now();
    // For `--transport http`, `runner.run` itself calls
    // `tokio::runtime::Handle::current().block_on(...)` internally (each
    // format's own HTTP arm); this just needs a runtime CONTEXT to find via
    // `Handle::current()` — entering it (not blocking on it here) is what
    // makes that inner `block_on` the only one on the call stack. Wrapping
    // this call in an outer `rt.block_on(async { runner.run(..) })` would
    // instead panic ("Cannot start a runtime from within a runtime") the
    // moment the inner call reaches its own `block_on`.
    //
    // MUST be `new_multi_thread` (not `new_current_thread`), even though
    // this process only ever runs one scenario at a time: reproduced and
    // confirmed in isolation (a standalone `object_store::http::HttpStore`
    // GET, driven the same way — `rt.enter()` here, `Handle::current().
    // block_on(...)` deeper in the call stack) that a `current_thread`
    // runtime entered-but-never-block_on'd-at-this-frame hangs forever on
    // the underlying `reqwest` request; a `multi_thread` runtime (even with
    // a single worker thread) does not.
    let rt = if is_http {
        Some(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .context("building the child's tokio runtime for --transport http")?,
        )
    } else {
        None
    };
    let _rt_guard = rt.as_ref().map(|rt| rt.enter());
    let outcome = runner.run(&source, scenario, &params)?;
    let time_s = start.elapsed().as_secs_f64();
    let peak_heap_bytes = alloc::peak_heap_bytes();
    let ru_maxrss_bytes = max_rss_bytes()?;

    match outcome.io {
        Some(io) => println!(
            "{time_s:.6} {peak_heap_bytes} {ru_maxrss_bytes} {} {} {}",
            outcome.result_count, io.bytes, io.requests
        ),
        None => println!(
            "{time_s:.6} {peak_heap_bytes} {ru_maxrss_bytes} {}",
            outcome.result_count
        ),
    }
    Ok(())
}

/// clap's value parser for `--format`/`--formats`: [`Format`]'s own
/// `FromStr`, so an unknown name is REJECTED at parse time with the enum's
/// own every-variant error message. Previously an unknown `--formats` entry
/// was silently skipped with a warning deep inside the coordinator, which
/// hid a typo behind a benchmark that quietly measured less than asked.
fn parse_format(raw: &str) -> Result<Format, String> {
    Format::from_str(raw)
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

/// Convert a raw `getrusage(RUSAGE_SELF).ru_maxrss` reading into BYTES.
///
/// The unit is platform-defined: Linux reports **KiB** (`getrusage(2)`:
/// "expressed in kilobytes"); macOS/BSD reports **bytes**. Kept as a pure
/// function so the conversion itself is unit-testable. Non-Linux, non-macOS
/// platforms fall through to the raw value (BSD-lineage bytes) — this crate
/// only ever runs on the two.
fn rss_to_bytes(raw: i64) -> u64 {
    #[cfg(target_os = "linux")]
    {
        (raw as u64).saturating_mul(1024)
    }
    #[cfg(not(target_os = "linux"))]
    {
        raw as u64
    }
}

/// `getrusage(RUSAGE_SELF).ru_maxrss`, normalised to BYTES on every
/// platform via [`rss_to_bytes`].
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
    Ok(rss_to_bytes(usage.ru_maxrss))
}

#[cfg(test)]
mod tests {
    use super::rss_to_bytes;

    /// P1 regression: `ru_maxrss`'s unit is platform-defined — KiB on Linux
    /// (`getrusage(2)`), bytes on macOS/BSD. Before `rss_to_bytes` existed
    /// the raw value was reported as bytes unconditionally, under-reporting
    /// Linux peak RSS 1024x in every Linux-produced results CSV.
    #[test]
    fn rss_to_bytes_converts_the_platform_unit_to_bytes() {
        #[cfg(target_os = "linux")]
        assert_eq!(rss_to_bytes(2048), 2048 * 1024, "Linux ru_maxrss is KiB");
        #[cfg(not(target_os = "linux"))]
        assert_eq!(rss_to_bytes(2048), 2048, "macOS/BSD ru_maxrss is bytes");
    }
}
