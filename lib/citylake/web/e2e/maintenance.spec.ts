import path from "node:path";
import { fileURLToPath } from "node:url";

import { expect, test, type Page } from "@playwright/test";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// The merge destination: `lib/citylake/tests/data/delft.city.jsonl`, three
// flat Buildings, EPSG:7415, all routed to the `building` module.
const DESTINATION_FIXTURE = path.resolve(__dirname, "../../tests/data/delft.city.jsonl");
// The merge source: `lib/citylake/tests/data/minimal_7415.city.json`, a
// single Building also routed to `building`. Chosen for the two properties
// the extension checks before it will merge at all — it declares EPSG:7415,
// matching the destination, and its one object id (`building1`) collides
// with none of Delft's `NL.IMBAG.Pand.*` ids.
const SOURCE_FIXTURE = path.resolve(__dirname, "../../tests/data/minimal_7415.city.json");

// The destination's `building` row count before and after the merge. Written
// as literals rather than read from the page, because a count read from the
// page before the merge and compared to itself after would agree with a
// merge that did nothing.
const ROWS_BEFORE_MERGE = "3";
const ROWS_AFTER_MERGE = "4";

/** Unique per run, for the same reason as `journey.spec.ts`'s. */
function uniqueName(prefix: string): string {
  return `${prefix}_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 8)}`;
}

const DESTINATION = uniqueName("maint_dst");
const SOURCE = uniqueName("maint_src");

// One `test`, not one per operation. These steps are ordered — validate has
// to see a clean dataset, so it must run before the merge changes one — and
// `workers: 1` (`playwright.config.ts`) orders whole files against each
// other, not the blocks inside one. A single block is also what keeps the
// two uploads to one pair rather than one pair per operation.
//
// Above the 30s default because this block uploads two datasets and then
// runs seven DuckDB operations against them, every one of which queues
// behind the API's single `Mutex<Connection>`. A deliberate budget, not a
// retry: nothing here polls or sleeps, and every wait below is anchored on a
// specific response or a specific rendered result.
test.setTimeout(120_000);

/**
 * Uploads `fixture` as a new dataset named `name` and waits for the upload
 * to be acknowledged. Leaves the browser on the new dataset's detail page,
 * where `UploadPage` navigates on success.
 */
async function upload(page: Page, name: string, fixture: string): Promise<void> {
  const uploadResponse = page.waitForResponse(
    (response) =>
      response.url().endsWith(`/api/datasets/${name}/upload`) &&
      response.request().method() === "POST",
  );

  await page.goto("/upload");
  await page.getByLabel("City file").setInputFiles(fixture);
  await page.getByLabel("Dataset name").fill(name);
  // Scoped to `<main>`: AppShell's persistent header carries its own
  // icon-only "Upload" shortcut button, which the unscoped role query also
  // matches.
  await page.getByRole("main").getByRole("button", { name: "Upload", exact: true }).click();

  expect((await uploadResponse).ok()).toBe(true);
  await expect(page).toHaveURL(new RegExp(`/datasets/${name}$`));
}

test("maintain, merge and write a dataset, and refuse a path outside the root", async ({
  page,
}) => {
  // ---- 0. Two datasets ---------------------------------------------------
  await upload(page, DESTINATION, DESTINATION_FIXTURE);
  await upload(page, SOURCE, SOURCE_FIXTURE);

  // A full navigation, so every query below starts from the server rather
  // than from whatever the upload left in the client cache.
  await page.goto(`/datasets/${DESTINATION}`);
  await expect(page.getByRole("heading", { name: DESTINATION, level: 1 })).toBeVisible();

  const main = page.getByRole("main");
  // The destination's `building` row in the modules table. Its accessible
  // name is the row's cell text, so it still matches once the row count
  // changes.
  const buildingRow = main.getByRole("row", { name: /building/ });
  await expect(
    buildingRow.getByRole("cell", { name: ROWS_BEFORE_MERGE, exact: true }),
  ).toBeVisible();

  // ---- 1. Validate -------------------------------------------------------
  // A fresh dataset from this fixture reports nothing at all (the same
  // outcome `tests/inspect.rs`'s `a_freshly_created_dataset_validates_clean`
  // asserts), so the clean result is what should render. This discriminates:
  // a validate call that failed renders the error paragraph instead, and a
  // validate that found something renders the findings table.
  await main.getByRole("button", { name: "Validate", exact: true }).click();
  await expect(main.getByText("No problems found.")).toBeVisible();

  // ---- 2. Reconcile ------------------------------------------------------
  await main.getByRole("button", { name: "Reconcile", exact: true }).click();
  await expect(main.getByText("Reconcile completed.")).toBeVisible();

  // ---- 3. Vacuum ---------------------------------------------------------
  // Zero is the only honest expectation here: nothing in the interface can
  // produce an orphaned sidecar row, so this covers that vacuum runs and
  // reports, not that it reclaims. The positive path is a known gap.
  await main.getByRole("button", { name: "Vacuum", exact: true }).click();
  const vacuumConfirm = page.getByRole("alertdialog", { name: "Vacuum dataset?" });
  await expect(vacuumConfirm).toBeVisible();
  await vacuumConfirm.getByRole("button", { name: "Vacuum", exact: true }).click();
  await expect(main.getByText("0 rows reclaimed.")).toBeVisible();

  // ---- 4. Compact --------------------------------------------------------
  // The counts themselves are not pinned: how many files DuckLake finds
  // adjacent enough to merge is its business and not a contract this
  // interface makes. What is asserted is that the success branch rendered
  // with both counts in it, which a failed call would not.
  await main.getByRole("button", { name: "Compact", exact: true }).click();
  await expect(main.getByText(/\d+ files? processed, \d+ files? created\./)).toBeVisible();

  // ---- 5. Merge ----------------------------------------------------------
  // The row count growing is the assertion. A merge that silently does
  // nothing still closes its dialog and still renders "Merge completed." —
  // only the destination's `building` count going 3 → 4 distinguishes the
  // two.
  await main.getByRole("button", { name: "Merge…" }).click();
  const mergeDialog = page.getByRole("alertdialog", {
    name: `Merge a dataset into ${DESTINATION}?`,
  });
  await expect(mergeDialog).toBeVisible();
  await mergeDialog.getByLabel("Source dataset").selectOption(SOURCE);

  const mergeResponse = page.waitForResponse(
    (response) =>
      response.url().endsWith(`/api/datasets/${DESTINATION}/merge`) &&
      response.request().method() === "POST",
  );
  await mergeDialog.getByRole("button", { name: "Merge", exact: true }).click();
  expect((await mergeResponse).ok()).toBe(true);
  await expect(main.getByText("Merge completed.")).toBeVisible();

  // Both halves: the new count present and the old one gone. The second
  // rules out a modules table that grew a row rather than updating one.
  await expect(
    buildingRow.getByRole("cell", { name: ROWS_AFTER_MERGE, exact: true }),
  ).toBeVisible();
  await expect(buildingRow.getByRole("cell", { name: ROWS_BEFORE_MERGE, exact: true })).toHaveCount(
    0,
  );

  // ---- 6. Write a package inside the output root -------------------------
  // `CITYLAKE_OUTPUT_ROOT` is set by `playwright.config.ts` to an `out/`
  // directory under the per-run temp directory, so this relative path
  // resolves inside it and is allowed.
  //
  // One level deep, not `packages/<name>`: `cityparquet_write` creates the
  // package directory itself but not the directories above it, so a nested
  // path fails with an IO error from the extension — a real limitation of
  // the write, not of the path policy, and not one to paper over here.
  await main.getByRole("button", { name: "Write package…" }).click();
  const packageDialog = page.getByRole("dialog", { name: "Write package" });
  await expect(packageDialog).toBeVisible();
  await packageDialog.getByLabel("Output directory").fill(DESTINATION);
  await packageDialog.getByRole("button", { name: "Write package", exact: true }).click();

  // The written-files table, not a "done" message: one Parquet file per
  // non-empty object table plus the STAC Item is what makes the result a
  // package rather than an empty directory.
  await expect(
    packageDialog.getByRole("cell", { name: "building.parquet", exact: true }),
  ).toBeVisible();
  await expect(
    packageDialog.getByRole("cell", { name: "metadata.json", exact: true }),
  ).toBeVisible();
  // Escape, not a click on "Close": `ui/dialog.tsx` gives every dialog a
  // corner close control whose `sr-only` label is also exactly "Close", so
  // two buttons in this dialog carry that accessible name and a role query
  // cannot name one of them. Escape is the same close path — Radix fires
  // `onOpenChange(false)`, which is what resets the form for step 7.
  await page.keyboard.press("Escape");
  await expect(packageDialog).toBeHidden();

  // ---- 7. A path outside the output root is refused ----------------------
  // The one step that proves the server-side path policy reaches a user
  // rather than living only in the Rust suite.
  await main.getByRole("button", { name: "Write package…" }).click();
  await expect(packageDialog).toBeVisible();
  await packageDialog.getByLabel("Output directory").fill("../escaped");

  const refusal = page.waitForResponse(
    (response) =>
      response.url().endsWith(`/api/datasets/${DESTINATION}/package`) &&
      response.request().method() === "POST",
  );
  await packageDialog.getByRole("button", { name: "Write package", exact: true }).click();
  expect((await refusal).status()).toBe(400);

  // The policy's own sentence (`OutputPathError::Escapes`, carried through
  // `CityLakeError::BadRequest` and `ApiError` to the dialog), not merely
  // "some error is visible": a dataset that did not exist, or a malformed
  // body, would also paint that paragraph red.
  await expect(
    packageDialog.getByText("requested path escapes the configured output root"),
  ).toBeVisible();

  // And it was refused rather than written: the dialog is still showing its
  // form, so no files table replaced it.
  await expect(packageDialog.getByLabel("Output directory")).toBeVisible();
  await expect(packageDialog.getByRole("cell", { name: "building.parquet" })).toHaveCount(0);

  // ---- 8. Export a module ------------------------------------------------
  // Last, because it is the one step whose result the browser cannot see:
  // the file lands on the server, inside `CITYLAKE_OUTPUT_ROOT`, and
  // `tests/output_policy.rs` is what checks it lands at the resolved path.
  // What this step covers is the dialog — that its module select is
  // populated from the dataset's object modules (so it can offer
  // `building` at all), that the format select and the path field submit
  // together, and that a 204 paints the success branch rather than the
  // error one.
  await page.keyboard.press("Escape");
  await expect(packageDialog).toBeHidden();

  await main.getByRole("button", { name: "Export…" }).click();
  const exportDialog = page.getByRole("dialog", { name: "Export a module" });
  await expect(exportDialog).toBeVisible();
  await exportDialog.getByLabel("Module").selectOption("building");
  await exportDialog.getByLabel("Format").selectOption("cityjsonseq");
  await exportDialog.getByLabel("Output path").fill(`${DESTINATION}.city.jsonl`);

  const exportResponse = page.waitForResponse(
    (response) =>
      response.url().endsWith(`/api/datasets/${DESTINATION}/export`) &&
      response.request().method() === "POST",
  );
  await exportDialog.getByRole("button", { name: "Export", exact: true }).click();
  expect((await exportResponse).status()).toBe(204);
  await expect(exportDialog.getByText("Export completed.")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(exportDialog).toBeHidden();

  // ---- 9. Clean up -------------------------------------------------------
  // The per-run directory goes at teardown either way; this keeps the two
  // datasets out of the dataset list the other specs read, on a rerun
  // against a catalog that somehow survived.
  for (const ds of [DESTINATION, SOURCE]) {
    await page.goto(`/datasets/${ds}`);
    await page.getByRole("button", { name: "Drop dataset" }).click();
    const dropConfirm = page.getByRole("alertdialog", { name: "Drop dataset?" });
    await expect(dropConfirm).toBeVisible();
    const dropResponse = page.waitForResponse(
      (response) =>
        response.url().endsWith(`/api/datasets/${ds}`) && response.request().method() === "DELETE",
    );
    await dropConfirm.getByRole("button", { name: "Drop", exact: true }).click();
    expect((await dropResponse).ok()).toBe(true);
    await expect(page).toHaveURL(/\/datasets$/);
  }
});
