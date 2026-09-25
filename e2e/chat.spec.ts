import { expect, test } from "@playwright/test";

test("starts a chat, streams a reply, and reloads the saved thread", async ({ page }) => {
  await page.goto("/");
  const composer = page.getByRole("textbox", { name: "Message" });
  await composer.fill("Hello from a saved conversation.");
  await composer.press("Enter");

  await expect(page.getByText("Hello from a saved conversation.", { exact: true })).toBeVisible();
  await expect(page.getByText("You said: Hello from a saved conversation.", { exact: true })).toBeVisible({ timeout: 15_000 });
  await expect(page).toHaveURL(/\/.+$/);

  const url = page.url();
  await page.reload();
  await expect(page.getByText("You said: Hello from a saved conversation.", { exact: true })).toBeVisible();
  await expect(page).toHaveURL(url);
});
