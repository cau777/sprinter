# 12: Implementation plan

Status: **Active**

The plan builds vertical slices. Every milestone ends with the app running and a
visible step forward, committed to `main`. The riskiest unknowns come first (M0), so a
bad assumption costs a day rather than a rewrite. E2E specs for a milestone are written
at its end, following the working agreement in [11-testing.md](11-testing.md).

## Repository layout

```
Cargo.toml                    # workspace
crates/
  sprinter/                   # the server binary
    migrations/               # sqlx migrations (embedded)
    src/
      main.rs                 # CLI: serve | healthcheck | gen-types
      config.rs               # env vars (08-deployment)
      db.rs                   # pool, pragmas, migrate
      auth/                   # password hash, sessions, rate limit, client IP
      api/                    # axum routers per area: chats, messages, uploads, settings, usage, search
      generation/             # manager: spawn, broadcast, flush, cancel, shutdown
      openrouter/             # client, SSE parsing, request builder
      uploads/                # streaming store, sniffing, GC
      tasks.rs                # scheduler: GC, sessions, snapshots, model cache
      web.rs                  # rust-embed, SPA fallback, cache + security headers
  fake-openrouter/            # lib + bin, scenarios (11-testing)
web/                          # Vite + React SPA
  src/
    api/                      # fetch client, types.gen.ts (ts-rs), SSE client
    routes/                   # TanStack Router
    chat/                     # assistant-ui runtime adapter + styled primitives
    components/               # HeroUI-based UI pieces
    theme/                    # tokens (06-visual), Tailwind + HeroUI config
e2e/                          # Playwright suite
package.json                  # npm workspaces: web, e2e
Dockerfile
.github/workflows/image.yml   # multi-arch build to GHCR
```

## Milestones

Sizes are relative: S is about a day of focused work, M is 2–3 days, L is 4 days or more.

### M0: Scaffold and de-risk (M)

Build the skeleton and settle every open technical unknown before writing features.

- The Cargo workspace. The server serves `web/dist` through `rust-embed` with SPA
  fallback, plus `/healthz`, config parsing, and JSON logging.
- The web app: Vite, React, TS, Tailwind, HeroUI, TanStack Router and Query, and
  `@fontsource` fonts. The theme tokens from [06-visual.md](06-visual.md) are wired
  into Tailwind and HeroUI.
- The `ts-rs` → `types.gen.ts` pipeline (`cargo run -- gen-types`).
- A `fake-openrouter` skeleton serving the models list and the echo stream scenario.
- **Spikes.** Each spike writes its outcome back into the relevant design doc.
  1. **assistant-ui external store runtime:** a throwaway page drives a thread from a
     local tree with a fake streaming source. Confirm the branch picker, edit,
     reload/regenerate, cancel, and attachments map cleanly onto our API. _Fallback:_
     keep the assistant-ui thread, composer and markdown, but build the branch picker and
     edit flow ourselves with HeroUI.
  2. **HeroUI with Tailwind:** check which HeroUI version works with the current
     Tailwind major version, and that dark theme overrides work.
  3. **Static build:** a musl static binary with bundled SQLite (FTS5 confirmed working)
     and mimalloc, in a `scratch` Dockerfile. Measure image size (target < 20 MB) and
     idle RSS (target < 30 MB).
  4. **OpenRouter facts:** check against the current docs:
     - whether `usage: {include: true}` is needed to get `cost`
     - the endpoint and shape of the credits/balance response
     - the `reasoning: {exclude: true}` flag
     - whether `transforms: []` disables middle-out
     - the PDF `file-parser` plugin syntax and how annotations come back when streaming
     - the error shapes for 401, 402, 429 and context-length errors

  Then make the fake's responses match these.
- **Done when:** `cargo run` serves the themed empty shell, the Docker image builds,
  and every spike outcome is recorded.

### M1: Auth and app shell (M)

- Migrations for the **entire** schema in [09-data-model.md](09-data-model.md), up
  front, so later milestones never fight migrations.
- Startup password-hash check and rotation. Sessions with a sliding expiry and the
  `__Host-` cookie. Per-IP and global login rate limits. Client IP resolution through
  `TRUSTED_PROXIES`. The CSRF header check. The security headers and CSP.
- Login screen, and the app shell: glass sidebar (drawer on mobile), top bar, empty chat
  state, and redirecting to login on 401.
- **E2E harness:** Playwright config with the `webServer`s, global setup and both
  projects. Write `auth.spec`.
- **Backend tests:** rate limit and trusted proxies, and that a password change wipes
  sessions.
- **Done when:** you can log in on desktop and phone, and see the themed shell.

### M2: OpenRouter setup and models (S)

- API key encryption, `GET/PATCH /api/settings`, and key validation against OpenRouter.
- The model list cache (`/api/models`).
- The first-run setup screen. The model picker (HeroUI autocomplete with context
  length, price and modality badges) with favorites. The Settings page sections for the
  key, models and custom instructions.
- **Done when:** a fresh install walks you through the key and default model, and the
  picker lists real OpenRouter models.

### M3: Chat core and streaming (L)

This is the heart of the app. Branching is deliberately left for M4.

- The OpenRouter client: streaming SSE parser and request builder (system
  instructions, root-to-leaf path, reasoning excluded, usage included, attribution
  headers). Errors are mapped to readable messages.
- **Generation manager:**
  - spawn a task per generation, with an in-memory buffer and broadcast to subscribers
  - flush to the DB about once a second
  - cancel
  - one active generation per chat, and a global cap of 8
  - mark leftover `streaming` messages `interrupted` at startup
  - graceful shutdown within 8 s
- `POST /api/chats/new/messages` and `/api/chats/:id/messages`, and the
  `/api/messages/:id/stream` SSE (snapshot, deltas, done, error, heartbeat) and cancel.
- Parallel title generation with the `title` event.
- Chat list, get, rename and delete (cancelling any active generation).
- Frontend:
  - the assistant-ui runtime adapter over TanStack Query, with the SSE client that
    reattaches on mount and reload
  - the styled thread: user bubble, assistant accent rule, streaming glow,
    "Thinking…", and the error, cancelled and interrupted states with Retry
  - the composer with Send and Stop
  - the sidebar with date groups, rename and delete
- **Backend tests:** generation lifecycle (late subscriber, two subscribers, cancel,
  interrupted on restart).
- **E2E:** `chat`, and `streaming` (reload/reattach, stop, error, think).
- **Done when:** you can hold a real conversation with any OpenRouter model, close the
  tab mid-reply, and come back to the finished answer.

### M4: Edit, regenerate and branches (M)

- Edit (send with the original's parent), regenerate with an optional one-off model,
  and `POST /api/chats/:id/switch` resolving to the most recent leaf.
- Client-side computation of the visible branch (a pure helper, with a Vitest test).
  The `‹ n / m ›` picker on user and assistant messages. The retry-with-model dropdown.
  Switching the chat's model mid-chat.
- **Backend tests:** the message tree (switch resolution, sibling creation, prompt equals
  the path).
- **E2E:** `branches`, `models`.
- **Done when:** edits and regenerations never destroy anything, and every branch is
  reachable.

### M5: Rich rendering (S)

- GFM through assistant-ui markdown, Shiki with the custom cyan theme and lazy grammars,
  KaTeX and Mermaid loaded lazily (Mermaid renders once its block closes, with
  `strict` security), code copy buttons, and safe links.
- **E2E:** `render`.
- **Done when:** the `[[rich]]` scenario looks right and stays smooth while streaming.

### M6: File uploads (L)

- `PUT /api/uploads`: streamed to `tmp/` with hashing, a limit check during streaming,
  content sniffing, an atomic move into content-addressed storage, and dedupe.
  `GET` with the sandbox headers. `DELETE` for unattached uploads.
- Prompt expansion: images as `image_url`, PDFs as `file` parts with the per-attachment
  engine and annotation caching in `parse_cache`, and text inlined in fences. Images are
  replaced with a placeholder for models without vision. The per-prompt size cap applies.
- The GC sweep for orphans and `tmp/`.
- Frontend:
  - attach button, paste, and drop anywhere on the chat
  - client-side WebP downscale (GIFs left untouched) and upload progress chips
  - image thumbnails, and the OCR toggle on PDF chips
  - attachments shown on sent messages, opening in a viewer
  - edits pre-filled with the original message's attachments
  - image attach disabled for models without vision, and the warning when switching to
    one
- The upload limits section in Settings.
- **Backend tests:** sniffing, limits, dedupe, GC, and SVG rejection.
- **E2E:** `files`.
- **Done when:** a phone photo, a PDF and a source file can go into one message and the
  model sees all three.

### M7: Search, export, usage and sessions (M)

- The FTS5 table and triggers (skipping `streaming` rows), `GET /api/search`, and a
  Ctrl/⌘+K search dialog with snippets that jump to the matching message.
- Export in Markdown (visible branch) and JSON (full tree) from the chat menu.
- Usage: storing per-message cost, `usage_events` for titles, folding into
  `usage_rollup` when a chat is deleted, and `GET /api/usage` with `tz`. The Settings
  usage view shows the balance, totals, by model, by chat, and a daily sparkline.
- The sessions list and revoke actions in Settings.
- **Backend tests:** the rollup keeps totals the same after a delete.
- **E2E:** `search`, `export`, `settings`.
- **Done when:** every Settings section works and past chats are findable.

### M8: PWA, operations and release (M)

- `vite-plugin-pwa`: manifest, icons (regular and maskable), precaching including the
  lazy chunks, and a "Reload to update" toast that doesn't interrupt an open stream.
- The offline read-only mode: TanStack Query persisted to IndexedDB, an offline banner
  with sending disabled, and the cache cleared on logout.
- Daily `VACUUM INTO` snapshots and one at shutdown, with retention. The `healthcheck`
  subcommand.
- The GitHub Actions workflow building amd64 and arm64 to GHCR on tags.
- A README covering env vars, Caddy and nginx snippets (SSE buffering, body size), and
  backup and restore.
- **E2E:** `pwa`. The full suite runs in under 60 s.
- **Done when:** it's installed as a PWA on your phone, deployed behind your proxy
  from a GHCR image, with backups appearing in `DATA_DIR/backups`.

### M9: Polish (S–M)

- A visual pass against [06-visual.md](06-visual.md): motion, reduced motion, contrast
  checks, and safe areas on a real phone.
- An accessibility pass: focus states, labels, and keyboard shortcuts.
- A performance check: bundle size per route, idle and peak RSS against the targets in
  [04-backend.md](04-backend.md), and image size.
- Update any design docs that drifted during implementation.

## Dependency graph

```
M0 ─▶ M1 ─▶ M2 ─▶ M3 ─┬─▶ M4 ─┐
                      ├─▶ M5 ─┼─▶ M7 ─▶ M8 ─▶ M9
                      └─▶ M6 ─┘
```

M4, M5 and M6 depend only on M3 and can be done in any order. M7 needs M4 (export of
branches), M6 (attachment metadata) and M3.

## Risks

| Risk | Impact | Mitigation |
|---|---|---|
| assistant-ui's external store doesn't cover our branching model | Branch UX rework | M0 spike 1, with a fallback of our own branch picker on top of the primitives |
| HeroUI lags behind current Tailwind or React | Build friction | M0 spike 2. Pin the versions that work together. |
| The OpenRouter response shape differs from the docs (cost, annotations) | Wrong spend or PDF cache misses | M0 spike 4. Keep the fake aligned with the real responses. |
| Streaming markdown with Shiki, KaTeX and Mermaid jank on mobile | Poor UX | Lazy loading, Mermaid only on closed blocks, and a profile on a real phone in M5 |
| iOS Safari PWA quirks (not covered by Chromium-only E2E) | Mobile bugs | Manual check on a real device at the end of M3, M6 and M8 |
| Memory spikes from large attachments | Container OOM | Per-prompt caps. Measure peak RSS in M9 with a 50 MB PDF. |
