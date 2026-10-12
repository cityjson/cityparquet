"""Summary statistics for benchmark timings.

``report.py`` reports the seven-column timing block from ``timing_summary``:
the arithmetic mean (``time_mean_s``), the population standard deviation
(``time_std_s``), the median, the range and the quartiles. ``mad`` (median
absolute deviation about the median) is available as a further robust
estimate.
"""

import statistics
import os
import threading
import subprocess

from citybench import engine
from collections.abc import Callable
from typing import TypeVar

T = TypeVar("T")


def resident_bytes(pid: int) -> int | None:
    """Current Linux resident set size for ``pid``, in bytes."""
    try:
        with open(f"/proc/{pid}/status") as status:
            for line in status:
                if line.startswith("VmRSS:"):
                    return int(line.split()[1]) * 1024
    except (FileNotFoundError, PermissionError, ValueError):
        return None
    return None


def host_pid_from_engine_top(container: str, container_pid: int) -> int | None:
    """Return one PostgreSQL host PID from Podman's container-scoped ``top``
    table (``hpid`` is Podman's spelling; other engines return ``None``)."""
    active = engine.active()
    if active.name != "podman" or active.host_proc_gap():
        return None
    try:
        rows = subprocess.check_output(
            active.cmd("top", container, "hpid", "pid", "comm"),
            text=True, stderr=subprocess.DEVNULL,
        ).splitlines()[1:]
        matches = [line.split()[0] for line in rows if len(line.split()) >= 3
                   and line.split()[1] == str(container_pid)
                   and "postgres" in line.split()[2].lower()]
        return int(matches[0]) if len(matches) == 1 else None
    except (OSError, subprocess.SubprocessError, ValueError):
        return None


def container_init_host_pid(container: str) -> int | None:
    """Host PID of a verified container init, or ``None`` (always ``None``
    when the engine runs a virtual machine: its PIDs are not host PIDs)."""
    active = engine.active()
    if active.host_proc_gap():
        return None
    try:
        output = subprocess.check_output(
            active.cmd("inspect", "--format", "{{.State.Pid}}", container),
            text=True, stderr=subprocess.DEVNULL,
        ).strip()
        return int(output) if int(output) > 0 else None
    except (OSError, subprocess.SubprocessError, ValueError):
        return None


def host_pid_for_namespace_pid(namespace_pid: int, *, container_init_pid: int | None) -> int | None:
    """Map a container PID only within that container's verified PID namespace."""
    if container_init_pid is None:
        return None
    try:
        namespace = os.readlink(f"/proc/{container_init_pid}/ns/pid")
    except OSError:
        return None
    matches: list[int] = []
    for entry in os.scandir("/proc"):
        if not entry.name.isdecimal():
            continue
        try:
            if os.readlink(f"{entry.path}/ns/pid") != namespace:
                continue
            status = open(f"{entry.path}/status").read().splitlines()
            nspid = next(line for line in status if line.startswith("NSpid:"))
            name = next(line for line in status if line.startswith("Name:"))
            if int(nspid.split()[-1]) == namespace_pid and "postgres" in name.lower():
                matches.append(int(entry.name))
        except (FileNotFoundError, PermissionError, ValueError, StopIteration, OSError):
            continue
    return matches[0] if len(matches) == 1 else None


def peak_resident_bytes(call: Callable[[], T], *, pid: int | None = None) -> tuple[T, int | None]:
    """Run ``call`` while sampling one process's resident memory.

    PostgreSQL queries are sampled at the backend PID, not at the Python
    client. ``None`` means the host did not expose procfs; it is never a zero.
    """
    target = os.getpid() if pid is None else pid
    samples: list[int] = []
    stop = threading.Event()

    def sample() -> None:
        while not stop.is_set():
            value = resident_bytes(target)
            if value is not None:
                samples.append(value)
            stop.wait(0.005)

    thread = threading.Thread(target=sample, daemon=True)
    thread.start()
    try:
        result = call()
    finally:
        stop.set()
        thread.join()
    value = resident_bytes(target)
    if value is not None:
        samples.append(value)
    return result, max(samples) if samples else None


def mean(values: list[float]) -> float:
    """Arithmetic mean of ``values``. Raises ValueError if empty."""
    if not values:
        raise ValueError("mean requires at least one value")
    return statistics.mean(values)


def standard_deviation(values: list[float]) -> float:
    """Population standard deviation of ``values``. Raises ValueError if empty.

    Population rather than sample: the timed samples are the whole set that
    was measured, not a draw used to infer a wider population.
    """
    if not values:
        raise ValueError("standard_deviation requires at least one value")
    return statistics.pstdev(values)


def median(values: list[float]) -> float:
    """Median of ``values``. Raises ValueError if empty."""
    if not values:
        raise ValueError("median requires at least one value")
    return statistics.median(values)


def mad(values: list[float]) -> float:
    """Median absolute deviation from the median. Raises ValueError if empty."""
    if not values:
        raise ValueError("mad requires at least one value")
    centre = statistics.median(values)
    return statistics.median([abs(v - centre) for v in values])


def quantile(values: list[float], p: float) -> float:
    """The ``p`` quantile of ``values`` (0 <= p <= 1). Raises ValueError if empty.

    Definition: linear interpolation at position ``p * (n - 1)`` on the sorted
    samples -- numpy's default, and ``statistics.quantiles(method="inclusive")``.
    So the median (p = 0.5) of an even count is the mean of the two middle
    values. The readbench harness (``benchmark/readbench/src/stats.rs``) uses
    the identical definition, so both families' quartiles are the same statistic.
    """
    if not values:
        raise ValueError("quantile requires at least one value")
    ordered = sorted(values)
    position = p * (len(ordered) - 1)
    lower = int(position)
    upper = min(lower + 1, len(ordered) - 1)
    fraction = position - lower
    return ordered[lower] + (ordered[upper] - ordered[lower]) * fraction


def timing_summary(values: list[float]) -> dict[str, float]:
    """The seven timing statistics of ``values``, in the CSV block's order.

    Keys ``mean, std, median, min, max, q1, q3``: arithmetic mean, population
    standard deviation, and the median and quartiles by :func:`quantile`'s
    linear-interpolation definition. Raises ValueError if empty.
    """
    if not values:
        raise ValueError("timing_summary requires at least one value")
    return {
        "mean": mean(values),
        "std": standard_deviation(values),
        "median": quantile(values, 0.5),
        "min": min(values),
        "max": max(values),
        "q1": quantile(values, 0.25),
        "q3": quantile(values, 0.75),
    }
