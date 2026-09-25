import { mkdir } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { chromium, expect } from "@playwright/test";

export default async function globalSetup() {
  const statePath = resolve(".auth/user.json");
  await mkdir(dirname(statePath), { recursive: true });
  const browser = await chromium.launch();
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Good to have you back." })).toBeVisible();
  await page.getByLabel("Password").fill("test");
  await page.getByRole("button", { name: "Continue" }).click();
  await expect(page.getByText("Local and private")).toBeVisible();
  await context.storageState({ path: statePath });
  await browser.close();
}
