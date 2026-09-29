import type { PDFDocumentProxy } from "pdfjs-dist";
import type { TextItem } from "pdfjs-dist/types/src/display/api";

export const PDF_TEXT_EXTRACTOR = "pdfjs-6.3.289";

export type ExtractedPdfText = {
  text: string;
  chars: number;
  pages: number;
  empty_pages: number;
};

export type PdfPageProgress = { page: number; pages: number };

export function normalizeExtractedText(text: string) {
  return text.normalize("NFKC");
}

export function isPasswordProtectedPdfError(error: unknown) {
  return typeof error === "object" && error !== null && "name" in error && error.name === "PasswordException";
}

export function pdfExtractionErrorMessage(error: unknown) {
  return isPasswordProtectedPdfError(error)
    ? "Password-protected PDFs aren't supported."
    : "Couldn't read this PDF.";
}

export async function extractPdfText(
  file: File,
  onProgress?: (progress: PdfPageProgress) => void,
  signal?: AbortSignal,
): Promise<ExtractedPdfText> {
  const pdfjs = await import("pdfjs-dist/legacy/build/pdf.mjs");
  pdfjs.GlobalWorkerOptions.workerSrc = typeof window === "undefined"
    ? new URL("../../../node_modules/pdfjs-dist/legacy/build/pdf.worker.mjs", import.meta.url).toString()
    : "/pdfjs/pdf.worker.min.mjs";
  const options = {
    data: new Uint8Array(await file.arrayBuffer()),
    isEvalSupported: false,
    disableFontFace: true,
    useSystemFonts: false,
    cMapUrl: "/pdfjs/cmaps/",
    cMapPacked: true,
    standardFontDataUrl: "/pdfjs/standard_fonts/",
  };
  const loadingTask = pdfjs.getDocument(options as Parameters<typeof pdfjs.getDocument>[0]);
  const abort = () => { void loadingTask.destroy(); };
  signal?.addEventListener("abort", abort, { once: true });
  let document: PDFDocumentProxy | undefined;
  try {
    document = await loadingTask.promise;
    const pageTexts: string[] = [];
    let emptyPages = 0;
    for (let pageNumber = 1; pageNumber <= document.numPages; pageNumber += 1) {
      if (signal?.aborted) throw new DOMException("PDF extraction was cancelled", "AbortError");
      const page = await document.getPage(pageNumber);
      try {
        const content = await page.getTextContent();
        let pageText = "";
        for (const item of content.items) {
          if (!isTextItem(item)) continue;
          pageText += item.str;
          if (item.hasEOL) pageText += "\n";
        }
        pageText = normalizeExtractedText(pageText);
        if (pageText.replace(/\s/g, "").length < 10) emptyPages += 1;
        pageTexts.push(`## Page ${pageNumber}\n${pageText}`);
      } finally {
        page.cleanup();
      }
      onProgress?.({ page: pageNumber, pages: document.numPages });
    }
    const text = normalizeExtractedText(pageTexts.join("\n\n"));
    return { text, chars: text.length, pages: document.numPages, empty_pages: emptyPages };
  } finally {
    signal?.removeEventListener("abort", abort);
    await loadingTask.destroy();
  }
}

function isTextItem(item: unknown): item is TextItem {
  return typeof item === "object" && item !== null && "str" in item && "hasEOL" in item;
}
