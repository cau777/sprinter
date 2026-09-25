import { expect, test } from "@playwright/test";

test("renders rich markdown with safe links and copyable code", async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.goto("/");
  await page.getByRole("textbox", { name: "Message" }).fill("[[rich]]");
  await page.getByRole("textbox", { name: "Message" }).press("Enter");

  const answer = page.locator('.chat-message[data-role="assistant"]').last();
  await expect(answer.locator("table")).toBeVisible({ timeout: 15_000 });
  await expect(answer).toHaveAttribute("data-running", "false", { timeout: 15_000 });
  await expect(answer.locator("table th").first()).toHaveText("Name");
  await expect(answer.locator("table td").first()).toHaveText("Sprinter");
  await expect(answer.locator(".katex")).toContainText("E");
  await expect(answer.getByRole("img", { name: "Mermaid diagram" }).locator("svg")).toBeVisible();

  const copy = answer.locator(".markdown-code-header button").first();
  await expect(copy).toBeVisible();
  await copy.click();
  await expect(copy).toHaveText("Copied");
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toContain("println!(\"hello\")");

  const link = answer.getByRole("link", { name: "Sprinter" });
  await expect(link).toHaveAttribute("href", "https://example.com");
  await expect(link).toHaveAttribute("target", "_blank");
  await expect(link).toHaveAttribute("rel", "noopener noreferrer");
});
