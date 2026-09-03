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
  // — an empty list still requires one — and the error-card check rules out
  // the query having settled on failure instead.
  await expect(page.getByRole("heading", { name: "Datasets", exact: true })).toBeVisible();
  await expect(page).toHaveURL(/\/datasets$/);

  const response = await datasetsResponse;
  expect(response.ok()).toBe(true);

  await expect(page.getByText(/Failed to load datasets/)).toHaveCount(0);
});
