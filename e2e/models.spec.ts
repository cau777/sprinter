import { expect, test } from "@playwright/test";

test("favoriting a model in a new chat saves it and updates the picker", async ({ page }) => {
  await page.goto("/");
  await page.getByText("test/text", { exact: true }).first().click();

  const settingsPatches: string[] = [];
  page.on("response", (response) => {
    if (response.url().endsWith("/api/settings") && response.request().method() === "PATCH") settingsPatches.push(response.url());
  });
  const saveResponse = page.waitForResponse((response) => response.url().endsWith("/api/settings") && response.request().method() === "PATCH");
  await page.getByRole("button", { name: "Add Fake PDF to favorites" }).click();
  const response = await saveResponse;
  expect(response.ok()).toBeTruthy();
  expect(await response.json()).toMatchObject({ favorite_models: ["test/file"], default_model: "test/text" });

  await expect.poll(async () => page.evaluate(async () => {
    const settings = await (await fetch("/api/settings")).json();
    return { favorite_models: settings.favorite_models, default_model: settings.default_model };
  })).toEqual({ favorite_models: ["test/file"], default_model: "test/text" });
  await expect(page.getByRole("button", { name: "Remove Fake PDF from favorites" })).toBeVisible();
  expect(settingsPatches).toHaveLength(1);
});

test("changing a chat model affects later messages but keeps earlier model records", async ({ page }) => {
  await page.goto("/");
  const composer = page.getByRole("textbox", { name: "Message" });
  await composer.fill("Start with the default model.");
  await composer.press("Enter");
  await expect(page.getByText("You said: Start with the default model.", { exact: true })).toBeVisible({ timeout: 15_000 });

  const chatId = new URL(page.url()).pathname.slice(1);
  await page.getByText("test/text", { exact: true }).first().click();
  await page.getByRole("button", { name: "Select Fake PDF (test/file)" }).click();
  await expect.poll(async () => page.evaluate(async (id) => (await (await fetch(`/api/chats/${id}`)).json()).model, chatId)).toBe("test/file");

  await page.getByRole("textbox", { name: "Message" }).fill("Use the new model now.");
  await page.getByRole("textbox", { name: "Message" }).press("Enter");
  await expect(page.getByText("You said: Use the new model now.", { exact: true })).toBeVisible({ timeout: 15_000 });

  const detail = await page.evaluate(async (id) => (await (await fetch(`/api/chats/${id}`)).json()), chatId);
  const assistantMessages = detail.messages.filter((message: { role: string }) => message.role === "assistant");
  expect(assistantMessages.map((message: { model: string }) => message.model)).toEqual(["test/text", "test/file"]);
});
