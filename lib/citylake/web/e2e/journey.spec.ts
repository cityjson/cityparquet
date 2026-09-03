import path from "node:path";
import { fileURLToPath } from "node:url";

import { expect, test } from "@playwright/test";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// `lib/citylake/tests/data/delft.city.jsonl` — three flat Buildings (no
// parent/child hierarchy), EPSG:7415, all routed to the `building` module.
// Resolved from this file rather than the working directory, per the brief.
const FIXTURE_PATH = path.resolve(__dirname, "../../tests/data/delft.city.jsonl");

/**
 * A name unique to this run. The catalog is fresh per Playwright run (see
 * `playwright.config.ts`'s `runDir`), but a name collision against a
 * surviving catalog would fail the create with a confusing 409, so this
 * still needs to be unique on its own. Base-36 rather than decimal so the
 * digits `7415` (the fixture's EPSG code, asserted on later) are unlikely to
 * land inside the name and create a second match for that assertion.
 * `[a-zA-Z0-9_]+` is both the client's validation pattern and the server's.
 */
const DATASET_NAME = `journey_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 8)}`;

test("upload, list, open, browse, delete, drop", async ({ page }) => {
  // ---- 1. Upload and create -------------------------------------------
  // Proves the multipart path and that the API ingests.
  const uploadResponse = page.waitForResponse(
    (response) =>
      response.url().endsWith(`/api/datasets/${DATASET_NAME}/upload`) &&
      response.request().method() === "POST",
  );

  await page.goto("/upload");
  await page.getByLabel("City file").setInputFiles(FIXTURE_PATH);
  await page.getByLabel("Dataset name").fill(DATASET_NAME);
  // Scoped to `<main>`: AppShell's persistent header carries its own
  // icon-only "Upload" shortcut button, which the unscoped role query also
  // matches.
  await page.getByRole("main").getByRole("button", { name: "Upload", exact: true }).click();

  const uploadResult = await uploadResponse;
  expect(uploadResult.ok()).toBe(true);
  await expect(page).toHaveURL(new RegExp(`/datasets/${DATASET_NAME}$`));

  // ---- 2. It appears in the list ---------------------------------------
  // Proves the create invalidated the list query (cache-key defect piece B).
  const listResponse = page.waitForResponse(
    (response) => response.url().endsWith("/api/datasets") && response.request().method() === "GET",
  );
  await page.goto("/datasets");
  expect((await listResponse).ok()).toBe(true);

  // Scoped to `<main>`: the sidebar also carries a nav link per dataset name
  // (`AppShell` reads the same `["datasets"]` query), which the unscoped
  // role query also matches.
  const datasetLink = page.getByRole("main").getByRole("link", { name: new RegExp(DATASET_NAME) });
  await expect(datasetLink).toBeVisible();

  // ---- 3. Open it --------------------------------------------------------
  // Proves describeDataset, and that the CRS the extension minted survived
  // to the interface.
  const describeResponse = page.waitForResponse(
    (response) =>
      response.url().endsWith(`/api/datasets/${DATASET_NAME}`) &&
      response.request().method() === "GET",
  );
  await datasetLink.click();
  expect((await describeResponse).ok()).toBe(true);

  await expect(page.getByRole("heading", { name: DATASET_NAME, level: 1 })).toBeVisible();

  const buildingRow = page.getByRole("row", { name: /building/ });
  await expect(buildingRow).toBeVisible();
  await expect(buildingRow.getByRole("cell", { name: "3", exact: true })).toBeVisible();
  // Scoped to the page's own header (not AppShell's persistent top bar,
  // also a `<header>`), so this can't accidentally match a `7415` that
  // landed inside the run-unique dataset name.
  await expect(page.getByRole("main").locator("header").getByText(/7415/)).toBeVisible();

  // ---- 4. Browse the module ----------------------------------------------
  // Proves the query path and the bare-array shape.
  const moduleObjectsResponse = page.waitForResponse(
    (response) =>
      response.url().includes(`/api/datasets/${DATASET_NAME}/modules/building/objects`) &&
      response.request().method() === "GET",
  );
  await buildingRow.getByRole("link", { name: "Open" }).click();
  expect((await moduleObjectsResponse).ok()).toBe(true);

  await expect(page.getByRole("heading", { name: "building", level: 1 })).toBeVisible();
  await expect(page.getByText("3 rows")).toBeVisible();

  const fixtureIds = [
    "NL.IMBAG.Pand.0503100000019695",
    "NL.IMBAG.Pand.0503100000019696",
    "NL.IMBAG.Pand.0503100000019697",
  ];
  for (const id of fixtureIds) {
    // A row, not a cell: `id` and `feature_id` hold the same value for a
    // flat top-level object, so both preferred columns render it and a
    // cell-scoped lookup resolves to two elements.
    await expect(page.getByRole("row", { name: new RegExp(id) })).toBeVisible();
  }
  // One header row plus one per fixture object — rules out extra or missing
  // rows that individual id checks alone would not catch.
  await expect(page.getByRole("row")).toHaveCount(fixtureIds.length + 1);

  // ---- 5. Delete an object -----------------------------------------------
  // Proves the delete path and its cascade reporting.
  const deleteButton = page.getByRole("button", { name: /^Delete / }).first();
  const deleteButtonLabel = await deleteButton.getAttribute("aria-label");
  if (!deleteButtonLabel) throw new Error("delete button has no aria-label");
  const deletedId = deleteButtonLabel.replace(/^Delete /, "");
  const survivingIds = fixtureIds.filter((id) => id !== deletedId);

  await deleteButton.click();
  await expect(page.getByRole("alertdialog", { name: "Delete object?" })).toBeVisible();

  const deleteObjectResponse = page.waitForResponse(
    (response) =>
      response.url().endsWith(`/api/datasets/${DATASET_NAME}/objects/${deletedId}`) &&
      response.request().method() === "DELETE",
  );
  await page.getByRole("alertdialog").getByRole("button", { name: "Delete", exact: true }).click();

  expect((await deleteObjectResponse).ok()).toBe(true);
  await expect(page.getByText("Deleted 1 object (including children).")).toBeVisible();
  await expect(page.getByText("2 rows")).toBeVisible();
  await expect(page.getByRole("row", { name: new RegExp(deletedId) })).toHaveCount(0);
  for (const id of survivingIds) {
    await expect(page.getByRole("row", { name: new RegExp(id) })).toBeVisible();
  }

  // ---- 6. Drop the dataset ------------------------------------------------
  // Proves the drop and its invalidation.
  await page.getByRole("link", { name: `Back to ${DATASET_NAME}` }).click();
  await expect(page.getByRole("heading", { name: DATASET_NAME, level: 1 })).toBeVisible();

  await page.getByRole("button", { name: "Drop dataset" }).click();
  await expect(page.getByRole("alertdialog", { name: "Drop dataset?" })).toBeVisible();

  const dropResponse = page.waitForResponse(
    (response) =>
      response.url().endsWith(`/api/datasets/${DATASET_NAME}`) &&
      response.request().method() === "DELETE",
  );
  const listAfterDropResponse = page.waitForResponse(
    (response) => response.url().endsWith("/api/datasets") && response.request().method() === "GET",
  );
  await page.getByRole("alertdialog").getByRole("button", { name: "Drop", exact: true }).click();

  expect((await dropResponse).ok()).toBe(true);
  await expect(page).toHaveURL(/\/datasets$/);
  expect((await listAfterDropResponse).ok()).toBe(true);
  await expect(page.getByText(/Failed to load datasets/)).toHaveCount(0);
  await expect(
    page.getByRole("main").getByRole("link", { name: new RegExp(DATASET_NAME) }),
  ).toHaveCount(0);
});
