"""The container engine cascade and command construction, with no engine installed."""
import subprocess

import pytest

from citybench import engine as eng

HELP = {
    "container": "-c, --cpus <cpus>\n-m, --memory <m>\n--shm-size <s>\n--network <n>\n-p, --publish\n-v, --volume\n",
    "docker": "--cpus\n--memory\n--shm-size\n--network\n--cpuset-cpus\n--cpuset-mems\n-p\n",
    "podman": "--cpus\n--memory\n--shm-size\n--network\n--cpuset-cpus\n--cpuset-mems\n-p\n",
}


def fake(installed: set[str], responding: set[str]):
    calls: list[list[str]] = []

    def which(name):
        return f"/usr/bin/{name}" if name in installed else None

    def runner(argv):
        calls.append(argv)
        name = argv[0].rsplit("/", 1)[1]
        if argv[1:] == ["--version"]:
            return subprocess.CompletedProcess(argv, 0, f"{name} version 1.0\n", "")
        if argv[1:] == ["run", "--help"]:
            return subprocess.CompletedProcess(argv, 0, HELP[name], "")
        return subprocess.CompletedProcess(argv, 0 if name in responding else 1, "", "")

    return which, runner, calls


def test_cascade_prefers_apple_container_then_docker_then_podman():
    which, runner, _ = fake({"container", "docker", "podman"}, {"container", "docker", "podman"})
    assert eng.select_engine(which=which, runner=runner).name == "container"
    which, runner, _ = fake({"container", "docker", "podman"}, {"docker", "podman"})
    assert eng.select_engine(which=which, runner=runner).name == "docker"
    which, runner, _ = fake({"podman"}, {"podman"})
    assert eng.select_engine(which=which, runner=runner).name == "podman"


def test_responding_is_probed_with_each_engine_spelling():
    which, runner, calls = fake({"container", "docker"}, {"docker"})
    eng.select_engine(which=which, runner=runner)
    assert ["/usr/bin/container", "system", "status"] in calls
    assert ["/usr/bin/docker", "info"] in calls


def test_override_by_argument_and_env(monkeypatch):
    which, runner, _ = fake({"container", "podman"}, {"container", "podman"})
    assert eng.select_engine("podman", which=which, runner=runner).name == "podman"
    monkeypatch.setenv(eng.ENV_VAR, "podman")
    assert eng.select_engine(which=which, runner=runner).name == "podman"


def test_override_that_is_not_responding_fails_loudly(monkeypatch):
    which, runner, _ = fake({"container"}, {"container"})
    with pytest.raises(eng.NoContainerEngine, match="podman"):
        eng.select_engine("podman", which=which, runner=runner)
    with pytest.raises(ValueError, match="unknown container engine"):
        eng.select_engine("lxc", which=which, runner=runner)


def test_no_engine_raises():
    which, runner, _ = fake(set(), set())
    with pytest.raises(eng.NoContainerEngine, match="container, docker, podman"):
        eng.select_engine(which=which, runner=runner)


def _engine(name, platform):
    which, runner, _ = fake({name}, {name})
    return eng.probe(name, which=which, runner=runner, platform=platform)


def test_capabilities_are_probed_and_gaps_are_named():
    apple = _engine("container", "darwin")
    caps = apple.capabilities()
    assert caps["memory_limit"] == "applied" and caps["cpu_limit"] == "applied"
    assert caps["cpuset"].startswith("not applied: container run has no --cpuset-cpus")
    assert caps["host_network"].startswith("not applied:")
    assert caps["host_proc"].startswith("not applied:")
    assert apple.manifest()["version"] == "container version 1.0"
    assert apple.manifest()["virtual_machine"] is True
    docker_mac = _engine("docker", "darwin")
    assert docker_mac.capabilities()["cpuset"].startswith("not applied: docker on darwin")
    podman = _engine("podman", "linux")
    assert set(podman.capabilities().values()) == {"applied"}
    assert podman.manifest()["virtual_machine"] is False


def test_run_command_per_engine():
    for name, platform in (("container", "darwin"), ("docker", "darwin"), ("podman", "linux")):
        e = _engine(name, platform)
        argv = e.run_args(name="db", image="img", command=("postgres",), detach=True,
                          cpus="16", memory="32g", shm_size="2g", publish=(5432, 40001),
                          host_network=True, volumes=(("/a", "/b"),), env={"K": "v"})
        assert argv[:2] == [f"/usr/bin/{name}", "run"]
        assert argv[-2:] == ["img", "postgres"]
        assert "127.0.0.1:40001:5432" in argv and "/a:/b" in argv and "K=v" in argv
        assert argv[argv.index("--memory") + 1] == "32g"
        assert ("--network" in argv) == (name == "podman")


def test_rm_stop_exec_spelling():
    apple, podman = _engine("container", "darwin"), _engine("podman", "linux")
    assert apple.rm_args("x") == ["/usr/bin/container", "delete", "--force", "x"]
    assert podman.rm_args("x") == ["/usr/bin/podman", "rm", "-f", "x"]
    assert apple.stop_args("x") == ["/usr/bin/container", "stop", "x"]
    assert apple.exec_args("x", "cat", "/proc/7/status") == ["/usr/bin/container", "exec", "x", "cat", "/proc/7/status"]


def test_parse_container_address_for_each_engine():
    assert eng.parse_container_address('[{"networks": [{"address": "192.168.64.5/24"}]}]') == "192.168.64.5"
    assert eng.parse_container_address('[{"NetworkSettings": {"IPAddress": "10.88.0.4"}}]') == "10.88.0.4"
    assert eng.parse_container_address("not json") is None


def test_compose_front_end_per_engine():
    assert eng.compose_command(_engine("podman", "linux")) == "podman-compose"
    assert eng.compose_command(_engine("docker", "darwin")) == "docker compose"
    assert eng.compose_command(_engine("container", "darwin")) is None


@pytest.mark.parametrize("name", ["container", "docker", "podman"])
def test_run_args_pins_the_image_platform(name):
    # Both PostgreSQL images publish linux/amd64 only; an arm64 host runs
    # them under emulation only when the platform is named.
    argv = _engine(name, "darwin").run_args(name="x", image="img", platform="linux/amd64")
    assert argv[argv.index("--platform") + 1] == "linux/amd64"
    assert argv.index("--platform") < argv.index("img")


def test_only_apple_container_refuses_ownership_changes_on_bind_mounts():
    # Its virtiofs bind mounts reject chown/chmod, which the PostgreSQL
    # entrypoint runs on its data directory.
    assert _engine("container", "darwin").bind_mount_ownership_gap()
    assert _engine("docker", "linux").bind_mount_ownership_gap() is None
