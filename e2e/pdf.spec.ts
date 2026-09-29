import { expect, test } from "@playwright/test";

function pdfFixture(lines: string[], filename: string) {
  const content = lines.length
    ? lines.map((line, index) => `BT /F1 16 Tf 72 ${720 - index * 24} Td (${line}) Tj ET`).join("\n")
    : "BT /F1 16 Tf 72 720 Td ET";
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>",
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    `<< /Length ${Buffer.byteLength(content)} >>\nstream\n${content}\nendstream`,
  ];
  let source = "%PDF-1.7\n";
  const offsets = [0];
  for (let index = 0; index < objects.length; index += 1) {
    offsets.push(Buffer.byteLength(source));
    source += `${index + 1} 0 obj\n${objects[index]}\nendobj\n`;
  }
  const xrefOffset = Buffer.byteLength(source);
  source += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  for (const offset of offsets.slice(1)) source += `${String(offset).padStart(10, "0")} 00000 n \n`;
  source += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xrefOffset}\n%%EOF\n`;
  return { name: filename, mimeType: "application/pdf", buffer: Buffer.from(source) };
}

async function selectChatModel(page: import("@playwright/test").Page, modelId: "test/file") {
  const picker = page.getByRole("button", { name: "Choose model, current test/text" });
  await picker.click();
  await page.getByRole("button", { name: `Select Fake PDF (${modelId})` }).click();
}

async function lastRequestContaining(page: import("@playwright/test").Page, marker: string) {
  await expect.poll(async () => {
    const response = await page.request.get("http://127.0.0.1:4010/__requests");
    const requests = await response.json() as Array<{ path: string; body: Record<string, unknown> }>;
    return requests.some((request) => request.path === "/api/v1/chat/completions" && JSON.stringify(request.body).includes(marker));
  }, { timeout: 15_000 }).toBe(true);
  const response = await page.request.get("http://127.0.0.1:4010/__requests");
  const requests = await response.json() as Array<{ path: string; body: Record<string, unknown> }>;
  return requests.findLast((request) => request.path === "/api/v1/chat/completions" && JSON.stringify(request.body).includes(marker))!.body;
}

test("PDF attachments send extracted text, use native file fallback for scans, and omit unsupported scans", async ({ page }) => {
  await page.goto("/");

  const reportMarker = "PDF_E2E_EXTRACTED_REPORT_8731";
  const uploadResponsePromise = page.waitForResponse((response) => response.url().endsWith("/api/uploads") && response.request().method() === "PUT");
  await page.locator('input[type="file"]').setInputFiles(pdfFixture([reportMarker], "report.pdf"));
  const uploadResponse = await uploadResponsePromise;
  expect(uploadResponse.status(), await uploadResponse.text()).toBe(201);
  await expect(page.getByText(/~\d+ tokens/)).toBeVisible();
  const composer = page.getByRole("textbox", { name: "Message" });
  await composer.fill("Summarize the attached report [pdf-e2e-text]");
  await composer.press("Enter");
  const textBody = await lastRequestContaining(page, "pdf-e2e-text");
  expect(JSON.stringify(textBody)).toContain(reportMarker);
  expect(JSON.stringify(textBody)).not.toContain('"type":"file"');
  expect(JSON.stringify(textBody.plugins)).not.toContain("file-parser");

  const scanMarker = "pdf-e2e-native-scan";
  await page.goto("/");
  const primer = page.getByRole("textbox", { name: "Message" });
  await primer.fill("Start a chat for the scanned PDF test.");
  await primer.press("Enter");
  await expect(page.getByText("You said: Start a chat for the scanned PDF test.", { exact: true })).toBeVisible({ timeout: 15_000 });
  await selectChatModel(page, "test/file");
  await expect.poll(async () => page.evaluate(async () => {
    const chatId = location.pathname.slice(1);
    return (await (await fetch(`/api/chats/${chatId}`)).json()).model;
  })).toBe("test/file");
  await page.locator('input[type="file"]').setInputFiles(pdfFixture([], "scanned.pdf"));
  await expect(page.getByText("No text found (scanned?)")).toBeVisible();
  await page.getByRole("textbox", { name: "Message" }).fill(`Describe this scan [${scanMarker}]`);
  await page.getByRole("textbox", { name: "Message" }).press("Enter");
  const nativeBody = await lastRequestContaining(page, scanMarker);
  const nativeJson = JSON.stringify(nativeBody);
  expect(nativeJson).toContain('"type":"file"');
  expect(nativeBody.plugins).toContainEqual({ id: "file-parser", pdf: { engine: "native" } });

  const omittedMarker = "pdf-e2e-omitted-scan";
  await page.goto("/");
  await page.locator('input[type="file"]').setInputFiles(pdfFixture([], "unsupported-scan.pdf"));
  await expect(page.getByText("No text found (scanned?)")).toBeVisible();
  await expect(page.getByText("A scanned PDF has no extractable text and will be omitted because this model cannot read PDF files.")).toBeVisible();
  await page.getByRole("textbox", { name: "Message" }).fill(`Describe this scan [${omittedMarker}]`);
  await page.getByRole("textbox", { name: "Message" }).press("Enter");
  const omittedBody = await lastRequestContaining(page, omittedMarker);
  const omittedJson = JSON.stringify(omittedBody);
  expect(omittedJson).toContain("PDF omitted: unsupported-scan.pdf");
  expect(omittedJson).not.toContain('"type":"file"');
  expect(JSON.stringify(omittedBody.plugins)).not.toContain("file-parser");
});
