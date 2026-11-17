#!/usr/bin/env python3
"""Sets the next version of one release family, in every file that carries it.

    bump-version.py rust|citygml|mcp|py patch|minor|major|none [--root DIR]

Prints the version the family now has. `none` leaves the number alone and only
checks the files agree on it — the first release of a family publishes the
version already written down (0.1.0), not the one after it.

rust  The three crates in lib/cityparquet-rs share one version:
      `[workspace.package] version` and the workspace's requirements on
      `cityparquet` and `cityparquet-schema`. Both lockfiles that record the
      crates — the library's and benchmark/readbench's, which path-depends on
      it — are brought in step with cargo, so a `--locked` build still passes.
citygml  The five crates in lib/citygml-rs share one version:
      `[workspace.package] version` and the workspace's requirements on
      `citygml-core`, `citygml2` and `citygml3`. The library's requirements on
      `citygml` (the package `citygml-rs`) and `citygml-citydb` follow it, and
      every lockfile that records the crates is brought in step.
mcp   ai/mcp/package.json.
py    The Python bindings in lib/cityparquet-py: `[package] version`, which
      maturin gives the wheel, and that crate's own lockfile.

Standard library only: the publish workflows run it on every runner OS before
anything else is installed.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

KINDS = ("patch", "minor", "major", "none")

RUST_MANIFEST = Path("lib/cityparquet-rs/Cargo.toml")
RUST_LOCK_DIRS = (Path("lib/cityparquet-rs"), Path("benchmark/readbench"))
RUST_CRATES = ("cityparquet", "cityparquet-schema")
CITYGML_MANIFEST = Path("lib/citygml-rs/Cargo.toml")
CITYGML_CRATES = ("citygml-core", "citygml2", "citygml3")
# The workspace that depends on the CityGML crates, and the keys it does so by.
CITYGML_DEPENDANT_CRATES = ("citygml", "citygml-citydb")
CITYGML_LOCK_DIRS = (
    Path("lib/citygml-rs"),
    Path("lib/cityparquet-rs"),
    Path("lib/cityparquet-py"),
    Path("benchmark/readbench"),
)
MCP_MANIFEST = Path("ai/mcp/package.json")
PY_MANIFEST = Path("lib/cityparquet-py/Cargo.toml")
PY_LOCK_DIR = Path("lib/cityparquet-py")

VERSION = re.compile(r"^(\d+)\.(\d+)\.(\d+)$")


def bump(version: str, kind: str) -> str:
    match = VERSION.match(version)
    if not match:
        raise ValueError(f"not a plain x.y.z version: {version}")
    major, minor, patch = (int(part) for part in match.groups())
    if kind == "major":
        return f"{major + 1}.0.0"
    if kind == "minor":
        return f"{major}.{minor + 1}.0"
    if kind == "patch":
        return f"{major}.{minor}.{patch + 1}"
    if kind == "none":
        return version
    raise ValueError(f"bump must be one of {', '.join(KINDS)}, not {kind}")


# `version = "x"` inside [workspace.package], and the version requirement of
# each sibling crate in [workspace.dependencies]. Anchored on the table and the
# crate name, so no third-party `version = ` line can match.
_PACKAGE_VERSION = re.compile(
    r'(^\[workspace\.package\]\n(?:(?!^\[).*\n)*?^version = ")([^"]+)(")',
    re.MULTILINE,
)


def _dependency_version(crate: str) -> re.Pattern[str]:
    return re.compile(
        rf'(^{re.escape(crate)} = \{{[^}}\n]*\bversion = ")([^"]+)(")',
        re.MULTILINE,
    )


def _bump_workspace_manifest(
    text: str, kind: str, crates: tuple[str, ...]
) -> tuple[str, str]:
    """Returns the manifest with every version slot set, and the new version."""
    patterns = [_PACKAGE_VERSION, *(_dependency_version(c) for c in crates)]
    found = []
    for pattern in patterns:
        matches = pattern.findall(text)
        if len(matches) != 1:
            raise ValueError(
                f"expected exactly one match for {pattern.pattern!r}, "
                f"found {len(matches)}"
            )
        found.append(matches[0][1])
    if len(set(found)) != 1:
        raise ValueError(f"the crate versions disagree: {', '.join(found)}")
    new = bump(found[0], kind)
    for pattern in patterns:
        text = pattern.sub(lambda m: f"{m.group(1)}{new}{m.group(3)}", text)
    return text, new


def bump_rust_manifest(text: str, kind: str) -> tuple[str, str]:
    return _bump_workspace_manifest(text, kind, RUST_CRATES)


def bump_citygml_manifest(text: str, kind: str) -> tuple[str, str]:
    return _bump_workspace_manifest(text, kind, CITYGML_CRATES)


def set_dependency_versions(text: str, crates: tuple[str, ...], version: str) -> str:
    """Returns the manifest with each named dependency requiring `version`."""
    for crate in crates:
        pattern = _dependency_version(crate)
        if len(pattern.findall(text)) != 1:
            raise ValueError(f"expected exactly one versioned dependency on {crate}")
        text = pattern.sub(lambda m: f"{m.group(1)}{version}{m.group(3)}", text)
    return text


# `version = "x"` inside [package], anchored on the table like the workspace
# pattern, so a dependency's `version = ` cannot match.
_CRATE_VERSION = re.compile(
    r'(^\[package\]\n(?:(?!^\[).*\n)*?^version = ")([^"]+)(")',
    re.MULTILINE,
)


def bump_python_manifest(text: str, kind: str) -> tuple[str, str]:
    """Returns the bindings' manifest with its version set, and the version."""
    matches = _CRATE_VERSION.findall(text)
    if len(matches) != 1:
        raise ValueError(
            f"expected exactly one [package] version, found {len(matches)}"
        )
    new = bump(matches[0][1], kind)
    return _CRATE_VERSION.sub(lambda m: f"{m.group(1)}{new}{m.group(3)}", text), new


def bump_package_json(text: str, kind: str) -> tuple[str, str]:
    manifest = json.loads(text)
    new = bump(manifest["version"], kind)
    manifest["version"] = new
    return json.dumps(manifest, indent=2, ensure_ascii=False) + "\n", new


def _rewrite(path: Path, edit, kind: str) -> str:
    # newline="" both ways: keep whatever line endings the checkout has.
    with path.open(encoding="utf-8", newline="") as handle:
        text = handle.read()
    updated, version = edit(text, kind)
    with path.open("w", encoding="utf-8", newline="") as handle:
        handle.write(updated)
    return version


def _relock(root: Path, directories: tuple[Path, ...]) -> None:
    for directory in directories:
        # Any resolution rewrites the lockfile's entries for the path crates
        # and leaves every other locked version where it is. `cargo update -p`
        # would not: it unlocks the crate's transitive dependencies too.
        subprocess.run(
            ["cargo", "metadata", "--format-version", "1"],
            cwd=root / directory,
            check=True,
            stdout=subprocess.DEVNULL,
        )


def run(family: str, kind: str, root: Path) -> str:
    if family == "rust":
        version = _rewrite(root / RUST_MANIFEST, bump_rust_manifest, kind)
        if kind != "none":
            _relock(root, RUST_LOCK_DIRS)
        return version
    if family == "citygml":
        version = _rewrite(root / CITYGML_MANIFEST, bump_citygml_manifest, kind)
        if kind != "none":
            _rewrite(
                root / RUST_MANIFEST,
                lambda text, _: (
                    set_dependency_versions(text, CITYGML_DEPENDANT_CRATES, version),
                    version,
                ),
                kind,
            )
            _relock(root, CITYGML_LOCK_DIRS)
        return version
    if family == "mcp":
        return _rewrite(root / MCP_MANIFEST, bump_package_json, kind)
    if family == "py":
        version = _rewrite(root / PY_MANIFEST, bump_python_manifest, kind)
        if kind != "none":
            _relock(root, (PY_LOCK_DIR,))
        return version
    raise ValueError(f"family must be rust, citygml, mcp or py, not {family}")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("family", choices=("rust", "citygml", "mcp", "py"))
    parser.add_argument("kind", choices=KINDS)
    parser.add_argument("--root", type=Path, default=Path.cwd())
    args = parser.parse_args(argv)
    print(run(args.family, args.kind, args.root))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
