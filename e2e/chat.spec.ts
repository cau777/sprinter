import { expect, test } from "@playwright/test";

test("starts a chat, streams a reply, and reloads the saved thread", async ({ page }) => {
  await page.goto("/");
  const composer = page.getByRole("textbox", { name: "Message" });
  await composer.fill("Hello from a saved conversation.");
  await composer.press("Enter");

  await expect(page.getByTestId("user-message").getByText("Hello from a saved conversation.", { exact: true })).toBeVisible();
  await expect(page.getByText("You said: Hello from a saved conversation.", { exact: true })).toBeVisible({ timeout: 15_000 });
  await expect(page).toHaveURL(/\/.+$/);

  const url = page.url();
  await page.reload();
  await expect(page.getByText("You said: Hello from a saved conversation.", { exact: true })).toBeVisible();
  await expect(page).toHaveURL(url);

  await expect(page.getByRole("link", { name: "Hello from a saved conversation." })).toBeVisible();
  await page.getByRole("button", { name: "Rename Hello from a saved conversation." }).click();
  await expect(page.getByRole("heading", { name: "Rename conversation" })).toBeVisible();
  await page.getByRole("textbox", { name: "Conversation title" }).fill("A renamed conversation");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByRole("link", { name: "A renamed conversation" })).toBeVisible();
  await page.getByRole("button", { name: "Delete A renamed conversation" }).click();
  await expect(page.getByRole("heading", { name: "Delete conversation?" })).toBeVisible();
  await page.getByRole("button", { name: "Delete", exact: true }).click();
  await expect(page.getByRole("link", { name: "A renamed conversation" })).toHaveCount(0);
});
