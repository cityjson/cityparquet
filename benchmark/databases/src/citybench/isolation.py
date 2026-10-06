"""Host isolation for the database comparison: NUMA pinning and a load gate.

Everything here degrades explicitly. A step that cannot be applied records
``"not applied: <reason>"`` in the run's ``isolation`` object and the run
continues; no isolation failure ever aborts a run.

What is applied, and where:

- the citybench client process pins itself to one NUMA node's cores with
  ``os.sched_setaffinity``. DuckDB runs inside that process and the
  ``cityparquet`` reader child inherits the mask, so both are pinned too;
- memory binding needs ``numactl --membind``, which cannot be done in-process,
  and a ``numactl`` re-exec is not implemented. It is recorded as not applied;
  Linux's first-touch allocation then favours the pinned node's memory, but
  nothing enforces it;
- the PostgreSQL containers receive ``--cpuset-cpus``/``--cpuset-mems`` for the
  same node only when the user's cgroup v2 delegation includes the ``cpuset``
  controller. They share the node's cores with the client, because the matrix
  runs one system at a time;
- ``--memory-max`` is recorded but not applied: this family does not re-exec
  under ``systemd-run``, and the containers keep their own podman limits.

The parsers take strings and the builders take tool availability as input,
so the whole decision is unit-testable on a host without ``/proc`` or
``/sys``.
"""
from __future__ import annotations

import glob
import os
import platform
import re
import sys
import time
from pathlib import Path
from typing import Callable

NOT_LINUX = "not applied: not Linux"
NO_PROC = "not applied: no /proc"
NOT_DELEGATED = "not applied: cpuset controller not delegated to the user"
LOAD_WAIT_STEP_S = 10
MEMORY_BINDING = (
    "not applied: in-process pinning (sched_setaffinity) binds CPUs only and a "
    "numactl --membind re-exec is not implemented; first-touch allocation "
    "favours the pinned node's memory but does not enforce it"
)
MEMORY_MAX = (
    "not applied to the database family: the client is not re-executed under "
    "systemd-run; the PostgreSQL containers keep their podman --memory limit"
)


# --- pure parsers -----------------------------------------------------------

def parse_cpulist(text: str) -> list[int]:
    """``"0-31,64-95"`` -> ``[0, ..., 31, 64, ..., 95]``."""
    cores: list[int] = []
    for part in text.strip().split(","):
        if not part:
            continue
        low, _, high = part.partition("-")
        cores.extend(range(int(low), int(high or low) + 1))
    return cores


def format_cpulist(cores: list[int]) -> str:
    ranges: list[str] = []
    ordered = sorted(set(cores))
    start = prev = None
    for core in ordered + [None]:
        if start is not None and (core is None or core != prev + 1):
            ranges.append(str(start) if start == prev else f"{start}-{prev}")
            start = None
        if core is not None and start is None:
            start = core
        prev = core
    return ",".join(ranges)


def parse_node_memfree_bytes(text: str) -> int | None:
    """``Node N MemFree: <kB> kB`` from a node's sysfs ``meminfo``."""
    match = re.search(r"MemFree:\s+(\d+)\s*kB", text)
    return int(match.group(1)) * 1024 if match else None


def choose_node(memfree: dict[int, int]) -> int | None:
    """The node with the most free memory; ties go to the lowest id."""
    if not memfree:
        return None
    return min(memfree, key=lambda node: (-memfree[node], node))


def parse_loadavg(text: str) -> tuple[float, int, int]:
    """``"0.52 0.58 0.59 2/1234 5678"`` -> ``(0.52, 2, 1234)``."""
    fields = text.split()
    runnable, total = fields[3].split("/")
    return float(fields[0]), int(runnable), int(total)


def parse_meminfo(text: str) -> dict[str, int]:
    """``/proc/meminfo`` as bytes per key (the kB figures times 1024)."""
    values: dict[str, int] = {}
    for line in text.splitlines():
        match = re.match(r"(\w+):\s+(\d+)\s*kB", line)
        if match:
            values[match.group(1)] = int(match.group(2)) * 1024
    return values


def node_load_share(load1: float, node_cores: int, total_cores: int) -> float:
    """The pinned node's share of the machine's one-minute load."""
    return load1 * node_cores / total_cores


def resolve_max_load(requested: str, node_cores: int | None) -> float | None:
    """``auto`` is half the pinned node's cores; ``off`` disables the gate."""
    if requested == "off":
        return None
    if requested == "auto":
        return node_cores / 2 if node_cores else None
    return float(requested)


def cpuset_delegated(controllers: str | None) -> bool:
    return controllers is not None and "cpuset" in controllers.split()


def podman_cpuset_args(cores: list[int] | None, node: int | None,
                       controllers: str | None) -> tuple[list[str], str]:
    """Container cpuset flags, or none plus the reason they were omitted."""
    if not cores or node is None:
        return [], "not applied: no NUMA node pinned"
    if controllers is None:
        return [], f"{NOT_DELEGATED} (no user cgroup v2 controllers file)"
    if not cpuset_delegated(controllers):
        return [], NOT_DELEGATED
    return [f"--cpuset-cpus={format_cpulist(cores)}", f"--cpuset-mems={node}"], "applied"


# --- the plan ---------------------------------------------------------------

def plan(*, numa_node: str, max_load: str, max_load_wait_s: float,
         memory_max: str | None, is_linux: bool, has_setaffinity: bool,
         node_cpus: dict[int, str], node_meminfo: dict[int, str],
         meminfo: str | None, controllers: str | None,
         all_cpus: list[int]) -> dict:
    """Resolve the ``isolation`` record from host facts given as strings."""
    record: dict = {
        "requested": {"numa_node": numa_node, "max_load": max_load,
                      "max_load_wait_s": max_load_wait_s, "memory_max": memory_max},
        "node": None, "node_reason": None, "cores": None,
        "total_cores": len(all_cpus),
        "client": {"cpu": None},
        "duckdb": "in-process: inherits the client's CPU affinity",
        "cityparquet_reader": "child process: inherits the client's CPU affinity",
        "memory_binding": NOT_LINUX,
        "containers": {"cpuset": NOT_LINUX, "args": [],
                       "sharing": "the node's cores are shared with the client; the matrix runs one system at a time"},
        "memory_max": "not requested" if memory_max is None else MEMORY_MAX,
        "max_load": None,
        "host_memory": NOT_LINUX,
        "load": {"recording": NO_PROC, "granularity": "per cell, before and after each system's cell"},
    }
    if not is_linux:
        record["client"]["cpu"] = NOT_LINUX
        record["duckdb"] = record["cityparquet_reader"] = NOT_LINUX
        return record

    if meminfo is not None:
        values = parse_meminfo(meminfo)
        record["host_memory"] = {"mem_total_bytes": values.get("MemTotal"),
                                 "mem_available_bytes": values.get("MemAvailable")}
        record["load"]["recording"] = "applied"
    else:
        record["host_memory"] = NO_PROC

    topology = {node: parse_cpulist(text) for node, text in node_cpus.items()}
    if not topology:
        topology = {0: list(all_cpus)}
    node: int | None = None
    if numa_node == "off":
        reason = "not applied: --numa-node off"
    elif numa_node == "auto":
        memfree = {n: parse_node_memfree_bytes(t) for n, t in node_meminfo.items()}
        node = choose_node({n: v for n, v in memfree.items() if v is not None and n in topology})
        if node is None:
            node, record["node_reason"] = min(topology), "auto: no node MemFree information"
        else:
            record["node_reason"] = "auto: most MemFree at run start"
        reason = None
    else:
        node = int(numa_node)
        if node not in topology:
            node, reason = None, f"not applied: NUMA node {numa_node} not present"
        else:
            record["node_reason"], reason = "requested", None

    record["memory_binding"] = MEMORY_BINDING
    if node is None:
        record["client"]["cpu"] = reason
        record["containers"]["cpuset"] = reason
        record["max_load"] = resolve_max_load(max_load, None)
        return record

    cores = topology[node]
    record["node"], record["cores"] = node, format_cpulist(cores)
    record["client"]["cpu"] = (
        "pending" if has_setaffinity else "not applied: os.sched_setaffinity unavailable"
    )
    args, cpuset = podman_cpuset_args(cores, node, controllers)
    record["containers"]["args"], record["containers"]["cpuset"] = args, cpuset
    record["max_load"] = resolve_max_load(max_load, len(cores))
    return record


def apply_client(record: dict, setaffinity: Callable | None = None) -> None:
    """Pin this process (and so DuckDB) to the planned node's cores."""
    if record["client"]["cpu"] != "pending":
        return
    setaffinity = setaffinity or getattr(os, "sched_setaffinity")
    cores = parse_cpulist(record["cores"])
    try:
        setaffinity(0, cores)
        record["client"]["cpu"] = f"applied: sched_setaffinity to node {record['node']} cores {record['cores']}"
    except Exception as exc:  # never abort on an isolation failure
        record["client"]["cpu"] = f"not applied: {type(exc).__name__}: {exc}"


# --- host readers (thin, impure) --------------------------------------------

def _read(path: str | Path) -> str | None:
    try:
        return Path(path).read_text()
    except OSError:
        return None


def read_loadavg() -> str | None:
    return _read("/proc/loadavg")


def read_meminfo() -> str | None:
    return _read("/proc/meminfo")


def host_facts() -> dict:
    """The string inputs ``plan`` needs, read from this host."""
    nodes: dict[int, str] = {}
    meminfo: dict[int, str] = {}
    for directory in glob.glob("/sys/devices/system/node/node[0-9]*"):
        node = int(directory.rsplit("node", 1)[1])
        cpus = _read(f"{directory}/cpulist")
        if cpus is not None:
            nodes[node] = cpus
        mem = _read(f"{directory}/meminfo")
        if mem is not None:
            meminfo[node] = mem
    uid = os.getuid() if hasattr(os, "getuid") else 0
    controllers = _read(
        f"/sys/fs/cgroup/user.slice/user-{uid}.slice/user@{uid}.service/cgroup.controllers"
    )
    try:
        all_cpus = sorted(os.sched_getaffinity(0))
    except AttributeError:
        all_cpus = list(range(os.cpu_count() or 1))
    return {
        "is_linux": platform.system() == "Linux",
        "has_setaffinity": hasattr(os, "sched_setaffinity"),
        "node_cpus": nodes, "node_meminfo": meminfo,
        "meminfo": read_meminfo(), "controllers": controllers, "all_cpus": all_cpus,
    }


def setup(*, numa_node: str, max_load: str, max_load_wait_s: float,
          memory_max: str | None) -> tuple[dict, "LoadGate"]:
    """Plan from this host, pin the client, and build the run's load gate."""
    facts = host_facts()
    record = plan(numa_node=numa_node, max_load=max_load,
                  max_load_wait_s=max_load_wait_s, memory_max=memory_max, **facts)
    try:
        apply_client(record)
    except Exception as exc:
        record["client"]["cpu"] = f"not applied: {type(exc).__name__}: {exc}"
    node_cores = len(parse_cpulist(record["cores"])) if record["cores"] else record["total_cores"]
    gate = LoadGate(
        max_load=record["max_load"], node_cores=node_cores,
        total_cores=max(record["total_cores"], 1), wait_s=max_load_wait_s,
        read_loadavg=read_loadavg if record["load"]["recording"] == "applied" else (lambda: None),
        read_meminfo=read_meminfo,
    )
    return record, gate


# --- the load gate ----------------------------------------------------------

class LoadGate:
    """Wait for a quiet node before each cell; record load around it."""

    def __init__(self, *, max_load: float | None, node_cores: int, total_cores: int,
                 wait_s: float, read_loadavg: Callable[[], str | None],
                 read_meminfo: Callable[[], str | None],
                 sleep: Callable[[float], object] = time.sleep,
                 log: Callable[[str], object] = lambda m: print(m, file=sys.stderr),
                 step_s: float = LOAD_WAIT_STEP_S) -> None:
        self.max_load, self.node_cores, self.total_cores = max_load, node_cores, total_cores
        self.wait_s, self.step_s = wait_s, step_s
        self._loadavg, self._meminfo, self._sleep, self._log = read_loadavg, read_meminfo, sleep, log
        self.cells: list[dict] = []
        self._current: dict | None = None

    def _sample(self) -> tuple[float, int, int | None] | None:
        text = self._loadavg()
        if text is None:
            return None
        load1, runnable, _ = parse_loadavg(text)
        mem = self._meminfo()
        available = parse_meminfo(mem).get("MemAvailable") if mem else None
        return load1, runnable, available

    def _share(self, load1: float) -> float:
        return node_load_share(load1, self.node_cores, self.total_cores)

    def before_cell(self, label: str) -> bool:
        """Wait while the node is loaded; True if the cell proceeds busy."""
        sample = self._sample()
        if sample is None:
            self._current = None
            return False
        waited = 0.0
        busy = False
        if self.max_load is not None:
            while self._share(sample[0]) > self.max_load:
                if waited >= self.wait_s:
                    busy = True
                    self._log(f"load: {label} proceeds busy (node share {self._share(sample[0]):.1f} > {self.max_load})")
                    break
                self._log(f"load: node share {self._share(sample[0]):.1f} > {self.max_load}; waiting {self.step_s} s before {label}")
                self._sleep(self.step_s)
                waited += self.step_s
                sample = self._sample()
        self._current = {"label": label, "samples": [sample], "waited_s": waited, "busy": busy}
        return busy

    def after_cell(self) -> None:
        if self._current is None:
            return
        sample = self._sample()
        samples = self._current.pop("samples") + ([sample] if sample else [])
        available = [s[2] for s in samples if s[2] is not None]
        self._current.update(
            load1_max=max(s[0] for s in samples),
            runnable_max=max(s[1] for s in samples),
            mem_available_min_bytes=min(available) if available else None,
        )
        self.cells.append(self._current)
        self._current = None

    def summary(self) -> dict:
        if not self.cells:
            return {"load1_max": None, "runnable_max": None,
                    "mem_available_min_bytes": None, "busy_cells": 0}
        available = [c["mem_available_min_bytes"] for c in self.cells
                     if c["mem_available_min_bytes"] is not None]
        return {
            "load1_max": max(c["load1_max"] for c in self.cells),
            "runnable_max": max(c["runnable_max"] for c in self.cells),
            "mem_available_min_bytes": min(available) if available else None,
            "busy_cells": sum(1 for c in self.cells if c["busy"]),
        }
