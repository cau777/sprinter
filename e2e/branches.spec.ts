import { expect, test } from "@playwright/test";

test("retry preserves assistant siblings and switches the visible branch", async ({ page }) => {
  await page.goto("/");
  const composer = page.getByRole("textbox", { name: "Message" });
  await composer.fill("Keep both answers in this branch.");
  await composer.press("Enter");
  await expect(page.getByText("You said: Keep both answers in this branch.", { exact: true })).toBeVisible({ timeout: 15_000 });

  const initialUrl = new URL(page.url());
  const chatId = initialUrl.pathname.slice(1);
  const first = await page.evaluate(async (id) => (await (await fetch(`/api/chats/${id}`)).json()).current_leaf_id as string, chatId);
  const assistant = page.getByTestId("assistant-message").last();
  await assistant.getByRole("button", { name: "Retry response" }).click();
  await expect(page.getByText("2 / 2", { exact: true })).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId("assistant-message").last()).toHaveAttribute("data-running", "false", { timeout: 15_000 });

  const detail = await page.evaluate(async (id) => (await (await fetch(`/api/chats/${id}`)).json()), chatId);
  expect(detail.messages.filter((message: { role: string }) => message.role === "assistant")).toHaveLength(2);
  expect(detail.model).toBe("test/text");
  expect(detail.messages.find((message: { id: string }) => message.id === detail.current_leaf_id).model).toBe("test/text");

  await page.getByRole("button", { name: /Previous branch for message/ }).click();
  await expect.poll(async () => page.evaluate(async (id) => (await (await fetch(`/api/chats/${id}`)).json()).current_leaf_id, chatId)).toBe(first);
  await expect(page.getByText("1 / 2", { exact: true })).toBeVisible();

  const userMessage = page.getByTestId("user-message").first();
  await userMessage.getByRole("button", { name: "Edit" }).click();
  const editComposer = page.getByPlaceholder("Edit message…");
  await expect(editComposer).toHaveValue("Keep both answers in this branch.");
  await editComposer.fill("Edit this message without deleting its first version.");
  await editComposer.press("Enter");
  await expect(page.getByText("You said: Edit this message without deleting its first version.", { exact: true })).toBeVisible({ timeout: 15_000 });
  await expect(page.getByText("2 / 2", { exact: true })).toBeVisible();
  const edited = await page.evaluate(async (id) => (await (await fetch(`/api/chats/${id}`)).json()), chatId);
  expect(edited.messages.filter((message: { role: string; parent_id: string | null }) => message.role === "user" && message.parent_id === null)).toHaveLength(2);
});
