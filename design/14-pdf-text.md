# 14: PDF text extraction in the browser

Status: **Proposed**

PDFs are turned into text **in the browser, with pdf.js**, when they're attached. The
server stores the original PDF as it does today, plus the extracted text. Prompts carry the
text. The OpenRouter `file-parser` plugin with `cloudflare-ai` or `mistral-ocr` is no
longer used, so PDFs aren't sent to Cloudflare or Mistral. A PDF with no extractable text
(a scan) is sent to the model as a file only when the model can read PDFs itself.

This replaces the PDF parts of [02-files.md](02-files.md) (engines, parse cache) and the
PDF engine setting in [07-settings.md](07-settings.md).
[15-messages-api-migration.md](15-messages-api-migration.md) and
[16-openrouter-tools.md](16-openrouter-tools.md) assume this is done.

## Why

- **One less third party.** With `cloudflare-ai`, every new PDF went to Cloudflare Workers
  AI through OpenRouter. OpenRouter's ZDR routing doesn't cover plugins, so this data flow
  had no retention guarantee. Now the PDF stays on the user's server and only its text goes
  to the model provider.
- **`cloudflare-ai` was only doing text extraction.** For PDFs, Cloudflare's `toMarkdown`
  extracts the metadata and the text of each page as-is. For tagged PDFs it also builds
  Markdown from the structure tree. It doesn't do OCR and it ignores images, so local
  extraction loses nothing that `cloudflare-ai` gave us.
- **No parse cache that depends on the API.** The parse cache relied on Chat Completions
  returning the parsed file as a `file` annotation. The Anthropic Messages API returns
  nothing (checked 2026-09-29), so the cache wouldn't survive the move to it in
  [15-messages-api-migration.md](15-messages-api-migration.md). Local text works with
  every API.
- **Accepted loss: OCR.** `mistral-ocr` is gone. Scanned PDFs only work with models that
  read PDFs natively.

## Decisions

| Area | Decision |
|---|---|
| Where | In the browser, with `pdfjs-dist` (pdf.js) in its own Web Worker. Loaded on demand the first time a PDF is attached. |
| Why not the server | The Rust extractors peak at 230–460 MB of memory on PDFs of 5 MB and up (see [Benchmarks](#benchmarks)). That breaks the server's budget from [04-backend.md](04-backend.md), and an out-of-memory kill would stop every running generation. |
| Original PDF | Still uploaded and stored, unchanged. It's needed for **Open** on the chip across devices, for the scanned-PDF fallback, for re-extracting with a better extractor later, and for backups. |
| Text storage | A file next to the PDF, `uploads/<sha[0..2]>/<sha>.txt`, so identical PDFs share one extraction. |
| Send gating | Send waits for both the PDF upload and the extraction, like it already waits for uploads. There's no background upload. |
| Token estimate | The chip shows `~N tokens` (characters ÷ 4) and warns when that's more than the chat model's context length. |
| Scanned PDF | If the model supports `file` input, the PDF is sent with the `file-parser` plugin pinned to `native`. Otherwise the send is blocked with an explanation. |
| PDF engine setting | Removed from Settings, the composer and the API. |

## Client

### Extraction

`web/src/pdf/extractText.ts` lazy-imports `pdfjs-dist` and sets `GlobalWorkerOptions.workerSrc`
to the bundled `pdf.worker.min.mjs`. The parsing runs in pdf.js's worker, so the page
stays responsive.

```ts
getDocument({
  data,                       // the File's ArrayBuffer
  isEvalSupported: false,     // CSP has no 'unsafe-eval'
  disableFontFace: true,      // text only, no rendering
  useSystemFonts: false,
  cMapUrl: "/pdfjs/cmaps/", cMapPacked: true,
  standardFontDataUrl: "/pdfjs/standard_fonts/",
})
```

- **Page by page:**
  - For each page, `getTextContent()` is called and the item strings are joined, with a
    newline wherever an item has `hasEOL`.
  - `page.cleanup()` runs after each page, so memory stays close to the size of one page
    plus the text so far.
- **Output format**, one heading per page:

  ```
  ## Page 1
  …text…

  ## Page 2
  …
  ```

- **Unicode:** the text is normalized with `String.prototype.normalize("NFKC")`. pdf.js
  returns Arabic presentation forms and ligatures (`ﬁ`) as separate code points, and
  normalizing turns them back into ordinary text.
- **Empty pages:** a page with fewer than 10 non-whitespace characters counts as empty.
  The client reports `pages` and `empty_pages` with the text.
- **Errors:**
  - A password-protected PDF (`PasswordException`) is rejected: "Password-protected PDFs
    aren't supported."
  - Any other pdf.js error removes the attachment with "Couldn't read this PDF", and the
    error is sent to `/api/client-log` without any content.
- **Assets:**
  - Vite copies `pdfjs-dist/cmaps` and `standard_fonts` to `/pdfjs/`.
  - Sizes: 128 KB gzipped for the main module and 365 KB for the worker (pdf.js 6.3). CMaps
    are 1.7 MB, but only the ones a PDF needs are fetched.
  - They're kept out of the PWA precache (`globIgnores`) and cached at runtime
    (cache-first) after the first use. Attaching is already disabled offline.
- **CSP:** unchanged. `worker-src 'self'` covers the worker, and `isEvalSupported: false`
  avoids `eval`.

### Attach flow

```
user attaches report.pdf
   ├─ PUT /api/uploads (existing, streamed)   ──► {id, kind:"pdf", text: null | {…}}
   └─ extractText(file) in the worker         (skipped if the upload response already has text)
        ▼  both done
PUT /api/uploads/:id/text  (text/plain; UTF-8)  ──► {chars, pages, empty_pages}
        ▼
chip ready, Send enabled
```

- If an identical PDF (same SHA-256) was extracted before, the upload response already
  includes `text`, and the client skips extraction.
- Extraction and upload run in parallel, and Send stays disabled until both are done and
  the text is stored.
- If the text upload fails, the chip shows the error with **Retry**, and Send stays
  disabled.

### Chip

| State | Chip shows |
|---|---|
| Extracting | Filename, a progress bar while uploading, and "Reading page 12 of 40…" |
| Ready | Filename · size · `~12k tokens` |
| Too big for the model | `~310k tokens` in the warning color, with the tooltip "Larger than {model}'s 200k context". Sending is still allowed, and the provider's error explains it. |
| Some pages empty | `~9k tokens · 3 of 20 pages have no text`. The tooltip says they may be scans, which aren't read. |
| No text | "No text found (scanned?)". The tooltip explains the fallback below, based on the chat's current model. |

- **Estimate:** `ceil(chars / 4)`, shown as `~Nk`. It's only a rough figure, since real
  counts depend on the model's tokenizer.
- **Context length:** compared with the `context_length` from `GET /api/models`, and
  re-checked when the chat's model changes.
- **Already-sent attachments:** the chip gets `chars` from the upload record, so the
  estimate also shows on attachments that are pre-filled when editing a message.

## Server

### Storing text

`PUT /api/uploads/:id/text`:
- A raw `text/plain` body with `X-Sprinter: 1`, the same CSRF rule as uploads, plus
  `X-Pdf-Pages` and `X-Pdf-Empty-Pages` headers.
- **Only for `kind = 'pdf'` uploads.** Other kinds return `409`.
- The body is streamed to `tmp/` and checked as UTF-8 with no NUL bytes, then renamed to
  `uploads/<sha[0..2]>/<sha>.txt`. It's never held whole in memory.
- The size cap is **16 MB**, a hard ceiling. Anything larger returns `413`. For scale, an
  88 MB test PDF produced 5.4 MB of text.
- If a text file already exists for that SHA-256, it's replaced. Extraction is
  deterministic for the same file and extractor version, so this is harmless.
- It updates the `uploads` row (below) and returns `{chars, pages, empty_pages}`.

### Prompt assembly

`uploads.rs`, replacing the `file` part, parse cache and plugin logic:

| PDF state | Content part sent |
|---|---|
| Text with at least one non-empty page | A text part: `Attached PDF: report.pdf (40 pages, text extracted)` followed by the text in a fence, using the same fence-length rule as text files. |
| No text (`empty_pages = pages`) and the model's `input_modalities` has `file` | A `file` part with the base64 PDF, and the request gets `plugins: [{id: "file-parser", pdf: {engine: "native"}}]`. |
| No text and the model can't read files | A text part: `[PDF omitted: scan.pdf. No text could be extracted and the current model can't read PDFs]`. The composer shows a one-line warning, like the existing image rule. |
| Text never stored (an upload from before this change) | The send returns `409 pdf_text_missing` with the upload ids. See [Existing PDFs](#existing-pdfs). |

- The `native` engine is **always set explicitly** on the fallback. Without an engine,
  OpenRouter falls back to `mistral-ocr` for models that can't read files, which would
  bring the third party back.
- Text parts work the same way on Chat Completions and the Messages API. After
  [15-messages-api-migration.md](15-messages-api-migration.md), the native fallback
  becomes a `document` block with a base64 PDF source.
- **Memory:** reading a text file into the prompt costs far less than base64-encoding the
  PDF did (5.4 MB of text versus about 117 MB of base64 for the 88 MB PDF). The old
  worst-case peak only applies now to scanned PDFs on the native fallback.
- The `file-parser` plugin is only added for the native fallback.
  `context-compression: disabled` stays on every request.

### Existing PDFs

Uploads from before this change have no text. When a send or regenerate includes one:
1. The server returns `409 pdf_text_missing` with `{upload_ids}`.
2. The client downloads each PDF from `GET /api/uploads/:id`, extracts and stores its
   text, then retries the request automatically. A toast says "Reading 2 earlier PDFs…".
3. If extraction fails, the error is shown and the send isn't retried.

The same path covers anything that reaches the server without text. The old
`parse_cache` annotations aren't converted: one path for all missing text is simpler, and
extraction is fast.

## Data model changes

```sql
ALTER TABLE uploads ADD COLUMN text_chars INTEGER;          -- NULL = no text stored yet
ALTER TABLE uploads ADD COLUMN text_pages INTEGER;
ALTER TABLE uploads ADD COLUMN text_empty_pages INTEGER;
ALTER TABLE uploads ADD COLUMN text_extractor TEXT;         -- e.g. 'pdfjs-6.3.289'

ALTER TABLE message_attachments DROP COLUMN pdf_engine;
ALTER TABLE message_attachments DROP COLUMN parse_cache;
```

- The `pdf_engine` settings key is deleted in the same migration.
- `text_extractor` records which extractor produced the text. If the extractor improves
  later (for example Markdown from `getStructTree()`), a change in the client's version can
  trigger re-extraction, because the original PDF is still stored.
- **GC:** the upload GC in [02-files.md](02-files.md) deletes `<sha>.txt` together with
  `<sha>` when no upload row references that SHA-256.
- Extracted text is **not** indexed in `search_fts`. PDFs weren't searchable before
  either.

## API changes

| Endpoint | Change |
|---|---|
| `PUT /api/uploads` | For PDFs, the response gains `text: {chars, pages, empty_pages} \| null`. |
| `PUT /api/uploads/:id/text` | New. Described above. |
| `GET /api/chats/:id` | Attachments gain `text_chars`, `text_pages`, `text_empty_pages`. |
| `POST /api/chats/:id/messages`, `/api/chats/new/messages` | `pdf_engine` is removed. A request that still sends it gets `400`. New error `409 pdf_text_missing`. |
| `POST /api/messages/:id/regenerate` | New error `409 pdf_text_missing`. |
| `GET/PATCH /api/settings` | `pdf_engine` is removed. |

## Code removed

- `cache_pdf_annotations`, and the `file` annotation handling in `generation.rs`.
- The PDF engine selector in the composer, and the Settings field.
- The `file-parser` engines `cloudflare-ai` and `mistral-ocr`, from the request builder,
  the fake OpenRouter and the tests.

## Logging

| Event | Level | Fields |
|---|---|---|
| PDF text stored | INFO | `upload`, `chars`, `pages`, `empty_pages`, `bytes`, `extractor` |
| PDF text rejected | WARN | `upload`, `reason=too_large\|not_utf8\|not_pdf` |
| Generation started | INFO | adds `pdf_text=N`, `pdf_native=M`, `pdf_omitted=K` |
| PDF text missing | INFO | `uploads=[…]`, from the send or regenerate that returned `409` |
| Client extraction failed | via `/api/client-log` | `error_name`, `pages`, `size`. Never any text content. |

## Testing

- **Web unit tests (Vitest)**, running pdf.js's legacy build in Node:
  - Page headings, `hasEOL` newlines, and NFKC normalization, using a small fixture PDF.
  - Counting empty pages on a fixture scan.
  - Rejecting password-protected PDFs.
  - The token estimate and the context-length warning.
- **Backend:**
  - The `PUT /api/uploads/:id/text` rules: PDFs only, UTF-8, the size cap, `tmp/` then
    rename, and sharing by SHA-256.
  - Prompt assembly for each row of the table above, including `engine: "native"` on the
    fallback.
  - `409 pdf_text_missing`.
  - GC deleting `.txt` files.
- **E2E (update the `files` spec):**
  - Attaching a PDF shows `~N tokens`, and the fake OpenRouter receives a text part with
    the PDF's content and no `file` part or plugin engine.
  - A scanned fixture with a `file`-capable model sends a `file` part with
    `engine: "native"`. With a text-only model, it shows the omitted warning.
  - An upload created without text goes through `409`, extraction and the automatic retry.
  - The old PDF engine selector tests are removed.

## Benchmarks

Measured on 2026-09-29 on the development VM: peak resident memory and wall time to
extract all text. Rust ran as a release build with mimalloc, and pdf.js 6.3.289 ran in
Node 22. Node's own baseline is about 43 MB, and browser tabs differ, so the pdf.js
column is indicative. The test files were real documents (a 14-page paper, the IPCC AR6
report, the 756-page ISO 32000 spec), a 14-page scanned-style file, and combinations
reaching the default 50 MB limit and close to the 100 MB ceiling.

| PDF | pdf-extract 0.12 | pdf_oxide 0.3 | pdf.js 6.3 |
|---|---|---|---|
| 1 MB, 14-page paper | 51 MB · 0.04 s | 54 MB · 0.08 s | 145 MB · 0.2 s |
| 5 MB, multi-column report | 233 MB · 0.2 s | 124 MB · 0.3 s | 181 MB · 0.4 s |
| 21 MB, 756-page spec | 319 MB · 1.0 s | 366 MB · 1.6 s | 265 MB · 2.0 s |
| 65 MB, 14 scanned pages | 149 MB · 0 chars | 247 MB · 14 chars | 235 MB · 28 chars |
| 50 MB mixed | 367 MB · 1.5 s | 319 MB · 3.0 s | 361 MB · 3.9 s |
| 88 MB mixed | 464 MB · 2.1 s | 435 MB · 4.6 s | 420 MB · 6.1 s |

- **Server:** both Rust libraries exceed the budget in [04-backend.md](04-backend.md)
  (under 100 MB in normal use, peaks around 200 MB) from about 5 MB PDFs upwards. Running
  them in a child process with a memory limit would protect the main process, but the host
  would still need about 450 MB at peak.
- **pdf_oxide 0.3.78 didn't build.** Its dependency `office_oxide` 0.1.12 changed a public
  struct in a patch release. It also put footnotes in the middle of sentences in the IPCC
  report. pdf-extract and pdf.js both kept the reading order.
- **Client:** pdf.js peaks are similar to the Rust ones, but they happen in the user's tab.
  If a phone runs out of memory, only the extraction fails, and the chip says so. The
  server is unaffected.

## Open questions

1. **Structured Markdown.** pdf.js exposes `getStructTree()` for tagged PDFs. Using it
   would give headings and tables, like `cloudflare-ai` did for tagged files. It's deferred,
   and `text_extractor` makes it possible to re-extract later.
2. **Very large PDFs on low-memory phones.** Extraction may fail on old devices near the
   50 MB limit. If that happens in practice, a lower PDF size limit on mobile is the simple
   fix.
