import { mkdir } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { chromium, expect } from "@playwright/test";

export default async function globalSetup() {
  const statePath = resolve(".auth/user.json");
  await mkdir(dirname(statePath), { recursive: true });
  const browser = await chromium.launch();
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto("http://127.0.0.1:8080/");
  await expect(page.getByRole("heading", { name: "Good to have you back." })).toBeVisible();
  await page.getByLabel("Password").fill("test");
  await page.getByRole("button", { name: "Continue" }).click();
  await expect(page.getByText("Local and private")).toBeVisible();
  const configured = await page.evaluate(async () => {
    const response = await fetch("/api/settings", {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ openrouter_api_key: "test-key", default_model: "test/text" }),
    });
    return response.ok;
  });
  expect(configured).toBeTruthy();
  await page.goto("http://127.0.0.1:8080/");
  await expect(page.getByRole("heading", { name: "What’s on your mind?" })).toBeVisible();
  await context.storageState({ path: statePath });
  await browser.close();
}
