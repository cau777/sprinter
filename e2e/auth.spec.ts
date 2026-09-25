import { expect, test } from "@playwright/test";

test.use({ storageState: { cookies: [], origins: [] } });

test("rejects a wrong password, then logs in and out", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Good to have you back." })).toBeVisible();

  await page.getByLabel("Password").fill("incorrect password");
  await page.getByRole("button", { name: "Continue" }).click();
  await expect(page.getByRole("alert")).toContainText(/password|invalid|incorrect/i);

  await page.getByLabel("Password").fill("test");
  await page.getByRole("button", { name: "Continue" }).click();
  await expect(page.getByRole("heading", { name: "What’s on your mind?" })).toBeVisible();
  await page.getByRole("button", { name: "Log out" }).click();
  await expect(page.getByRole("heading", { name: "Good to have you back." })).toBeVisible();
});

test("returns to an unauthenticated deep link after login", async ({ page }) => {
  await page.goto("/spike/assistant-ui");
  await expect(page.getByRole("heading", { name: "Good to have you back." })).toBeVisible();
  await page.getByLabel("Password").fill("test");
  await page.getByRole("button", { name: "Continue" }).click();
  await expect(page.getByRole("heading", { name: "Thread runtime lab" })).toBeVisible();
  await expect(page).toHaveURL(/\/spike\/assistant-ui$/);
});
