# CityLake — workspace command runner.
#
# Run `just` (no args) to see this menu, or `just <recipe>` to run one.
#
# Recipes that need environment variables wrap the command in `dotenvx run`,
# which decrypts and loads encrypted .env files. dotenvx reads `./.env` by
# default; pass `--env-file <path>` (or set DOTENVX_ENV_FILE) to override.
#
# Required tooling (one-time):
#   - cargo (rustup)
#   - vp     — `curl -fsSL https://vite.plus | bash`  (see web/README.md)
#   - dotenvx — `brew install dotenvx/brew/dotenvx`   (or `npm i -g @dotenvx/dotenvx`)

set shell := ["bash", "-cu"]
# `just`'s built-in dotenv loader is off — we route everything through dotenvx
# so encrypted files Just Work.
set dotenv-load := false

web_dir := "web"

# Show available recipes (default).
default:
    @just --list

# === Rust (cargo) =====================================================

# Start the CityLake HTTP server. Forwards extra args, e.g. `just api --release`.
api *args:
    dotenvx run --quiet -- cargo run {{args}}

# Build the library + binary.
build:
    cargo build

# Run unit + e2e tests (offline; ~57 tests, sub-second).
test:
    cargo test --lib

# Run integration tests against real cityjson + the public Delft URLs (network, ~80 s).
test-integration:
    cargo test --lib -- --ignored --test-threads=1

# Lint the Rust crate (clippy with warnings as errors).
check:
    cargo clippy --all-targets -- -D warnings

# Format the Rust crate.
fmt:
    cargo fmt

# Library-only build (skips the axum/tower deps).
build-lib:
    cargo build --no-default-features --lib

# === Web (Vite+) ======================================================

# Start the web dev server (HMR + /api proxy to 127.0.0.1:3000).
web:
    cd {{web_dir}} && dotenvx run --quiet -- vp dev

# Production build of the web app.
web-build:
    cd {{web_dir}} && dotenvx run --quiet -- vp build

# Preview the production build locally.
web-preview:
    cd {{web_dir}} && dotenvx run --quiet -- vp preview

# Lint + format + typecheck the web app (oxlint + oxfmt + tsgo via vp check).
web-check:
    cd {{web_dir}} && vp check

# Run web tests (Vitest via vp).
web-test:
    cd {{web_dir}} && vp test

# Install web dependencies.
web-install:
    cd {{web_dir}} && npm install

# === Workspace ========================================================

# Install every dep (currently web only; cargo resolves on first build).
install: web-install

# Run server + web dev in parallel. Ctrl-C stops both.
dev:
    #!/usr/bin/env bash
    set -uo pipefail
    trap 'kill 0' SIGINT SIGTERM EXIT
    just api &
    just web &
    wait

# Run cargo + web checks.
check-all: check web-check

# Run every offline test (Rust unit/e2e + web).
test-all: test web-test

# Wipe build artifacts. Does not touch lockfiles or .env files.
clean:
    cargo clean
    rm -rf {{web_dir}}/dist {{web_dir}}/*.tsbuildinfo
