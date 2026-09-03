import { mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { defineConfig, devices } from "@playwright/test";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// A fresh, out-of-tree home for the API's catalog and storage on every run.
// Outside the repository so a run never writes into a developer's checkout,
// and suffixed by pid so concurrent runs (or a rerun before cleanup) never
// share state. `reuseExistingServer: false` below means every run starts
// from this clean directory rather than a developer's own server.
export const runDir = path.join(tmpdir(), `citylake-e2e-${process.pid}`);
// The root `CITYLAKE_OUTPUT_ROOT` confines the two write endpoints to. It
// has to exist before the server resolves anything against it: the policy
// canonicalises the root, canonicalising a directory that is not there
// fails, and every write would then be refused as `ParentMissing` — a
// refusal that looks exactly like a working policy while testing nothing.
export const outputRoot = path.join(runDir, "out");
// This module is also imported by each worker process (they need `use`'s
// options), which would otherwise `mkdirSync` a second, empty, same-shaped
// directory under the worker's own pid that nothing ever writes to or
// cleans up — `TEST_WORKER_INDEX` is set only in a worker, never in the
// orchestrator that actually starts the webServers above and runs
// `globalTeardown`, so this keeps directory creation to the one process
// whose `runDir` is real.
// `outputRoot` sits inside `runDir`, so this one recursive call creates
// both — and `globalTeardown`'s removal of `runDir` takes the output root
// with it, which is why neither needs its own cleanup.
if (!process.env.TEST_WORKER_INDEX) mkdirSync(outputRoot, { recursive: true });

// `--manifest-path` needs an absolute path once `cwd` points outside the
// crate, so resolve both it and the extension relative to this file rather
// than hardcoding a machine-specific path.
const citylakeManifest = path.resolve(__dirname, "../Cargo.toml");
const cityjsonExtension = path.resolve(
  __dirname,
  "../../duckdb-cityjson/build/release/extension/cityjson/cityjson.duckdb_extension",
);

const apiPort = 3100;
const apiTarget = `http://127.0.0.1:${apiPort}`;

// Off `vite.config.ts`'s default (5173) on purpose: the whole point of this
// harness is to share nothing with a developer's own servers, and a
// developer running `vp dev` for manual testing is exactly the case that
// would otherwise collide with it. `--strictPort` turns that collision into
// a loud failure instead of Vite silently trying the next free port, which
// would desynchronise this file's `clientUrl` from where the client is
// actually listening.
const clientPort = 5183;
const clientUrl = `http://127.0.0.1:${clientPort}`;

export default defineConfig({
  testDir: "./e2e",
  // One worker, deliberately. Parallel workers buy nothing here and cost
  // stability: every API call queues behind the server's single
  // `Mutex<Connection>`, so the DuckDB work these specs are made of is
  // serialised no matter how many browsers ask for it — while three
  // browsers cold-loading one Vite dev server at once is real contention,
  // and it made the first render of the first page miss the 5s assertion
  // budget on roughly one run in five. Measured over eight consecutive
  // runs each way: three workers finished in 15.9-17.1s with intermittent
  // whole-suite timeouts, one worker in 19.9-20.2s with none. Four seconds
  // for a run that does not lie is the right trade.
  workers: 1,
  // `open: "never"` — the default ("on-failure") serves the report and
  // blocks the process until interrupted, which turns a deliberately failing
  // run (or any CI failure) into a hang rather than a clean exit.
  reporter: [["html", { open: "never" }]],
  // Removes the per-run directory created below, whether the suite passed
  // or failed — see e2e/global-teardown.ts.
  globalTeardown: "./e2e/global-teardown.ts",
  use: {
    baseURL: clientUrl,
    trace: "on-first-retry",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  webServer: [
    {
      // A cold `cargo run` compiles the crate before it serves, hence the
      // generous timeout below.
      command: `cargo run --manifest-path ${citylakeManifest}`,
      cwd: runDir,
      env: {
        ...process.env,
        CITYLAKE_PORT: String(apiPort),
        CITYLAKE_CATALOG_PATH: path.join(runDir, "metadata.ducklake"),
        CITYLAKE_STORAGE_PATH: path.join(runDir, "data"),
        CITYLAKE_OUTPUT_ROOT: outputRoot,
        CITYLAKE_CITYJSON_EXTENSION: cityjsonExtension,
      },
      url: `${apiTarget}/health`,
      timeout: 180_000,
      reuseExistingServer: false,
    },
    {
      // `VITE_E2E_AUTH_BYPASS` is the flag `AuthContext` checks; it must
      // carry the `VITE_` prefix or Vite never exposes it to the browser.
      // `CITYLAKE_API_TARGET` retargets `vite.config.ts`'s `/api` proxy at
      // the API instance above instead of its port-3000 default.
      command: `vp dev --port ${clientPort} --strictPort`,
      cwd: __dirname,
      env: {
        ...process.env,
        VITE_E2E_AUTH_BYPASS: "1",
        CITYLAKE_API_TARGET: apiTarget,
      },
      url: clientUrl,
      timeout: 60_000,
      reuseExistingServer: false,
    },
  ],
});
