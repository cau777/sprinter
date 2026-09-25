import { expect, test } from "@playwright/test";

test("changing a chat model affects later messages but keeps earlier model records", async ({ page }) => {
  await page.goto("/");
  const composer = page.getByRole("textbox", { name: "Message" });
  await composer.fill("Start with the default model.");
  await composer.press("Enter");
  await expect(page.getByText("You said: Start with the default model.", { exact: true })).toBeVisible({ timeout: 15_000 });

  const chatId = new URL(page.url()).pathname.slice(1);
  await page.locator(".chat-composer .model-trigger").click();
  await page.getByRole("option", { name: /Fake Vision test\/vision/ }).click();
  await expect.poll(async () => page.evaluate(async (id) => (await (await fetch(`/api/chats/${id}`)).json()).model, chatId)).toBe("test/vision");

  await page.getByRole("textbox", { name: "Message" }).fill("Use the new model now.");
  await page.getByRole("textbox", { name: "Message" }).press("Enter");
  await expect(page.getByText("You said: Use the new model now.", { exact: true })).toBeVisible({ timeout: 15_000 });

  const detail = await page.evaluate(async (id) => (await (await fetch(`/api/chats/${id}`)).json()), chatId);
  const assistantMessages = detail.messages.filter((message: { role: string }) => message.role === "assistant");
  expect(assistantMessages.map((message: { model: string }) => message.model)).toEqual(["test/text", "test/vision"]);
});
