# 02: File uploads

Status: **Decided**

## Supported types in v1

| Kind | Examples | How it reaches the model |
|---|---|---|
| Images | png, jpg, webp, gif | Sent as OpenRouter `image_url` content parts (base64 data URL). Only offered when the selected model supports image input. |
| PDFs | pdf | **Superseded by [14-pdf-text.md](14-pdf-text.md):** text is extracted in the browser and sent as a text part. Previously: sent as an OpenRouter `file` content part. OpenRouter parses PDFs for any model through its `file-parser` plugin. Engines: `cloudflare-ai` (free, PDF→markdown), `mistral-ocr` (paid, for scans), and `native` (models with built-in PDF support, billed as input tokens). The plugin accepts one engine per request, so the Settings default or a single composer override applies to all uncached PDFs in the prompt. `message_attachments.pdf_engine` records the engine requested when each PDF was attached. The effective engine override is sent in the message request. |
| Text and code | txt, md, csv, json, source files | Inlined into the user message as a fenced block labeled with the filename. No provider-side file support is needed. |

Office documents (docx/xlsx/pptx) are **out of scope** for v1.

## Lifecycle

```
pick / paste / drop
   │
   ├─ (client) images: downscale to ≤2048px long edge, re-encode as webp/jpeg
   │
   ▼
PUT /api/uploads  (raw body, filename in header)
   │  server streams the body → $DATA_DIR/tmp/<uuid>
   │  while computing sha256 and counting bytes (aborts at limit)
   │  sniffs magic bytes → decides kind (image/pdf/text) and ignores the client MIME
   │    png/jpeg/webp/gif magic → image    %PDF- → pdf
   │    otherwise: valid UTF-8 with no NUL bytes → text (mime from extension, else text/plain)
   │    .svg, and anything else → 415
   ▼
atomic rename → $DATA_DIR/uploads/<sha[0..2]>/<sha256>   (skipped if it already exists)
INSERT uploads(id, sha256, filename, mime, kind, size, created_at)
   │
   ▼
client shows a chip or thumbnail in the composer and holds the upload id
   │
POST /api/chats/:id/messages { content, attachment_ids: [...], pdf_engine? }
   → message_attachments(message_id, upload_id) rows; each PDF records the selected engine
   │
   ▼
on each generation, the server rebuilds the prompt from the branch path:
   for every attachment on every message in the path:
     image → read file, base64 → image_url part
     pdf   → read file, base64 → file part (+ cached parse annotations, see below)
     text  → read file → fenced text block
   select one file-parser engine for this request: request override, else Settings default
   apply that engine to every uncached PDF included in the prompt
```

## Client-side rules

- Uploads start **as soon as a file is picked, pasted or dropped**. Sending a message
  only references upload IDs.
- Attachments belong to the message they were sent with. Branches created by
  regenerating reuse them. An **edit** starts with the original message's attachments
  pre-filled in the composer, and they can be removed or added to.
- The PDF engine selector is one control for the send, not one toggle per attachment.
  Its effective value is the explicit send override when set, otherwise the Settings
  default. OpenRouter accepts one parser engine per request, and the selected engine
  applies to every uncached PDF in the full prompt, including PDFs from earlier messages.
- Images are downscaled before upload: max 2048 px on the long edge, re-encoded as WebP
  at quality 0.85. GIFs are uploaded as-is so animation survives. Pasted images are named
  `pasted-YYYYMMDD-HHMMSS.png`.
- The model's `input_modalities` (from `/api/models`) decides whether image attachments
  are offered. For what happens to images already in the history when switching to a
  model without vision, see [01-chat.md](01-chat.md).

## Storage options

| | A. Content-addressed files (**chosen**) | B. Per-upload files | C. BLOBs in SQLite |
|---|---|---|---|
| Layout | `uploads/ab/ab12…` named by sha256 | `uploads/<uuid>` | `uploads` table column |
| Dedupe | Yes. The same file uploaded twice is stored once. | No | No |
| Memory while uploading | Streamed, so O(chunk) | Streamed, so O(chunk) | sqlx needs the whole blob in memory |
| DB size | Small, metadata only | Small | Grows with every file, which bloats WAL, backups, and VACUUM |
| Backup | `sqlite3 .backup` plus copying `uploads/` | Same | One file |
| Integrity | The filename is the checksum | None | SQLite |
| Deletion | Delete the file when no `uploads` row references its hash | Delete the file | Delete the row |

**Chosen: A.** We store files on disk rather than in SQLite because it keeps memory flat and the DB
small. Content addressing costs one extra hash pass during the stream and gives us dedupe
and integrity for free.

## Limits (defaults)

| Limit | Value | Reason |
|---|---|---|
| Image | 20 MB after client downscale | These are usually under 1 MB after downscale anyway |
| PDF | 50 MB | Base64 adds 33%, so ~67 MB of transient memory per request in the worst case |
| Text / code | 1 MB | About 250k tokens, already more than most context windows |
| Files per message | 10 | |
| Total attachments in a single outgoing prompt | 100 MB raw | Bounds peak memory while building the OpenRouter request |

All limits are configurable in Settings, up to hard ceilings compiled into the server:
**image 40 MB, PDF 100 MB, text 5 MB, 20 files per message, 200 MB per prompt**.
Separately, the typed content of a message is capped at 256 KB.

## Cost and repeated re-sending

Chat requests can only take attachments inline (base64 `image_url` / `file.file_data`, or
a public URL, which isn't usable for a private server). So **every** request re-sends
every attachment that appears in the branch path. This matters in two ways:

- **Tokens:** images and PDFs are billed again on every turn, just as they are in any
  chat UI. Prompt caching at providers that support it softens this.
- **PDF parsing** (superseded by [14-pdf-text.md](14-pdf-text.md)): OpenRouter returns file `annotations` with the parsed PDF content.
  We store them in `message_attachments.parse_cache` and send them back on later turns,
  so OpenRouter skips re-parsing. This matters for the paid `mistral-ocr` engine.
  Annotation shape (from the OpenRouter PDF guide):
  `{"type":"file","file":{"hash":…,"name":…,"content":[{"type":"text",…},{"type":"image_url",…}]}}`.

## OpenRouter Files API: evaluated, not used in v1

OpenRouter has a Files API (`POST /api/v1/files`, IDs like `or_file_…`, 100 MiB per file,
10 GiB per workspace, free, no expiry). As documented (checked 2026-09-25), it doesn't
fit our core flow:

- **Uploaded file IDs can't be referenced in chat message content parts.** They are
  consumed only by server tools: the `openrouter:files` tool (the model reads, writes,
  and lists workspace files) and shell/bash **containers**.
- **Files you upload can't be downloaded again** (`/content` returns 400). We need local
  copies anyway for UI previews and re-sending.
- It's in beta and works on the global endpoint only.

**Local storage stays the source of truth.**

Possible v2 uses:
- Mirror large text/code files to the Files API and enable the `openrouter:files` tool,
  so the model reads them on demand instead of us inlining them every turn.
- Office docs (docx/xlsx/pptx), which the Files API accepts, handled the same
  tool-based way.

## Serving files back to the browser

`GET /api/uploads/:id`:
- Authenticated.
- `Content-Type` comes from the sniffed MIME.
- PDFs and text are served with `Content-Disposition: inline`. Anything the server can't
  classify is served as an `attachment`.
- `Content-Security-Policy: sandbox` on every upload response, plus
  `X-Content-Type-Options: nosniff`. This prevents stored XSS through, for example, crafted
  HTML or SVG. **SVG is not accepted** as an image type.
- `Cache-Control: private, max-age=31536000, immutable`, because content never changes
  for a given id.
- No server-side thumbnailing or image decoding. This keeps the binary small and avoids
  image-parser attack surface. The client renders the original with CSS sizing.

## Garbage collection

- Uploads that are never attached to a sent message are deleted after 24 h.
- Deleting a chat deletes its messages and `message_attachments` rows. Uploads that end up
  with no references are deleted along with their files (within the same maintenance
  sweep).
- A periodic sweep (hourly) runs both of these and also removes stale `tmp/` files.

## OpenRouter PDF protocol spike (2026-09-25)

The current official [PDF Inputs guide](https://openrouter.ai/docs/guides/overview/multimodal/pdfs.md)
uses `plugins: [{id: "file-parser", pdf: {engine: "cloudflare-ai"}}]`; the supported
engines are `cloudflare-ai`, `mistral-ocr`, and `native`. The engine is a request-level
plugin option, not a field on an individual `file` content part. PDF annotations appear
in non-streaming assistant messages as `choices[0].message.annotations`, and parsed
annotations from provider failures appear at `error.metadata.file_annotations`. The
current docs do not specify annotations on streaming responses; the planned cache needs
an implementation spike before it can rely on the streaming reply to provide them.
