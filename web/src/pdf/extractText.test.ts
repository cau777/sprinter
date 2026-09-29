import { describe, expect, it } from "vitest";
import { extractPdfText, isPasswordProtectedPdfError, normalizeExtractedText, pdfExtractionErrorMessage } from "./extractText";

function fixturePdf(pageContents: Array<string[]>) {
  const encoder = new TextEncoder();
  const fontId = pageContents.length + 3;
  const contentStartId = fontId + 1;
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    `<< /Type /Pages /Kids [${pageContents.map((_, index) => `${index + 3} 0 R`).join(" ")}] /Count ${pageContents.length} >>`,
    ...pageContents.map((_, index) => `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 ${fontId} 0 R >> >> /Contents ${contentStartId + index} 0 R >>`),
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    ...pageContents.map((lines) => {
      const commands = lines.length
        ? lines.map((line, index) => `BT /F1 16 Tf 72 ${720 - index * 24} Td (${escapePdfString(line)}) Tj ET`).join("\n")
        : "BT /F1 16 Tf 72 720 Td ET";
      return `<< /Length ${encoder.encode(commands).length} >>\nstream\n${commands}\nendstream`;
    }),
  ];
  let source = "%PDF-1.7\n";
  const offsets: number[] = [0];
  for (let index = 0; index < objects.length; index += 1) {
    offsets.push(encoder.encode(source).length);
    source += `${index + 1} 0 obj\n${objects[index]}\nendobj\n`;
  }
  const xrefOffset = encoder.encode(source).length;
  source += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  for (const offset of offsets.slice(1)) source += `${String(offset).padStart(10, "0")} 00000 n \n`;
  source += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xrefOffset}\n%%EOF\n`;
  return new File([encoder.encode(source)], "fixture.pdf", { type: "application/pdf" });
}

function escapePdfString(value: string) {
  return value.replace(/[\\()]/g, "\\$&");
}

describe("PDF text extraction", () => {
  it("adds page headings, uses hasEOL for line breaks, and normalizes NFKC", async () => {
    const progress: Array<{ page: number; pages: number }> = [];
    const result = await extractPdfText(fixturePdf([["first line", "second line"]]), (item) => progress.push(item));

    expect(result.text).toContain("## Page 1\nfirst line\nsecond line");
    expect(result.pages).toBe(1);
    expect(result.empty_pages).toBe(0);
    expect(progress).toEqual([{ page: 1, pages: 1 }]);
    expect(normalizeExtractedText("oﬃce")).toBe("office");
  });

  it("counts pages with fewer than ten non-whitespace characters as empty", async () => {
    const result = await extractPdfText(fixturePdf([["Readable report page"], []]));

    expect(result.pages).toBe(2);
    expect(result.empty_pages).toBe(1);
    expect(result.text).toContain("## Page 2\n");
  });

  it("identifies password-protected PDF errors", () => {
    const error = Object.assign(new Error("password required"), { name: "PasswordException" });
    expect(isPasswordProtectedPdfError(error)).toBe(true);
    expect(pdfExtractionErrorMessage(error)).toBe("Password-protected PDFs aren't supported.");
    expect(pdfExtractionErrorMessage(new Error("broken PDF"))).toBe("Couldn't read this PDF.");
  });
});
