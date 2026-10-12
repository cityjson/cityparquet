//! `lod-normalise INPUT OUTPUT`: the first stage of the benchmark's
//! preparation. Leaves every CityObject of a CityJSON document (`.city.json`)
//! or CityJSONSeq (`.city.jsonl`) with at most one geometry per LoD, the
//! first in source order, and removes the vertices nothing references any
//! more (see [`cityparquet_readbench::lod::keep_first_per_lod`]).
//!
//! A source that needs no change is copied byte for byte. A CityJSONSeq is
//! normalised feature by feature (each feature indexes its own vertices) and
//! its header line is copied as is; a document is normalised as a whole,
//! against its one shared vertex list.
//!
//! Prints one line to stdout, `key=value` pairs read by
//! `readbench_prepare.sh`:
//! `dropped=N geometries=BEFORE->AFTER vertices=BEFORE->AFTER`.

use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use cityparquet_readbench::lod::{Normalised, keep_first_per_lod, keep_first_per_lod_line};
use clap::Parser;
use serde_json::Value;

#[derive(Parser)]
struct Args {
    /// The source: a `.city.json` document or a `.city.jsonl` sequence.
    input: PathBuf,
    /// Where the normalised source is written.
    output: PathBuf,
}

fn is_seq(path: &std::path::Path) -> bool {
    path.extension().is_some_and(|e| e == "jsonl")
}

fn main() -> Result<()> {
    let args = Args::parse();
    let tmp = args.output.with_extension("normalising.tmp");
    let report = if is_seq(&args.input) {
        normalise_seq(&args, &tmp)?
    } else {
        normalise_document(&args, &tmp)?
    };
    if report.dropped == 0 {
        std::fs::copy(&args.input, &tmp).context("copying the unchanged source")?;
    }
    std::fs::rename(&tmp, &args.output).context("placing the normalised source")?;
    println!(
        "dropped={} geometries={}->{} vertices={}->{}",
        report.dropped,
        report.geometries_before,
        report.geometries_after,
        report.vertices_before,
        report.vertices_after
    );
    Ok(())
}

fn normalise_document(args: &Args, tmp: &std::path::Path) -> Result<Normalised> {
    let text = std::fs::read_to_string(&args.input)
        .with_context(|| format!("reading {}", args.input.display()))?;
    let mut doc: Value = serde_json::from_str(&text).context("parsing the CityJSON document")?;
    drop(text);
    let report = keep_first_per_lod(&mut doc)?;
    if report.dropped > 0 {
        let mut out = BufWriter::new(std::fs::File::create(tmp)?);
        serde_json::to_writer(&mut out, &doc)?;
        out.flush()?;
    }
    Ok(report)
}

fn normalise_seq(args: &Args, tmp: &std::path::Path) -> Result<Normalised> {
    let input = BufReader::new(
        std::fs::File::open(&args.input)
            .with_context(|| format!("opening {}", args.input.display()))?,
    );
    let mut out = BufWriter::new(std::fs::File::create(tmp)?);
    let mut report = Normalised::default();
    for (i, line) in input.lines().enumerate() {
        let line = line?;
        // The header (and any blank line) carries no CityObject to normalise.
        if i == 0 || line.trim().is_empty() {
            writeln!(out, "{line}")?;
            continue;
        }
        let (normalised, r) = keep_first_per_lod_line(&line)
            .with_context(|| format!("line {} of {}", i + 1, args.input.display()))?;
        report += r;
        writeln!(out, "{normalised}")?;
    }
    out.flush()?;
    Ok(report)
}
