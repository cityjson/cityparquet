"""Robust summary statistics for benchmark timings.

The median and median absolute deviation are used rather than mean and
standard deviation because a benchmark sample set routinely contains
outliers from OS scheduling and background load, and the median is not
dragged by them.
"""

import statistics
import os
import threading
import subprocess
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


def host_pid_from_podman(container: str, container_pid: int) -> int | None:
    """Return one PostgreSQL host PID from Podman's container-scoped table."""
    try:
        rows = subprocess.check_output(
            ["podman", "top", container, "hpid", "pid", "comm"],
            text=True, stderr=subprocess.DEVNULL,
        ).splitlines()[1:]
        matches = [line.split()[0] for line in rows if len(line.split()) >= 3
                   and line.split()[1] == str(container_pid)
                   and "postgres" in line.split()[2].lower()]
        return int(matches[0]) if len(matches) == 1 else None
    except (OSError, subprocess.SubprocessError, ValueError):
        return None


def container_init_host_pid(container: str) -> int | None:
    """Host PID of a verified Podman container init, or ``None``."""
    try:
        output = subprocess.check_output(
            ["podman", "inspect", "--format", "{{.State.Pid}}", container],
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
