# 03: Frontend

Status: **Decided**

## Stack

| Concern | Choice |
|---|---|
| Framework | React (SPA, no SSR, no Next.js) |
| Build | Vite |
| Chat UI | **assistant-ui**, using its unstyled *primitives* (`Thread`, `Composer`, `Message`, `BranchPicker`, `ActionBar`, attachments) that we style ourselves |
| General UI components | **HeroUI** (built on React Aria, Tailwind CSS). Not shadcn/ui (user preference). |
| Styling | Tailwind CSS with a custom dark theme (tokens are shared by HeroUI and our assistant-ui styling) |
| Routing | TanStack Router |
| Server state | TanStack Query |
| PWA | vite-plugin-pwa (Workbox) |
| Language | TypeScript (strict) |
| API types | Generated from the Rust structs with `ts-rs` into `web/src/api/types.gen.ts`. There's no handwritten duplication, and a type change breaks the TS build. |
| Package manager | npm workspaces (`web`, `e2e`) |

### Scaffold compatibility check (M0)

The web scaffold builds with React 19, Vite 7, Tailwind CSS 4, and HeroUI 3.2.6. HeroUI
3's `@heroui/styles` imports its Tailwind 4 layer and exposes semantic CSS variables; the
Midnight neon tokens override those variables in `web/src/theme/app.css`. The build is
self hosted and produces `web/dist` for Rust embedding.

The route `/spike/assistant-ui` is a throwaway external store spike. A local parent-linked
tree drives `useExternalStoreRuntime`; edit creates a sibling user turn, regenerate creates
a sibling assistant reply, runtime branch changes return the selected head to the fake
store, and cancellation stops the local stream while keeping partial text. The stock image
and text attachment adapters put completed content on the user message. For the app API,
the attachment adapter can upload the file and retain the returned upload id as metadata;
`onNew` can then send `attachment_ids` alongside text, and the returned message tree carries
attachment metadata back to the runtime. No custom assistant-ui message converter is needed
for ordinary text, image, and file message parts. The demo's file adapters are in-memory
examples only; they do not call `/api/uploads`.

## Why assistant-ui

It already implements the chat-specific UX that is expensive to get right:
- keeping the view pinned to the bottom while text streams in
- rendering markdown and code highlighting as it streams
- a composer with attachments, paste, and drag-drop
- edit and regenerate
- a branch picker (`< 2/3 >`)

Its external-store/custom runtime lets our backend own the thread state. That fits the
server-owned generation model in [01-chat.md](01-chat.md): the client renders whatever
the server says the current branch is, and reattaches to in-flight SSE streams.

assistant-ui's *styled* components are distributed through the shadcn CLI. Because we're
not using shadcn, we build on the primitives and style them to match our design.

## Why HeroUI

- It's built on React Aria, which handles touch, press, focus, and keyboard behavior
  correctly across mobile and desktop. That's the kind of common UX we don't want to
  debug.
- It's dark-first and animated, which is a good base for a UI that doesn't look boring.
- Both it and our assistant-ui styling use Tailwind, so there's **one theme token set**
  for the whole app.
- Rejected: Mantine (more complete, but it uses a separate styling system from the chat
  UI), Radix Themes (looks plain), and MUI (looks like Material).

## PWA and offline behavior

Offline mode is a **read-only cache**:
- The service worker precaches the app shell, so the app always opens, including offline
  or when the server is unreachable.
- TanStack Query persists to IndexedDB. The chat list and recently opened chats (the
  current branch plus attachment metadata) are readable offline. Attachment files
  themselves are cached opportunistically through the browser HTTP cache.
- When offline, a clear banner appears and sending, editing, regenerating, and uploading
  are disabled. There is no outbox or queueing.
- **Logging out** clears the IndexedDB cache.
- API responses are **not** cached by the service worker. Only TanStack Query's
  persistence stores data, so data-freshness logic lives in one place.
- Update flow: a new build is detected and a "Reload to update" toast appears. The
  service worker does not skip waiting silently while a stream might be open.

## Markdown rendering in replies

| Feature | Library | Loading |
|---|---|---|
| GFM (tables, task lists, strikethrough, autolinks) | `react-markdown` + `remark-gfm`, through assistant-ui's markdown text component | Always included |
| Syntax highlighting | Shiki, with a custom theme tinted cyan and blue (see [06-visual.md](06-visual.md)) | Grammars are lazy-loaded per language |
| Math: `$…$`, `$$…$$`, `\(…\)`, `\[…\]` | `remark-math` + `rehype-katex` | KaTeX JS and CSS load on first use |
| Mermaid diagrams (` ```mermaid `) | `mermaid`, with a dark theme matched to the tokens | Loaded on first use. It renders only once the code block is **closed**, so it doesn't re-render on every streamed delta. Invalid diagrams fall back to showing the source. |

- Raw HTML in model output is **not rendered** (no `rehype-raw`). Links open in a new tab
  with `rel="noopener noreferrer"`.
- Mermaid runs with `securityLevel: 'strict'`.
- All lazy chunks are precached by the service worker after the first load, so math and
  diagrams also render offline.

## Input conventions

- **Desktop:** Enter sends and Shift+Enter adds a newline. **Touch devices:** Enter adds
  a newline and sending uses the button. Detected with `(pointer: coarse)`.
- Esc stops a streaming reply (when the composer is focused), or closes a dialog.
- Ctrl/⌘+K opens search. Ctrl/⌘+Shift+O starts a new chat.
- Paste an image or file into the composer, or drop it anywhere on the chat, to attach.

## First run

After the first login with no OpenRouter key set, the app opens a short setup screen:
1. Paste the API key (validated right away).
2. Pick a default model from the model list, with a search box. Setting favorites and a
   title model is optional.

The setup screen is skipped once a key exists. Until then, sending is disabled and the
composer links to setup. The model list is public on OpenRouter, so the picker works
before a key is set.

## Build and serving

- `vite build` outputs to `web/dist`, which the Rust binary embeds at compile time (see
  [04-backend.md](04-backend.md)).
- Hashed assets are served with `Cache-Control: immutable`. `index.html` and the service
  worker are served with `no-cache`.
