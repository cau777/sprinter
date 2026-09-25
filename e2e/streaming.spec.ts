import { expect, test } from "@playwright/test";

test("reattaches after reload, shows thinking, and stops a reply", async ({ page }) => {
  await page.goto("/");
  const composer = page.getByRole("textbox", { name: "Message" });
  await composer.fill("[[think]] Give me a considered answer.");
  await composer.press("Enter");
  await expect(page.getByText("THINKING…")).toBeVisible();
  await expect(page.getByText("You said: Give me a considered answer.", { exact: true })).toBeVisible({ timeout: 10_000 });

  await composer.fill("[[slow]] Please keep streaming after a reload.");
  await composer.press("Enter");
  await expect(page.locator('.chat-message[data-running="true"]')).toBeVisible();
  await page.reload();
  await expect(page.getByText("You said: Please keep streaming after a reload.", { exact: true })).toBeVisible({ timeout: 15_000 });

  const refreshedComposer = page.getByRole("textbox", { name: "Message" });
  await refreshedComposer.fill("[[slow]] Stop this longer answer.");
  await refreshedComposer.press("Enter");
  await expect(page.getByRole("button", { name: "Stop" })).toBeVisible();
  await page.getByRole("button", { name: "Stop" }).click();
  await expect(page.getByText(/Stopped\. You can retry/)).toBeVisible({ timeout: 5_000 });
});

test("shows provider errors and offers a retry", async ({ page }) => {
  await page.goto("/");
  const composer = page.getByRole("textbox", { name: "Message" });
  await composer.fill("[[error]] Show the provider error.");
  await composer.press("Enter");
  await expect(page.locator(".chat-message-error")).toContainText(/rate limited/i, { timeout: 10_000 });
  await expect(page.getByRole("button", { name: /Retry/ })).toBeVisible();
});
