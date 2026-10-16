# Contributing

Thanks for looking. CityParquet is early — the specification is still changing
and the implementations are catching up to it — so the most useful contributions
right now are the ones that find where the format is wrong, not just where the
code is.

## Setup

```sh
git clone --recurse-submodules https://github.com/cityjson/cityparquet.git
cd cityparquet
just setup        # or `just setup-shallow` for a --depth 1 checkout
just hooks        # rustfmt + Prettier on staged files, one-off per clone
```

`just setup` pulls the two DuckDB extensions' own submodules (DuckDB, `vcpkg`,
`extension-ci-tools`) — roughly 1.2 GB. You do **not** need it to work on the
specification or on the Rust library; `just setup-shallow`, or no submodules at
all, is enough for those.

What each area needs installed:

| Area                                    | Needs                                                   |
| --------------------------------------- | ------------------------------------------------------- |
| `documents/` (the specification)        | Node 24, pnpm 10                                        |
| `lib/cityparquet-rs`                    | Rust (pinned in `rust-toolchain.toml`), `just`          |
| `lib/citylake`                          | Rust                                                    |
| `lib/duckdb-*`                          | a C++ toolchain, `ninja`, `ccache` (recommended)        |
| `benchmark/plot`, `benchmark/databases` | `uv`; the database harness also needs rootless `podman` |
| the benchmark shell suites              | `jq`, `zip`/`unzip`                                     |
| `scripts/catalog2cityparquet`           | `uv`                                                    |

## Where a change goes

**A change to the format itself** goes to
[`documents/docs/03-specification/`](documents/docs/03-specification/), and the
reasoning goes to `04-design-decisions/` in the same pull request. A
specification change with no recorded rationale is the one thing that reliably
becomes unmaintainable: six months later nobody can tell a decision from an
accident. If the question is genuinely still open, put it in
`05-open-questions/` instead of deciding it by writing it down.

**A change to the encoding in code** goes to `lib/cityparquet-rs`, which owns
the encoding. If it makes the implementation disagree with the specification,
either the specification changes in the same PR or the divergence is recorded
in the implementation-status table in
`documents/docs/06-resources/02-software.mdx`. Silent divergence is not an
option.

**A change to the DuckDB extensions** goes to their own repositories
(`cityjson/duckdb-cityjson`, `HideBa/duckdb-3d`); they are consumed here as
submodules, and this repository only records which commit it is pinned to.

## What has to pass

```sh
cd lib/cityparquet-rs && just check   # the LIBRARY: clippy, tests, isolation, fmt, prettier
just plot-test                        # from the root; the plotting suite
just scripts-test                     # from the root; the benchmark shell suites
just check                            # all of the above plus benchmark/readbench, from the root
```

`just check` in `lib/cityparquet-rs` is deliberately self-contained — no `uv`,
no `jq`, no corpus — so it runs anywhere. It gates the **library alone**: the
read benchmark's harness is a separate Cargo workspace under
`benchmark/readbench`, and the root `just check` is what covers both.

The benchmarks themselves are **not** a gate. They are multi-hour and
corpus-dependent, and they are not in CI.

## House style

- **Strict red-green TDD** in `lib/cityparquet-rs`: the failing test first, then
  the smallest change that passes it. Tests read real CityJSON fixtures
  (`just fixtures`), never inline hand-written CityJSON.
- **Breaking changes are welcome.** There are no users yet. Pick the right
  design and update every call site; do not carry a shim, a deprecation path or
  a legacy branch for the old one.
- **Document the present, not the past.** No "fixed", "was broken", "now uses",
  no changelog voice in reference documentation. A reader wants how it is.
  History belongs in git.
- **British English** in prose.
- `AGENTS.md` mirrors `CLAUDE.md` at each level — edit one, copy it to the
  other.

## Releasing

Two things are published, each by a workflow run by hand from `main`
(Actions → the workflow → Run workflow), and each with its own tags:

| Workflow                                  | Publishes                                                           | Version lives in                                                 | Tag          |
| ----------------------------------------- | ------------------------------------------------------------------- | ---------------------------------------------------------------- | ------------ |
| **Publish crates** (`publish-crates.yml`) | `cityparquet-schema`, `cityparquet`, `cityparquet-cli` to crates.io | `[workspace.package] version` in `lib/cityparquet-rs/Cargo.toml` | `vX.Y.Z`     |
| **Publish MCP** (`publish-mcp.yml`)       | `@cityjson/cityparquet-mcp` to npm                                  | `ai/mcp/package.json`                                            | `mcp-vX.Y.Z` |

Bare `vX.Y.Z` belongs to the Rust reference implementation. Anything else
published from this repository takes a `<name>-vX.Y.Z` prefix, never a
`-suffix`, which semver reads as a pre-release.

Each run takes a bump (`patch`, `minor`, `major`, or `none` to publish the
version already written down, as for a first release) and a dry-run switch,
on by default. It bumps the version with `.github/scripts/bump-version.py`,
runs its gate, builds and packs, and uploads only once all of that has passed;
then it pushes a `release: <tag>` commit and the tag to `main` and merges that
commit back into `develop`. Do not edit these versions by hand. A dry run does
everything but the upload and the push, and runs from any branch.

When a publish succeeds, **Release** (`release.yml`) creates the GitHub
release for its tag, with what the run built attached and a `SHA256SUMS`: the
`cityparquet` CLI for five targets, or the npm tarball. Rerun it by hand with
the publish run's id if it fails.

A crates.io or npm version is permanent: yanking or deprecating hides it, it
never frees the number. That is why every build happens before the upload.

The crate publish needs the organisation secret `CARGO_REGISTRY_TOKEN`; the
MCP publish needs `NPM_TOKEN`, a token that may publish under `@cityjson`.
Both push with the workflow's own token, so `main` and `develop` must accept
pushes from `github-actions[bot]`.

`cityparquet-readbench` is `publish = false` and lives outside the library
workspace entirely, in `benchmark/readbench`: it is evidence, and only means
anything next to the corpora under `benchmark/`. CityLake is not published.

## Measurements

If you change a benchmark, read [`benchmark/README.md`](benchmark/README.md)
first. Several results there are deliberately **not** citable as rankings — the
codec comparison runs every codec at its implementation default, so it does not
support a "smallest codec" claim — and the caveats that say so are part of the
artefact, not commentary on it. A change that makes a number look better by
quietly dropping a caveat will be rejected.

## Licence of contributions

Contributions to the software are accepted under MIT OR Apache-2.0;
contributions to `documents/` under CC BY 4.0. By opening a pull request you
agree to license your work on those terms.
