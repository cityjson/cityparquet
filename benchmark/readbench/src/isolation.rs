//! Process isolation on a shared host: NUMA pinning of the measured
//! children, an optional memory ceiling, and a load gate before every sample.
//!
//! Everything that decides is a PURE function of strings and tool
//! availability, so it is unit-tested on any platform. The thin impure layer
//! ([`probe`], [`read_load`]) reads `/sys` and `/proc` and looks tools up on
//! `PATH`; off Linux it returns explicit `"not applied: <reason>"` records and
//! never fails the run.
//!
//! The child command is composed outermost-first: `systemd-run` (memory
//! ceiling), then `numactl`/`taskset` (pinning), then the child itself.

use std::path::Path;
use std::time::Duration;

use anyhow::{Result, bail};
use serde::Serialize;

/// `--numa-node`: a node id, `auto` (most free memory) or `off`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumaRequest {
    Off,
    Auto,
    Node(u32),
}

impl std::str::FromStr for NumaRequest {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.trim() {
            "off" => Ok(Self::Off),
            "auto" | "" => Ok(Self::Auto),
            n => match n.parse() {
                Ok(id) => Ok(Self::Node(id)),
                Err(_) => bail!("--numa-node takes a node id, `auto` or `off`, not {n:?}"),
            },
        }
    }
}

impl std::fmt::Display for NumaRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Off => f.write_str("off"),
            Self::Auto => f.write_str("auto"),
            Self::Node(n) => write!(f, "{n}"),
        }
    }
}

/// `--max-load`: a threshold, `auto` (half the pinned node's cores) or `off`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MaxLoadRequest {
    Off,
    Auto,
    Value(f64),
}

impl std::str::FromStr for MaxLoadRequest {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.trim() {
            "off" => Ok(Self::Off),
            "auto" | "" => Ok(Self::Auto),
            x => match x.parse::<f64>() {
                Ok(v) if v.is_finite() && v > 0.0 => Ok(Self::Value(v)),
                _ => bail!("--max-load takes a positive number, `auto` or `off`, not {x:?}"),
            },
        }
    }
}

impl std::fmt::Display for MaxLoadRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Off => f.write_str("off"),
            Self::Auto => f.write_str("auto"),
            Self::Value(v) => write!(f, "{v}"),
        }
    }
}

/// Parses a kernel cpulist such as `"0-31,64-95"` (sorted, deduplicated).
pub fn parse_cpulist(s: &str) -> Result<Vec<u32>> {
    let mut cpus = Vec::new();
    for part in s.trim().split(',').filter(|p| !p.is_empty()) {
        match part.split_once('-') {
            Some((a, b)) => {
                let (a, b): (u32, u32) = (a.trim().parse()?, b.trim().parse()?);
                if a > b {
                    bail!("descending cpulist range {part:?}");
                }
                cpus.extend(a..=b);
            }
            None => cpus.push(part.trim().parse()?),
        }
    }
    cpus.sort_unstable();
    cpus.dedup();
    Ok(cpus)
}

/// Formats cores back into compact cpulist ranges (`[1,2,3,65]` -> `"1-3,65"`).
pub fn format_cpulist(cpus: &[u32]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < cpus.len() {
        let start = cpus[i];
        let mut end = start;
        while i + 1 < cpus.len() && cpus[i + 1] == end + 1 {
            i += 1;
            end = cpus[i];
        }
        out.push(if start == end {
            start.to_string()
        } else {
            format!("{start}-{end}")
        });
        i += 1;
    }
    out.join(",")
}

/// `MemFree` of one `/sys/devices/system/node/nodeN/meminfo`
/// (`"Node 0 MemFree:  1234 kB"`), in bytes.
pub fn parse_node_memfree(meminfo: &str) -> Option<u64> {
    meminfo_field(meminfo, "MemFree:")
}

/// `MemAvailable` of `/proc/meminfo`, in bytes.
pub fn parse_mem_available(meminfo: &str) -> Option<u64> {
    meminfo_field(meminfo, "MemAvailable:")
}

/// `MemTotal` of `/proc/meminfo`, in bytes.
pub fn parse_mem_total(meminfo: &str) -> Option<u64> {
    meminfo_field(meminfo, "MemTotal:")
}

fn meminfo_field(meminfo: &str, key: &str) -> Option<u64> {
    meminfo.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        words.position(|w| w == key)?;
        let kb: u64 = words.next()?.parse().ok()?;
        Some(kb * 1024)
    })
}

/// One `/proc/loadavg` reading.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct LoadAvg {
    pub load1: f64,
    pub runnable: u32,
    pub total: u32,
}

/// Parses `/proc/loadavg` (`"0.52 0.58 0.59 2/1234 5678"`).
pub fn parse_loadavg(s: &str) -> Option<LoadAvg> {
    let mut words = s.split_whitespace();
    let load1 = words.next()?.parse().ok()?;
    let (runnable, total) = words.nth(2)?.split_once('/')?;
    Some(LoadAvg {
        load1,
        runnable: runnable.parse().ok()?,
        total: total.parse().ok()?,
    })
}

/// One NUMA node's topology and free memory at run start.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeInfo {
    pub id: u32,
    pub cpus: Vec<u32>,
    pub mem_free_bytes: Option<u64>,
}

/// `auto`: the node with the most free memory, ties to the lowest id; a node
/// without a reading ranks last. `None` only when there are no nodes.
pub fn pick_auto_node(nodes: &[NodeInfo]) -> Option<u32> {
    nodes
        .iter()
        .max_by(|a, b| {
            a.mem_free_bytes
                .cmp(&b.mem_free_bytes)
                .then(b.id.cmp(&a.id))
        })
        .map(|n| n.id)
}

/// The coordinator's reserved core and the children's cores. The node's
/// first core goes to the coordinator; a one-core node is shared (`shared`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreSplit {
    pub coordinator: u32,
    pub children: Vec<u32>,
    pub shared: bool,
}

pub fn split_cores(cpus: &[u32]) -> Option<CoreSplit> {
    let (&first, rest) = cpus.split_first()?;
    Some(if rest.is_empty() {
        CoreSplit {
            coordinator: first,
            children: vec![first],
            shared: true,
        }
    } else {
        CoreSplit {
            coordinator: first,
            children: rest.to_vec(),
            shared: false,
        }
    })
}

/// Which tools are on `PATH`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tools {
    pub numactl: bool,
    pub taskset: bool,
    pub systemd_run: bool,
}

/// What pinning was requested and what was applied.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PinRecord {
    pub requested: String,
    /// The node the children run on, `null` when not applied.
    pub node: Option<u32>,
    /// `numactl`, `taskset` or `null`.
    pub tool: Option<String>,
    pub child_cores: Option<String>,
    pub coordinator_core: Option<u32>,
    /// `applied` or `not applied: <reason>`.
    pub status: String,
    /// Memory binding to the node: `applied` or `not applied: <reason>`.
    pub memory_binding: String,
    /// `"shared with the children: one-core node"` when the node has one core.
    pub coordinator_note: Option<String>,
}

/// Builds the pinning argv prefix. Pure: the caller supplies the platform,
/// the topology and the tools found on `PATH`.
pub fn build_pinning(
    is_linux: bool,
    request: NumaRequest,
    nodes: &[NodeInfo],
    tools: Tools,
) -> (Vec<String>, PinRecord) {
    let mut record = PinRecord {
        requested: request.to_string(),
        node: None,
        tool: None,
        child_cores: None,
        coordinator_core: None,
        status: String::new(),
        memory_binding: String::new(),
        coordinator_note: None,
    };
    let not = |record: &mut PinRecord, reason: &str| {
        record.status = format!("not applied: {reason}");
        record.memory_binding = record.status.clone();
    };
    if !is_linux {
        not(&mut record, "not Linux");
        return (Vec::new(), record);
    }
    let id = match request {
        NumaRequest::Off => {
            not(&mut record, "--numa-node off");
            return (Vec::new(), record);
        }
        NumaRequest::Node(id) => id,
        NumaRequest::Auto => pick_auto_node(nodes).unwrap_or(0),
    };
    let Some(node) = nodes.iter().find(|n| n.id == id) else {
        not(&mut record, &format!("no topology for node {id}"));
        return (Vec::new(), record);
    };
    let Some(split) = split_cores(&node.cpus) else {
        not(&mut record, &format!("node {id} lists no cores"));
        return (Vec::new(), record);
    };
    let child_cores = format_cpulist(&split.children);
    let prefix: Vec<String> = if tools.numactl {
        record.tool = Some("numactl".into());
        record.memory_binding = "applied".into();
        let cpu = if split.shared {
            format!("--cpunodebind={id}")
        } else {
            format!("--physcpubind={child_cores}")
        };
        vec!["numactl".into(), cpu, format!("--membind={id}")]
    } else if tools.taskset {
        record.tool = Some("taskset".into());
        record.memory_binding = "not applied: numactl missing".into();
        vec!["taskset".into(), "-c".into(), child_cores.clone()]
    } else {
        not(&mut record, "neither numactl nor taskset found");
        return (Vec::new(), record);
    };
    record.status = "applied".into();
    record.node = Some(id);
    record.child_cores = Some(child_cores);
    record.coordinator_core = Some(split.coordinator);
    if split.shared {
        record.coordinator_note = Some("shared with the children: one-core node".into());
    }
    (prefix, record)
}

/// The `systemd-run` scope that caps the children's memory.
pub fn memory_prefix(bytes: u64) -> Vec<String> {
    [
        "systemd-run",
        "--user",
        "--scope",
        "--quiet",
        "-p",
        &format!("MemoryMax={bytes}"),
        "--",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// The full child argv: memory scope outermost, then pinning, then the child.
pub fn compose(memory: &[String], pinning: &[String], child: &[String]) -> Vec<String> {
    memory.iter().chain(pinning).chain(child).cloned().collect()
}

/// The pinned node's share of the machine load: `load1 * node_cores / total_cores`.
pub fn node_share(load1: f64, node_cores: usize, total_cores: usize) -> f64 {
    if total_cores == 0 {
        return load1;
    }
    load1 * node_cores as f64 / total_cores as f64
}

/// `auto`'s threshold: half the pinned node's cores (all of them, the
/// coordinator's included). Our own child adds about one runnable task, so
/// half the node leaves room for it and light co-tenancy while still
/// catching a node that is genuinely contended.
pub fn resolve_max_load(request: MaxLoadRequest, node_cores: usize) -> Option<f64> {
    match request {
        MaxLoadRequest::Off => None,
        MaxLoadRequest::Auto => Some(node_cores as f64 / 2.0),
        MaxLoadRequest::Value(v) => Some(v),
    }
}

/// Seconds between load re-checks while waiting.
pub const WAIT_STEP: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadDecision {
    Proceed,
    /// Sleep [`WAIT_STEP`] and check again.
    Wait,
    /// Still above the threshold after the longest wait: proceed, tag `busy`.
    Busy,
}

/// The gate before every sample.
pub fn load_decision(
    share: Option<f64>,
    threshold: Option<f64>,
    waited: Duration,
    max_wait: Duration,
) -> LoadDecision {
    match (share, threshold) {
        (Some(share), Some(limit)) if share > limit => {
            if waited + WAIT_STEP <= max_wait {
                LoadDecision::Wait
            } else {
                LoadDecision::Busy
            }
        }
        _ => LoadDecision::Proceed,
    }
}

/// One per-sample load reading.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct LoadSample {
    pub load1: f64,
    pub runnable: u32,
    pub mem_available_bytes: Option<u64>,
}

/// A cell's load summary.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct CellLoad {
    pub load1_max: Option<f64>,
    pub runnable_max: Option<u32>,
    pub mem_available_min_bytes: Option<u64>,
}

impl CellLoad {
    pub fn add(&mut self, s: &LoadSample) {
        self.load1_max = Some(self.load1_max.map_or(s.load1, |m| m.max(s.load1)));
        self.runnable_max = Some(self.runnable_max.map_or(s.runnable, |m| m.max(s.runnable)));
        if let Some(a) = s.mem_available_bytes {
            self.mem_available_min_bytes =
                Some(self.mem_available_min_bytes.map_or(a, |m| m.min(a)));
        }
    }
}

// ---- impure layer -------------------------------------------------------

fn on_path(tool: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(tool).is_file()))
}

/// Probes `PATH` for the tools.
pub fn probe_tools() -> Tools {
    Tools {
        numactl: on_path("numactl"),
        taskset: on_path("taskset"),
        systemd_run: on_path("systemd-run"),
    }
}

/// Reads every `/sys/devices/system/node/node*/{cpulist,meminfo}`.
pub fn read_nodes() -> Vec<NodeInfo> {
    let root = Path::new("/sys/devices/system/node");
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut nodes: Vec<NodeInfo> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let id: u32 = name.strip_prefix("node")?.parse().ok()?;
            let cpus =
                parse_cpulist(&std::fs::read_to_string(e.path().join("cpulist")).ok()?).ok()?;
            let mem_free_bytes = std::fs::read_to_string(e.path().join("meminfo"))
                .ok()
                .and_then(|m| parse_node_memfree(&m));
            Some(NodeInfo {
                id,
                cpus,
                mem_free_bytes,
            })
        })
        .collect();
    nodes.sort_by_key(|n| n.id);
    nodes
}

/// One `/proc` reading, `None` where `/proc` is absent (macOS).
pub fn read_load() -> Option<LoadSample> {
    let load = parse_loadavg(&std::fs::read_to_string("/proc/loadavg").ok()?)?;
    let mem = std::fs::read_to_string("/proc/meminfo").ok();
    Some(LoadSample {
        load1: load.load1,
        runnable: load.runnable,
        mem_available_bytes: mem.as_deref().and_then(parse_mem_available),
    })
}

/// `MemTotal` and `MemAvailable` at start, `None` without `/proc`.
pub fn read_memory() -> Option<(Option<u64>, Option<u64>)> {
    let mem = std::fs::read_to_string("/proc/meminfo").ok()?;
    Some((parse_mem_total(&mem), parse_mem_available(&mem)))
}

/// Probes the memory scope once (`systemd-run ... true`): `Ok(())` when it
/// works, otherwise the reason it does not.
pub fn probe_memory_scope(is_linux: bool, tools: Tools, bytes: u64) -> Result<(), String> {
    if !is_linux {
        return Err("not Linux".into());
    }
    if !tools.systemd_run {
        return Err("systemd-run not found".into());
    }
    let mut argv = memory_prefix(bytes);
    argv.push("true".into());
    match std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .output()
    {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
        Err(e) => Err(e.to_string()),
    }
}

/// Pins the coordinator to `core` (`taskset -p -c <core> <pid>`).
pub fn pin_self(is_linux: bool, tools: Tools, core: u32) -> String {
    if !is_linux {
        return "not applied: not Linux".into();
    }
    if !tools.taskset {
        return "not applied: taskset not found".into();
    }
    let pid = std::process::id().to_string();
    match std::process::Command::new("taskset")
        .args(["-p", "-c", &core.to_string(), &pid])
        .output()
    {
        Ok(out) if out.status.success() => "applied".into(),
        Ok(out) => format!(
            "not applied: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ),
        Err(e) => format!("not applied: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn epyc() -> Vec<NodeInfo> {
        vec![
            NodeInfo {
                id: 0,
                cpus: parse_cpulist("0-31,64-95").unwrap(),
                mem_free_bytes: Some(100),
            },
            NodeInfo {
                id: 1,
                cpus: parse_cpulist("32-63,96-127").unwrap(),
                mem_free_bytes: Some(200),
            },
        ]
    }

    const ALL: Tools = Tools {
        numactl: true,
        taskset: true,
        systemd_run: true,
    };

    #[test]
    fn cpulist_round_trip() {
        assert_eq!(parse_cpulist("0-2,5\n").unwrap(), vec![0, 1, 2, 5]);
        assert_eq!(parse_cpulist("7").unwrap(), vec![7]);
        assert!(parse_cpulist("3-1").is_err());
        assert_eq!(
            format_cpulist(&parse_cpulist("1-31,64-95").unwrap()),
            "1-31,64-95"
        );
    }

    #[test]
    fn meminfo_and_loadavg_parse() {
        let node = "Node 0 MemTotal:  263856412 kB\nNode 0 MemFree:   200000000 kB\n";
        assert_eq!(parse_node_memfree(node), Some(200_000_000 * 1024));
        let proc = "MemTotal:  527712824 kB\nMemFree: 1 kB\nMemAvailable:   400000000 kB\n";
        assert_eq!(parse_mem_available(proc), Some(400_000_000 * 1024));
        assert_eq!(parse_mem_total(proc), Some(527_712_824 * 1024));
        assert_eq!(
            parse_loadavg("0.52 0.58 0.59 2/1234 5678\n"),
            Some(LoadAvg {
                load1: 0.52,
                runnable: 2,
                total: 1234
            })
        );
        assert_eq!(parse_loadavg("garbage"), None);
    }

    #[test]
    fn auto_node_most_free_ties_lowest() {
        assert_eq!(pick_auto_node(&epyc()), Some(1));
        let mut tied = epyc();
        tied[1].mem_free_bytes = Some(100);
        assert_eq!(pick_auto_node(&tied), Some(0));
        assert_eq!(pick_auto_node(&[]), None);
    }

    #[test]
    fn split_reserves_first_core() {
        let split = split_cores(&[4, 5, 6]).unwrap();
        assert_eq!(
            (split.coordinator, split.children, split.shared),
            (4, vec![5, 6], false)
        );
        assert!(split_cores(&[9]).unwrap().shared);
        assert!(split_cores(&[]).is_none());
    }

    #[test]
    fn pinning_prefers_numactl_then_taskset_then_none() {
        let (p, r) = build_pinning(true, NumaRequest::Node(0), &epyc(), ALL);
        assert_eq!(
            p,
            s(&["numactl", "--physcpubind=1-31,64-95", "--membind=0"])
        );
        assert_eq!(
            (r.node, r.coordinator_core, r.status.as_str()),
            (Some(0), Some(0), "applied")
        );
        let taskset_only = Tools {
            numactl: false,
            ..ALL
        };
        let (p, r) = build_pinning(true, NumaRequest::Auto, &epyc(), taskset_only);
        assert_eq!(p, s(&["taskset", "-c", "33-63,96-127"]));
        assert_eq!(r.memory_binding, "not applied: numactl missing");
        let (p, r) = build_pinning(true, NumaRequest::Auto, &epyc(), Tools::default());
        assert!(p.is_empty());
        assert_eq!(r.status, "not applied: neither numactl nor taskset found");
    }

    #[test]
    fn pinning_degrades_explicitly() {
        let (p, r) = build_pinning(false, NumaRequest::Auto, &epyc(), ALL);
        assert!(p.is_empty());
        assert_eq!(r.status, "not applied: not Linux");
        let (_, r) = build_pinning(true, NumaRequest::Off, &epyc(), ALL);
        assert_eq!(r.status, "not applied: --numa-node off");
        let (_, r) = build_pinning(true, NumaRequest::Node(7), &epyc(), ALL);
        assert_eq!(r.status, "not applied: no topology for node 7");
        let one = vec![NodeInfo {
            id: 0,
            cpus: vec![0],
            mem_free_bytes: None,
        }];
        let (p, r) = build_pinning(true, NumaRequest::Auto, &one, ALL);
        assert_eq!(p, s(&["numactl", "--cpunodebind=0", "--membind=0"]));
        assert!(r.coordinator_note.is_some());
    }

    #[test]
    fn composed_child_command_on_linux() {
        let (pin, _) = build_pinning(true, NumaRequest::Node(0), &epyc(), ALL);
        let argv = compose(
            &memory_prefix(8_000_000_000),
            &pin,
            &s(&["/bin/readbench", "--child"]),
        );
        assert_eq!(
            argv,
            s(&[
                "systemd-run",
                "--user",
                "--scope",
                "--quiet",
                "-p",
                "MemoryMax=8000000000",
                "--",
                "numactl",
                "--physcpubind=1-31,64-95",
                "--membind=0",
                "/bin/readbench",
                "--child",
            ])
        );
    }

    #[test]
    fn max_load_and_decision() {
        assert_eq!(
            "auto".parse::<MaxLoadRequest>().unwrap(),
            MaxLoadRequest::Auto
        );
        assert!("-1".parse::<MaxLoadRequest>().is_err());
        assert_eq!("3".parse::<NumaRequest>().unwrap(), NumaRequest::Node(3));
        assert_eq!(resolve_max_load(MaxLoadRequest::Auto, 32), Some(16.0));
        assert_eq!(resolve_max_load(MaxLoadRequest::Off, 32), None);
        assert_eq!(node_share(64.0, 64, 128), 32.0);
        let max = Duration::from_secs(600);
        let d = |share, waited| load_decision(share, Some(16.0), Duration::from_secs(waited), max);
        assert_eq!(d(Some(10.0), 0), LoadDecision::Proceed);
        assert_eq!(d(Some(20.0), 0), LoadDecision::Wait);
        assert_eq!(d(Some(20.0), 590), LoadDecision::Wait);
        assert_eq!(d(Some(20.0), 600), LoadDecision::Busy);
        assert_eq!(d(None, 0), LoadDecision::Proceed);
        assert_eq!(
            load_decision(Some(99.0), None, Duration::ZERO, max),
            LoadDecision::Proceed
        );
    }

    #[test]
    fn cell_load_summary() {
        let mut cell = CellLoad::default();
        cell.add(&LoadSample {
            load1: 1.0,
            runnable: 3,
            mem_available_bytes: Some(50),
        });
        cell.add(&LoadSample {
            load1: 2.0,
            runnable: 1,
            mem_available_bytes: Some(40),
        });
        assert_eq!(
            cell,
            CellLoad {
                load1_max: Some(2.0),
                runnable_max: Some(3),
                mem_available_min_bytes: Some(40)
            }
        );
    }
}
