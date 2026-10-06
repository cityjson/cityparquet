//! `net-sim`: serve a directory over HTTP under a simulated network profile
//! (see `cityparquet_readbench::netsim`). Prints the base URL, serves until
//! stdin closes (or Ctrl-C), then prints the server's request and byte totals.

use std::io::Read;
use std::path::PathBuf;

use cityparquet_readbench::netsim::{NetProfile, NetSim};
use clap::Parser;

#[derive(Parser)]
struct Args {
    /// The directory to serve.
    #[arg(long)]
    root: PathBuf,
    #[arg(long)]
    bandwidth_mbps: f64,
    #[arg(long)]
    latency_ms: f64,
    /// The port on 127.0.0.1 (0 = ephemeral).
    #[arg(long, default_value_t = 0)]
    port: u16,
}

fn main() -> anyhow::Result<()> {
    let a = Args::parse();
    let profile = NetProfile {
        bandwidth_mbps: a.bandwidth_mbps,
        latency_ms: a.latency_ms,
    };
    let sim = NetSim::start_on(&a.root, profile, a.port)?;
    println!("{}", sim.base_url());
    let mut sink = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut sink);
    let t = sim.totals();
    eprintln!(
        "net-sim totals: requests={} body_bytes={} connections={}",
        t.requests, t.body_bytes, t.connections
    );
    Ok(())
}
