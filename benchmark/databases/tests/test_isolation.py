"""Host isolation: parsers take strings, builders take tool availability."""
from __future__ import annotations

import pytest

from citybench import isolation as iso

NODE_CPUS = {0: "0-31,64-95\n", 1: "32-63,96-127\n"}
NODE_MEMINFO = {
    0: "Node 0 MemTotal:  270000000 kB\nNode 0 MemFree:   1000 kB\n",
    1: "Node 1 MemTotal:  270000000 kB\nNode 1 MemFree:   2000 kB\n",
}
MEMINFO = "MemTotal:       540000000 kB\nMemFree:  100 kB\nMemAvailable:   400000000 kB\n"
LOADAVG = "0.52 0.58 0.59 2/1234 5678\n"
CONTROLLERS_WITH = "cpuset cpu io memory pids\n"
CONTROLLERS_WITHOUT = "cpu io memory pids\n"


def test_parse_cpulist_ranges_and_singletons():
    assert iso.parse_cpulist("0-3,8,10-11\n") == [0, 1, 2, 3, 8, 10, 11]
    assert iso.parse_cpulist("") == []


def test_format_cpulist_round_trips_ranges():
    assert iso.format_cpulist([0, 1, 2, 3, 8, 10, 11]) == "0-3,8,10-11"


def test_parse_node_memfree_returns_bytes():
    assert iso.parse_node_memfree_bytes(NODE_MEMINFO[1]) == 2000 * 1024
    assert iso.parse_node_memfree_bytes("garbage") is None


def test_choose_node_most_memfree_ties_lowest():
    assert iso.choose_node({0: 5, 1: 9}) == 1
    assert iso.choose_node({3: 7, 1: 7}) == 1
    assert iso.choose_node({}) is None


def test_parse_loadavg():
    assert iso.parse_loadavg(LOADAVG) == (0.52, 2, 1234)


def test_parse_meminfo_bytes():
    m = iso.parse_meminfo(MEMINFO)
    assert m["MemTotal"] == 540000000 * 1024
    assert m["MemAvailable"] == 400000000 * 1024


def test_node_load_share_scales_by_node_fraction():
    assert iso.node_load_share(64.0, 64, 128) == 32.0


def test_resolve_max_load():
    assert iso.resolve_max_load("auto", 32) == 16.0
    assert iso.resolve_max_load("off", 32) is None
    assert iso.resolve_max_load("3.5", 32) == 3.5
    assert iso.resolve_max_load("auto", None) is None


def test_podman_cpuset_args_when_delegated():
    args, record = iso.podman_cpuset_args([0, 1, 2, 5], 1, CONTROLLERS_WITH)
    assert args == ["--cpuset-cpus=0-2,5", "--cpuset-mems=1"]
    assert record == "applied"


def test_podman_cpuset_args_not_delegated_omits_flags():
    args, record = iso.podman_cpuset_args([0, 1], 0, CONTROLLERS_WITHOUT)
    assert args == []
    assert record == "not applied: cpuset controller not delegated to the user"


def test_podman_cpuset_args_missing_controllers_file():
    args, record = iso.podman_cpuset_args([0, 1], 0, None)
    assert args == []
    assert record.startswith("not applied: cpuset controller not delegated to the user")


def _plan(**overrides):
    kwargs = dict(
        numa_node="auto", max_load="auto", max_load_wait_s=600, memory_max=None,
        is_linux=True, has_setaffinity=True, node_cpus=NODE_CPUS,
        node_meminfo=NODE_MEMINFO, meminfo=MEMINFO, controllers=CONTROLLERS_WITH,
        all_cpus=list(range(128)),
    )
    kwargs.update(overrides)
    return iso.plan(**kwargs)


def test_plan_auto_picks_node_with_most_memfree():
    record = _plan()
    assert record["node"] == 1
    assert record["cores"] == "32-63,96-127"
    assert record["max_load"] == 32.0
    assert record["containers"]["cpuset"] == "applied"
    assert record["containers"]["args"] == ["--cpuset-cpus=32-63,96-127", "--cpuset-mems=1"]
    assert record["host_memory"]["mem_total_bytes"] == 540000000 * 1024
    assert record["memory_binding"].startswith("not applied:")


def test_plan_explicit_node_and_off():
    assert _plan(numa_node="0")["node"] == 0
    off = _plan(numa_node="off")
    assert off["node"] is None
    assert off["client"]["cpu"] == "not applied: --numa-node off"
    assert off["containers"]["args"] == []


def test_plan_missing_node_is_recorded_not_raised():
    record = _plan(numa_node="7")
    assert record["node"] is None
    assert record["client"]["cpu"] == "not applied: NUMA node 7 not present"


def test_plan_without_numa_info_falls_back_to_node_zero_all_cpus():
    record = _plan(node_cpus={}, node_meminfo={}, all_cpus=[0, 1, 2, 3])
    assert record["node"] == 0
    assert record["cores"] == "0-3"


def test_plan_memory_max_is_recorded_not_applied():
    assert _plan(memory_max="8000000000")["memory_max"].startswith(
        "not applied to the database family"
    )
    assert _plan()["memory_max"] == "not requested"


def test_plan_on_macos_records_not_linux_everywhere():
    record = _plan(is_linux=False, has_setaffinity=False, node_cpus={},
                   node_meminfo={}, meminfo=None, controllers=None, all_cpus=[0, 1])
    assert record["node"] is None
    assert record["client"]["cpu"] == iso.NOT_LINUX
    assert record["duckdb"] == iso.NOT_LINUX
    assert record["containers"]["cpuset"] == iso.NOT_LINUX
    assert record["host_memory"] == iso.NOT_LINUX
    assert record["load"]["recording"] == "not applied: no /proc"
    assert record["max_load"] is None


def test_apply_client_pins_and_records():
    record = _plan()
    calls = []
    iso.apply_client(record, setaffinity=lambda pid, cores: calls.append((pid, set(cores))))
    assert calls == [(0, set(range(32, 64)) | set(range(96, 128)))]
    assert record["client"]["cpu"].startswith("applied")


def test_apply_client_failure_is_recorded():
    record = _plan()

    def boom(pid, cores):
        raise OSError("EINVAL")

    iso.apply_client(record, setaffinity=boom)
    assert record["client"]["cpu"] == "not applied: OSError: EINVAL"


class FakeHost:
    def __init__(self, loads):
        self.loads = list(loads)
        self.slept = []

    def loadavg(self):
        load = self.loads.pop(0) if len(self.loads) > 1 else self.loads[0]
        return f"{load} 0 0 3/900 1\n"

    def meminfo(self):
        return MEMINFO


def _gate(host, max_load=16.0, wait_s=30):
    return iso.LoadGate(
        max_load=max_load, node_cores=64, total_cores=128, wait_s=wait_s,
        read_loadavg=host.loadavg, read_meminfo=host.meminfo,
        sleep=host.slept.append, log=lambda message: None,
    )


def test_gate_passes_immediately_under_threshold():
    host = FakeHost([10.0])
    gate = _gate(host)
    assert gate.before_cell("c") is False
    gate.after_cell()
    assert host.slept == []
    cell = gate.cells[0]
    assert cell["label"] == "c" and cell["load1_max"] == 10.0
    assert cell["runnable_max"] == 3
    assert cell["mem_available_min_bytes"] == 400000000 * 1024
    assert cell["busy"] is False


def test_gate_waits_then_proceeds():
    # share = load1 * 64 / 128: 40 -> 20 (above 16), 20 -> 10 (below)
    host = FakeHost([40.0, 20.0])
    gate = _gate(host)
    assert gate.before_cell("c") is False
    assert host.slept == [10]
    gate.after_cell()
    assert gate.cells[0]["waited_s"] == 10


def test_gate_gives_up_after_wait_and_flags_busy():
    host = FakeHost([40.0])
    gate = _gate(host, wait_s=30)
    assert gate.before_cell("c") is True
    assert host.slept == [10, 10, 10]
    gate.after_cell()
    assert gate.summary()["busy_cells"] == 1
    assert gate.summary()["load1_max"] == 40.0


def test_gate_disabled_without_proc_records_nothing():
    gate = iso.LoadGate(
        max_load=16.0, node_cores=64, total_cores=128, wait_s=30,
        read_loadavg=lambda: None, read_meminfo=lambda: None,
        sleep=lambda s: pytest.fail("must not sleep"), log=lambda m: None,
    )
    assert gate.before_cell("c") is False
    gate.after_cell()
    assert gate.cells == []
    assert gate.summary() == {"load1_max": None, "runnable_max": None,
                              "mem_available_min_bytes": None, "busy_cells": 0}


def test_gate_without_threshold_records_but_never_waits():
    host = FakeHost([400.0])
    gate = _gate(host, max_load=None)
    assert gate.before_cell("c") is False
    assert host.slept == []
