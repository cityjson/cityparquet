import { rm } from "node:fs/promises";

import { runDir } from "../playwright.config";

/**
 * Removes the per-run directory `playwright.config.ts` creates for the API's
 * catalog and storage. Playwright runs this after the whole suite finishes —
 * pass or fail, since a run that only cleans up on success leaks a
 * catalog-bearing `/tmp/citylake-e2e-<pid>` directory on exactly the runs
 * where you are iterating (and re-running) most. `force: true` makes the
 * removal a no-op rather than an error if the directory was never created
 * (for example, if `mkdirSync` in the config itself failed).
 */
export default async function globalTeardown(): Promise<void> {
  await rm(runDir, { recursive: true, force: true });
}
