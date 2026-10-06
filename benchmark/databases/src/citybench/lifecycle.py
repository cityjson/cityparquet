"""Isolated, disposable PostgreSQL containers for one benchmark run.

Every container call goes through ``citybench.engine`` (Apple ``container``,
``docker`` or ``podman``, chosen by its cascade)."""
from __future__ import annotations

import os
import shutil
import subprocess
import tempfile
import time
import uuid
from contextlib import contextmanager
from pathlib import Path

from citybench import engine as container_engine

DATABASES_DIR = Path(__file__).resolve().parents[2]
ROOT = (DATABASES_DIR.parent / "runs").resolve()
CJDB_IMAGE = "docker.io/postgis/postgis:16-3.4"
CITYDB_IMAGE = "docker.io/3dcitydb/3dcitydb-pg:16-3.4-5.1.2-alpine"
POSTGRES_CONF = Path(__file__).resolve().parents[2] / "docker" / "postgresql.conf"
CPU_LIMIT = "16"
MEMORY_LIMIT = "32g"
SHM_SIZE = "2g"
DEFAULT_TMPDIR = Path("/data2/hideba/tmp")


def runs_root() -> Path:
    """The directory every data root must lie below: ``benchmark/runs/``,
    or ``CITYBENCH_RUNS_ROOT`` when set (a scratch run outside the
    repository)."""
    override = os.environ.get("CITYBENCH_RUNS_ROOT")
    return Path(override).resolve() if override else ROOT


def benchmark_temp_directory() -> Path:
    """Return this process's benchmark temporary directory.

    ``isolated_databases`` supplies a UUID-scoped child of ``TMPDIR``. The
    explicit fallback keeps adapters outside that context off the operating
    system's default temporary directory too.
    """
    return Path(
        os.environ.get("CITYBENCH_TEMP_DIR")
        or os.environ.get("TMPDIR")
        or DEFAULT_TMPDIR
    ).resolve()


def citydb_tool_temp_directory() -> Path:
    return Path(os.environ.get("CITYBENCH_CITYDB_TOOL_TMPDIR") or benchmark_temp_directory()).resolve()


def duckdb_temp_directory() -> Path:
    return Path(os.environ.get("CITYBENCH_DUCKDB_TMPDIR") or benchmark_temp_directory()).resolve()


def _new_temp_directory() -> Path:
    base = Path(os.environ.get("TMPDIR") or DEFAULT_TMPDIR).resolve()
    base.mkdir(parents=True, exist_ok=True)
    directory = Path(tempfile.mkdtemp(prefix="citybench-", dir=base))
    # Containers run as mapped users, so their shared /tmp needs the normal
    # world-writable sticky-bit semantics. This only changes our fresh child.
    directory.chmod(0o1777)
    return directory


def _new_temp_child(parent: Path, name: str) -> Path:
    directory = parent / name
    directory.mkdir()
    directory.chmod(0o1777)
    return directory


def _run(*args: str, capture: bool = False) -> str:
    return subprocess.run(args, check=True, text=True, capture_output=capture).stdout if capture else (subprocess.run(args, check=True), "")[1]


def _wait(port: int, *, citydb: bool = False) -> None:
    deadline = time.monotonic() + 180
    while time.monotonic() < deadline:
        try:
            import psycopg
            conn = psycopg.connect(host="127.0.0.1", port=port, dbname="bench", user="bench", password="bench", connect_timeout=1)
            if citydb:
                with conn.cursor() as cur:
                    cur.execute("SELECT count(*) FROM information_schema.tables WHERE table_schema='citydb' AND table_name IN ('feature','property','geometry_data','objectclass')")
                    ready = cur.fetchone()[0] == 4
                conn.close()
                if not ready:
                    time.sleep(1); continue
            else:
                conn.close()
            return
        except Exception:
            time.sleep(1)
    raise TimeoutError(f"database on port {port} did not become ready")


@contextmanager
def isolated_databases(data_root: Path, srid: int, *, container_args: list[str] | None = None):
    """Start both PostgreSQL containers; ``container_args`` (the run's
    ``--cpuset-cpus``/``--cpuset-mems`` flags, when the cpuset controller is
    delegated) are added to each container ``run``."""
    root = data_root.resolve()
    allowed = runs_root()
    if root != allowed and allowed not in root.parents:
        raise ValueError(f"data root must be below {allowed}")
    run_id = uuid.uuid4().hex
    run_root = root / "databases" / run_id
    run_root.mkdir(parents=True)
    temp_root = _new_temp_directory()
    temp_dirs = {
        "cjdb": _new_temp_child(temp_root, "cjdb"),
        "3dcitydb": _new_temp_child(temp_root, "3dcitydb"),
        "citydb-tool": _new_temp_child(temp_root, "citydb-tool"),
        "duckdb": _new_temp_child(temp_root, "duckdb"),
    }
    temp_environment = {
        "CITYBENCH_TEMP_DIR": str(temp_root),
        "CITYBENCH_CITYDB_TOOL_TMPDIR": str(temp_dirs["citydb-tool"]),
        "CITYBENCH_DUCKDB_TMPDIR": str(temp_dirs["duckdb"]),
    }
    previous_temp_environment = {name: os.environ.get(name) for name in temp_environment}
    os.environ.update(temp_environment)
    names = {"cjdb": f"citybench-cjdb-{run_id}", "3dcitydb": f"citybench-citydb-{run_id}"}
    engine = container_engine.active()
    extra = list(container_args or [])
    ports = {key: container_engine.free_host_port() for key in names}
    created: list[str] = []
    try:
        for key, image in (("cjdb", CJDB_IMAGE), ("3dcitydb", CITYDB_IMAGE)):
            data = run_root / key
            data.mkdir()
            env = {"POSTGRES_USER": "bench", "POSTGRES_PASSWORD": "bench", "POSTGRES_DB": "bench"}
            if key == "3dcitydb": env["SRID"] = str(srid)
            def run_args(flags: list[str]) -> list[str]:
                return engine.run_args(
                    name=names[key], image=image, detach=True, cpus=CPU_LIMIT, memory=MEMORY_LIMIT,
                    shm_size=SHM_SIZE, publish=(5432, ports[key]), env=env, extra=tuple(flags),
                    volumes=((str(data), "/var/lib/postgresql/data"), (str(temp_dirs[key]), "/tmp"),
                             (str(POSTGRES_CONF), "/etc/postgresql/postgresql.conf:ro")),
                    command=("postgres", "-c", "config_file=/etc/postgresql/postgresql.conf"))
            try:
                _run(*run_args(extra))
            except subprocess.CalledProcessError:
                if not extra:
                    raise
                # An isolation failure never aborts the run: the engine
                # rejected the cpuset flags, so start every container without them.
                subprocess.run(engine.rm_args(names[key]), check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                extra = []
                _run(*run_args(extra))
            created.append(names[key])
        _wait(ports["cjdb"]); _wait(ports["3dcitydb"], citydb=True)
        yield {"ports": ports, "containers": names, "run_root": run_root, "temp_root": temp_root, "temp_dirs": temp_dirs, "container_args_applied": bool(container_args) and extra == list(container_args)}
    finally:
        for name in reversed(created):
            subprocess.run(engine.stop_args(name), check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for name, previous_value in previous_temp_environment.items():
            if previous_value is None:
                os.environ.pop(name, None)
            else:
                os.environ[name] = previous_value
        shutil.rmtree(temp_root, ignore_errors=True)
