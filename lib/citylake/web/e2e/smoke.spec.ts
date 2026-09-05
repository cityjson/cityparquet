import { expect, test } from "@playwright/test";

test("the datasets page renders past the login gate", async ({ page }) => {
  const datasetsResponse = page.waitForResponse(
    (response) => response.url().includes("/api/datasets") && response.request().method() === "GET",
  );

  await page.goto("/datasets");

  // Reaching this heading proves the bypass satisfied ProtectedRoute and the
  // client booted. It does not, on its own, prove the API answered: the
  // heading sits in DatasetsPage's header, outside the loading/error/success
  // branches, so it renders before the dataset query even settles. The
  // response assertion below is what actually proves the request succeeded
  // — an empty list still requires one.
  await expect(page.getByRole("heading", { name: "Datasets", exact: true })).toBeVisible();
  await expect(page).toHaveURL(/\/datasets$/);

  const response = await datasetsResponse;
  expect(response.ok()).toBe(true);

  // `waitForResponse` resolves on headers, not on the query settling into
  // React state, so a 2xx here does not yet mean DatasetsPage's success
  // branch has rendered — and a 200 with a wrong-shaped body would let the
  // query *succeed* while the component crashes in render (`datasets.map`,
  // which the `error &&` branch does not catch). Anchor on what the success
  // branch actually produces: the loading skeletons gone, and either the
  // dataset grid or the empty-state card standing in their place.
  const main = page.getByRole("main");
  await expect(main.locator(".animate-pulse")).toHaveCount(0);
  const emptyState = main.getByText("No datasets yet.");
  const datasetCard = main.locator('a[href^="/datasets/"]').first();
  await expect(emptyState.or(datasetCard)).toBeVisible();

  // Belt-and-braces, now evaluated at a meaningful moment: the query has
  // demonstrably settled on success above, so this checks that it did not
  // also settle on failure, rather than passing instantly against a page
  // that has not rendered yet.
  await expect(page.getByText(/Failed to load datasets/)).toHaveCount(0);
});
