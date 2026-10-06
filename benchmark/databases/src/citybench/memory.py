"""Working memory of the process(es) executing a query.

The measure is ``RssAnon`` from ``/proc/<pid>/status``: the anonymous resident
pages a process owns (heap, sort and hash memory, ``work_mem``). It excludes
``RssShmem``, where PostgreSQL's ``shared_buffers`` land once a backend has
touched them, and ``RssFile`` (mapped binaries, OS page cache). Under a
parallel plan the leader's and its workers' readings are summed at each
sampling instant, and the peak of those sums is reported.

The status files are read where the engine exposes them: the host ``/proc``
for an engine whose containers run on the host kernel (rootless podman on
Linux), or one ``exec`` into the container per sampling instant on an engine
that runs a virtual machine (Apple ``container``, docker on macOS). A failed
read is no reading, never a zero; a measurement with no readings is blank.
"""

from __future__ import annotations

import os
import subprocess
import threading
from collections.abc import Callable, Iterable
from dataclasses import dataclass
from pathlib import Path
from typing import TypeVar

from citybench import engine as engine_mod

T = TypeVar("T")
Status = dict[str, int]
Source = Callable[[Iterable[int]], dict[int, Status]]

_KEYS = ("RssAnon", "RssShmem", "VmRSS")

# Parallel workers of a query report their leader's PID (PostgreSQL >= 13).
WORKER_SQL = "SELECT pid FROM pg_stat_activity WHERE leader_pid = %s"

# Host procfs reads cost microseconds; an engine exec costs tens of
# milliseconds, so it bounds the sampling rate on its own.
HOST_INTERVAL_S = 0.005
EXEC_INTERVAL_S = 0.05


def parse_status(text: str) -> Status:
    """``RssAnon``, ``RssShmem`` and ``VmRSS`` of one status file, in bytes."""
    out: Status = {}
    for line in text.splitlines():
        key, _, rest = line.partition(":")
        if key in _KEYS and rest.split():
            out[key] = int(rest.split()[0]) * 1024
    return out


def parse_multi_status(text: str) -> dict[int, Status]:
    """Status files concatenated as ``== <pid>`` blocks, one per process."""
    out: dict[int, Status] = {}
    pid, lines = None, []
    for line in text.splitlines() + ["== end"]:
        if line.startswith("== "):
            if pid is not None and lines:
                parsed = parse_status("\n".join(lines))
                if parsed:
                    out[pid] = parsed
            token = line[3:].strip()
            pid, lines = (int(token) if token.isdigit() else None), []
        else:
            lines.append(line)
    return out


@dataclass
class Peak:
    """Peak over sampling instants of the per-instant sum across processes."""
    anon: int | None = None
    rss: int | None = None
    samples: int = 0

    def add(self, snapshot: dict[int, Status]) -> None:
        self.samples += 1
        anon = [s["RssAnon"] for s in snapshot.values() if "RssAnon" in s]
        rss = [s["VmRSS"] for s in snapshot.values() if "VmRSS" in s]
        if anon:
            self.anon = max(self.anon or 0, sum(anon))
        if rss:
            self.rss = max(self.rss or 0, sum(rss))


def sample(call: Callable[[], T], pids: Callable[[], Iterable[int]], source: Source,
           *, interval_s: float = HOST_INTERVAL_S) -> tuple[T, Peak]:
    """Run ``call`` while a separate thread samples ``source`` for ``pids``.

    One reading is taken before and one after the call, so a query shorter
    than the interval still records the backend at its edges (a lower bound
    of its working memory, not a zero).
    """
    peak = Peak()
    stop = threading.Event()

    def snap() -> None:
        try:
            peak.add(source(list(pids())))
        except Exception:  # a failed read is no reading
            peak.add({})

    def loop() -> None:
        while not stop.is_set():
            snap()
            stop.wait(interval_s)

    snap()
    thread = threading.Thread(target=loop, daemon=True)
    thread.start()
    try:
        result = call()
    finally:
        stop.set()
        thread.join()
    snap()
    return result, peak


def host_source(to_host: Callable[[int], int | None]) -> Source:
    """Read ``/proc/<host pid>/status`` on the host, mapping each PID first."""
    def read(pids: Iterable[int]) -> dict[int, Status]:
        out: dict[int, Status] = {}
        for pid in pids:
            host = to_host(pid)
            if host is None:
                continue
            try:
                out[pid] = parse_status(Path(f"/proc/{host}/status").read_text())
            except OSError:
                continue
        return out
    return read


def exec_source(eng: engine_mod.Engine, container: str, *,
                runner=lambda argv: subprocess.run(argv, capture_output=True, text=True)) -> Source:
    """Read every PID's status file with ONE ``exec`` into the container."""
    def read(pids: Iterable[int]) -> dict[int, Status]:
        script = "".join(f'echo "== {int(p)}"; cat /proc/{int(p)}/status 2>/dev/null; ' for p in pids)
        result = runner(eng.exec_args(container, "sh", "-c", script))
        return parse_multi_status(result.stdout) if result.returncode == 0 else {}
    return read


def self_source() -> Source | None:
    """This process's own status file, or ``None`` without procfs (macOS)."""
    if not Path("/proc/self/status").exists():
        return None
    pid = os.getpid()
    return lambda _pids: {pid: parse_status(Path("/proc/self/status").read_text())}


def choose_pg_source(*, container: str | None, to_host: Callable[[int], int | None] | None = None,
                     eng: engine_mod.Engine | None = None) -> tuple[Source | None, str]:
    """The status-file reader for a PostgreSQL container, and how it reads."""
    if container is None:
        return None, "not applied: no container is known for this connection"
    eng = eng or engine_mod.active()
    if eng.host_proc_gap() is None and to_host is not None:
        return host_source(to_host), "host /proc"
    return exec_source(eng, container), f"{eng.name} exec"
