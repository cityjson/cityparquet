"""Working-memory sampling: /proc parsing, worker discovery, aggregation, sources."""

import subprocess

from citybench import engine, memory

STATUS = """Name:\tpostgres
VmRSS:\t  9000 kB
RssAnon:\t  1200 kB
RssFile:\t   300 kB
RssShmem:\t  7500 kB
"""


def test_parse_status_reads_rssanon_rssshmem_and_vmrss_in_bytes():
    s = memory.parse_status(STATUS)
    assert s == {"RssAnon": 1200 * 1024, "RssShmem": 7500 * 1024, "VmRSS": 9000 * 1024}


def test_parse_multi_status_splits_per_pid():
    text = f"== 11\n{STATUS}== 12\n{STATUS.replace('1200', '800')}"
    out = memory.parse_multi_status(text)
    assert out[11]["RssAnon"] == 1200 * 1024 and out[12]["RssAnon"] == 800 * 1024


def test_worker_sql_finds_parallel_workers_by_leader_pid():
    assert "pg_stat_activity" in memory.WORKER_SQL
    assert "leader_pid = %s" in memory.WORKER_SQL


def test_peak_sums_leader_and_workers_per_instant_then_takes_max():
    snaps = iter([
        {1: {"RssAnon": 10, "VmRSS": 100}},
        {1: {"RssAnon": 30, "VmRSS": 120}, 2: {"RssAnon": 25, "VmRSS": 90}},
        {1: {"RssAnon": 40, "VmRSS": 130}},
    ])
    peak = memory.Peak()
    for snap in snaps:
        peak.add(snap)
    assert peak.anon == 55 and peak.rss == 210 and peak.samples == 3


def test_sample_runs_call_and_reports_peak_and_samples():
    reads = []

    def source(pids):
        reads.append(tuple(pids))
        return {p: {"RssAnon": 7, "VmRSS": 9} for p in pids}

    result, peak = memory.sample(lambda: "done", lambda: [5], source, interval_s=0.001)
    assert result == "done" and peak.anon == 7 and peak.samples >= 2  # start and end
    assert all(r == (5,) for r in reads)


def test_exec_source_reads_status_through_one_engine_exec():
    seen = []

    def runner(argv):
        seen.append(argv)
        return subprocess.CompletedProcess(argv, 0, stdout=f"== 7\n{STATUS}", stderr="")

    eng = engine.Engine(name="container", binary="container", version="1.0", run_flags=(), platform="darwin")
    src = memory.exec_source(eng, "citybench-cjdb-x", runner=runner)
    assert src([7])[7]["RssAnon"] == 1200 * 1024
    assert seen[0][:3] == ["container", "exec", "citybench-cjdb-x"]


def test_exec_source_failure_yields_no_reading_not_zero():
    def runner(argv):
        return subprocess.CompletedProcess(argv, 1, stdout="", stderr="boom")

    eng = engine.Engine(name="container", binary="container", version="1.0", run_flags=(), platform="darwin")
    assert memory.exec_source(eng, "c", runner=runner)([7]) == {}
    peak = memory.Peak()
    peak.add({})
    assert peak.anon is None


def test_choose_pg_source_without_container_says_not_applied():
    src, reason = memory.choose_pg_source(container=None)
    assert src is None and reason.startswith("not applied:")
