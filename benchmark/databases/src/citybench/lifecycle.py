"""Isolated, disposable Podman databases for one benchmark run."""
from __future__ import annotations

import subprocess
import time
import uuid
from contextlib import contextmanager
from pathlib import Path

DATABASES_DIR = Path(__file__).resolve().parents[2]
ROOT = (DATABASES_DIR.parent / "runs").resolve()
CJDB_IMAGE = "docker.io/postgis/postgis:16-3.4"
CITYDB_IMAGE = "docker.io/3dcitydb/3dcitydb-pg:16-3.4-5.1.2-alpine"
POSTGRES_CONF = Path(__file__).resolve().parents[2] / "docker" / "postgresql.conf"
CPU_LIMIT = "16"
MEMORY_LIMIT = "32g"
SHM_SIZE = "2g"


def _run(*args: str, capture: bool = False) -> str:
    return subprocess.run(args, check=True, text=True, capture_output=capture).stdout if capture else (subprocess.run(args, check=True), "")[1]


def _port(name: str) -> int:
    value = _run("podman", "port", name, "5432/tcp", capture=True).strip()
    return int(value.rsplit(":", 1)[1])


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
def isolated_databases(data_root: Path, srid: int):
    root = data_root.resolve()
    if root != ROOT and ROOT not in root.parents:
        raise ValueError(f"data root must be below {ROOT}")
    run_id = uuid.uuid4().hex
    run_root = root / "databases" / run_id
    run_root.mkdir(parents=True)
    names = {"cjdb": f"citybench-cjdb-{run_id}", "3dcitydb": f"citybench-citydb-{run_id}"}
    common = ["-d", "--rm", "--cpus", CPU_LIMIT, "--memory", MEMORY_LIMIT, "--shm-size", SHM_SIZE, "-p", "127.0.0.1::5432", "-v"]
    created: list[str] = []
    try:
        for key, image in (("cjdb", CJDB_IMAGE), ("3dcitydb", CITYDB_IMAGE)):
            data = run_root / key
            data.mkdir()
            args = ["podman", "run", *common, f"{data}:/var/lib/postgresql/data", "--name", names[key], "-e", "POSTGRES_USER=bench", "-e", "POSTGRES_PASSWORD=bench", "-e", "POSTGRES_DB=bench"]
            if key == "3dcitydb": args += ["-e", f"SRID={srid}"]
            args += ["-v", f"{POSTGRES_CONF}:/etc/postgresql/postgresql.conf:ro", image, "postgres", "-c", "config_file=/etc/postgresql/postgresql.conf"]
            _run(*args); created.append(names[key])
        ports = {key: _port(name) for key, name in names.items()}
        _wait(ports["cjdb"]); _wait(ports["3dcitydb"], citydb=True)
        yield {"ports": ports, "containers": names, "run_root": run_root}
    finally:
        for name in reversed(created):
            subprocess.run(["podman", "stop", name], check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
