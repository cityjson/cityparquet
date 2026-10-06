"""The one container engine every container call goes through.

The engine is chosen at run time by a cascade over ``ENGINE_PRIORITY``: the
first engine that is installed AND responding wins. ``CITYBENCH_CONTAINER_ENGINE``
(or ``citybench --container-engine``) overrides the cascade. Capabilities are
probed per engine (its ``run --help`` text and the host platform), never
assumed; anything an engine lacks is reported as ``"not applied: <reason>"``
so the run manifest records it instead of the harness skipping it silently.

On macOS both Apple ``container`` and ``docker`` run Linux in a virtual
machine, so numbers measured through them test the harness and are not
citable.
"""
from __future__ import annotations

import json
import os
import re
import shutil
import socket
import subprocess
import sys
from dataclasses import dataclass, field
from typing import Callable

ENV_VAR = "CITYBENCH_CONTAINER_ENGINE"

# The cascade, highest priority first. Adding an engine is one entry here
# plus one in ``_SPELLINGS``.
ENGINE_PRIORITY: tuple[str, ...] = ("container", "docker", "podman")

# Per-engine command spelling: how to ask "are you responding?", and how to
# force-remove a container.
_SPELLINGS: dict[str, dict[str, tuple[str, ...]]] = {
    "container": {"ping": ("system", "status"), "rm": ("delete", "--force")},
    "docker": {"ping": ("info",), "rm": ("rm", "-f")},
    "podman": {"ping": ("info",), "rm": ("rm", "-f")},
}

Runner = Callable[[list[str]], "subprocess.CompletedProcess[str]"]


def _default_runner(argv: list[str]) -> "subprocess.CompletedProcess[str]":
    try:
        return subprocess.run(argv, text=True, capture_output=True, timeout=20, check=False)
    except (OSError, subprocess.SubprocessError) as error:
        return subprocess.CompletedProcess(argv, 127, "", str(error))


class NoContainerEngine(RuntimeError):
    pass


@dataclass(frozen=True)
class Engine:
    name: str
    binary: str
    version: str
    run_flags: frozenset[str]
    platform: str = field(default_factory=lambda: sys.platform)

    # ---- probed capabilities -------------------------------------------
    @property
    def native_linux(self) -> bool:
        """Containers share the host kernel (no virtual machine)."""
        return self.platform.startswith("linux") and self.name != "container"

    def _gap(self, ok: bool, reason: str) -> str | None:
        return None if ok else f"not applied: {reason}"

    def cpu_limit_gap(self) -> str | None:
        return self._gap("--cpus" in self.run_flags, f"{self.name} run has no --cpus")

    def memory_limit_gap(self) -> str | None:
        return self._gap("--memory" in self.run_flags, f"{self.name} run has no --memory")

    def shm_size_gap(self) -> str | None:
        return self._gap("--shm-size" in self.run_flags, f"{self.name} run has no --shm-size")

    def cpuset_gap(self) -> str | None:
        if "--cpuset-cpus" not in self.run_flags:
            return f"not applied: {self.name} run has no --cpuset-cpus/--cpuset-mems"
        return self._gap(self.native_linux, f"{self.name} on {self.platform} runs a virtual machine; host cores are not addressable")

    def host_network_gap(self) -> str | None:
        return self._gap(self.native_linux and "--network" in self.run_flags,
                         f"{self.name} on {self.platform} has no host network; tools reach the database at its container address")

    def host_proc_gap(self) -> str | None:
        """Whether a container process's ``/proc/<pid>/status`` is readable on the host."""
        return self._gap(self.native_linux,
                         f"{self.name} on {self.platform} runs a virtual machine; /proc is read through exec")

    def capabilities(self) -> dict[str, str]:
        """Every capability as ``"applied"`` or ``"not applied: <reason>"``."""
        gaps = {
            "cpu_limit": self.cpu_limit_gap(), "memory_limit": self.memory_limit_gap(),
            "shm_size": self.shm_size_gap(), "cpuset": self.cpuset_gap(),
            "host_network": self.host_network_gap(), "host_proc": self.host_proc_gap(),
        }
        return {key: gap or "applied" for key, gap in gaps.items()}

    def manifest(self) -> dict:
        return {"name": self.name, "binary": self.binary, "version": self.version,
                "virtual_machine": not self.native_linux, "capabilities": self.capabilities()}

    # ---- command construction ------------------------------------------
    def cmd(self, *args: str) -> list[str]:
        return [self.binary, *args]

    def run_args(self, *, name: str | None, image: str, command: tuple[str, ...] = (),
                 detach: bool = False, remove: bool = True, cpus: str | None = None,
                 memory: str | None = None, shm_size: str | None = None,
                 publish: tuple[str, int] | None = None, host_network: bool = False,
                 volumes: tuple[tuple[str, str], ...] = (), env: dict[str, str] | None = None,
                 extra: tuple[str, ...] = ()) -> list[str]:
        """A ``run`` command; flags the engine lacks are left out (the caller
        records the matching ``*_gap()`` in the manifest)."""
        argv = [self.binary, "run"]
        if detach: argv.append("-d")
        if remove: argv.append("--rm")
        if cpus and not self.cpu_limit_gap(): argv += ["--cpus", cpus]
        if memory and not self.memory_limit_gap(): argv += ["--memory", memory]
        if shm_size and not self.shm_size_gap(): argv += ["--shm-size", shm_size]
        if host_network and not self.host_network_gap(): argv += ["--network", "host"]
        if publish:
            # An explicit host port: Apple `container` has no random-port form
            # and no `port` subcommand, so every engine gets the same spelling.
            argv += ["-p", f"127.0.0.1:{publish[1]}:{publish[0]}"]
        for source, target in volumes:
            argv += ["-v", f"{source}:{target}"]
        for key, value in (env or {}).items():
            argv += ["-e", f"{key}={value}"]
        if name: argv += ["--name", name]
        return [*argv, *extra, image, *command]

    def rm_args(self, name: str) -> list[str]:
        return [self.binary, *_SPELLINGS[self.name]["rm"], name]

    def stop_args(self, name: str) -> list[str]:
        return [self.binary, "stop", name]

    def exec_args(self, name: str, *command: str) -> list[str]:
        return [self.binary, "exec", name, *command]

    def inspect_args(self, name: str) -> list[str]:
        return [self.binary, "inspect", name]

    # ---- queries -------------------------------------------------------
    def container_address(self, name: str, runner: Runner = _default_runner) -> str | None:
        """The container's own IP address (how a sibling container reaches it
        when there is no host network)."""
        result = runner(self.inspect_args(name))
        if result.returncode != 0:
            return None
        return parse_container_address(result.stdout)

    def proc_status(self, name: str, pid: int, runner: Runner = _default_runner) -> str | None:
        """``/proc/<pid>/status`` of a container-namespace PID, read through exec."""
        result = runner(self.exec_args(name, "cat", f"/proc/{pid}/status"))
        return result.stdout if result.returncode == 0 else None


def parse_container_address(inspect_json: str) -> str | None:
    """First IPv4 address in an ``inspect`` document of any engine."""
    try:
        document = json.loads(inspect_json)
    except ValueError:
        return None
    text = json.dumps(document)
    for key in ("IPAddress", "ipv4Address", "address"):
        for match in re.finditer(rf'"{key}": "(\d+\.\d+\.\d+\.\d+)(?:/\d+)?"', text):
            return match.group(1)
    return None


def free_host_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def probe(name: str, *, which: Callable[[str], str | None] = shutil.which,
          runner: Runner = _default_runner, platform: str | None = None) -> Engine | None:
    """The engine if it is installed and responding, else ``None``."""
    if name not in _SPELLINGS:
        raise ValueError(f"unknown container engine {name!r}; known: {', '.join(_SPELLINGS)}")
    binary = which(name)
    if not binary:
        return None
    if runner([binary, *_SPELLINGS[name]["ping"]]).returncode != 0:
        return None
    version = runner([binary, "--version"]).stdout.strip().splitlines()
    run_help = runner([binary, "run", "--help"]).stdout
    flags = frozenset(re.findall(r"(--[a-z][a-z0-9-]*)", run_help))
    return Engine(name=name, binary=binary, version=version[0] if version else "unknown",
                  run_flags=flags, platform=platform or sys.platform)


def select_engine(override: str | None = None, *, which: Callable[[str], str | None] = shutil.which,
                  runner: Runner = _default_runner, platform: str | None = None,
                  priority: tuple[str, ...] = ENGINE_PRIORITY) -> Engine:
    """The cascade: an explicit override (flag, then env var) or the first
    responding engine in ``priority``."""
    chosen = override or os.environ.get(ENV_VAR) or None
    candidates = (chosen,) if chosen else priority
    tried: list[str] = []
    for name in candidates:
        engine = probe(name, which=which, runner=runner, platform=platform)
        if engine is not None:
            return engine
        tried.append(name)
    raise NoContainerEngine(f"no container engine installed and responding (tried: {', '.join(tried)})")


_ACTIVE: Engine | None = None


def active() -> Engine:
    """The process-wide engine, selected on first use."""
    global _ACTIVE
    if _ACTIVE is None:
        _ACTIVE = select_engine()
    return _ACTIVE


def set_active(engine: Engine | None) -> None:
    global _ACTIVE
    _ACTIVE = engine


def compose_command(engine: Engine) -> str | None:
    """The compose front end for the long-lived ``just up`` stack, or ``None``
    when the engine has none (Apple ``container``)."""
    return {"podman": "podman-compose", "docker": "docker compose"}.get(engine.name)


if __name__ == "__main__":  # `python -m citybench.engine [binary|compose|manifest]`
    selected = select_engine()
    what = sys.argv[1] if len(sys.argv) > 1 else "binary"
    if what == "compose":
        command = compose_command(selected)
        if command is None:
            sys.exit(f"{selected.name} has no compose front end; `citybench run --data-root` starts its own containers")
        print(command)
    elif what == "manifest":
        print(json.dumps(selected.manifest(), indent=2))
    else:
        print(selected.binary)
