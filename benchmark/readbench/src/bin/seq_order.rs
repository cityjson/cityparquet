//! `seq-order <document.city.json> <stream.city.jsonl>`: rewrite the
//! CityJSONSeq stream in place with its features in the document's order
//! (see `cityparquet_readbench::seq_order`). Prints `features=<n>`.

use std::io::{BufWriter, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use cityparquet_readbench::seq_order::{document_order, order_seq_lines};
use clap::Parser;

#[derive(Parser)]
struct Args {
    document: PathBuf,
    stream: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let text = std::fs::read_to_string(&args.document)
        .with_context(|| format!("reading {}", args.document.display()))?;
    let doc: serde_json::Value =
        serde_json::from_str(&text).context("parsing the CityJSON document")?;
    drop(text);
    let order = document_order(&doc)?;
    drop(doc);
    let stream = std::fs::read_to_string(&args.stream)
        .with_context(|| format!("reading {}", args.stream.display()))?;
    let lines: Vec<String> = stream.lines().map(str::to_string).collect();
    drop(stream);
    let ordered = order_seq_lines(&order, lines)?;
    let tmp = args.stream.with_extension("ordering.tmp");
    let mut out = BufWriter::new(std::fs::File::create(&tmp)?);
    for line in &ordered {
        out.write_all(line.as_bytes())?;
        out.write_all(b"\n")?;
    }
    out.flush()?;
    drop(out);
    std::fs::rename(&tmp, &args.stream).context("placing the ordered stream")?;
    println!("features={}", ordered.len() - 1);
    Ok(())
}
