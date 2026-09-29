# 11: Testing

Status: **Decided**

## Goals

- Confidence that **the app works end to end** in a real browser. Coverage is not a goal.
- **Testing must not slow implementation down.** The suite is small, fast, and easy to
  change or delete when a feature changes on purpose.
- Tests run **locally only**, on demand. There is no CI test gate. CI only builds and
  publishes the image (see [08-deployment.md](08-deployment.md)).

## Layers

| Layer | Tool | Scope |
|---|---|---|
| **End-to-end UI** (main layer) | Playwright, Chromium | About 12 spec files of user journeys against the real server binary |
| **Backend integration** | `cargo test` + `tokio::test` | A handful of tests for logic that's hard to observe through the UI |
| Frontend unit | Vitest | **Only** for pure logic helpers (for example, computing the visible branch from a tree). No component tests. |

Not included: coverage targets, visual or screenshot regression, component tests, load
tests, and WebKit or Firefox.

## Fake OpenRouter

Tests never call the real OpenRouter. There's no cost, no flakiness, and the output is
repeatable.

- `crates/fake-openrouter` is a small Axum **library and binary** that implements the
  subset of OpenRouter we use:
  - `GET /api/v1/models` returns a fixed list: a text-only model, a vision model, and a
    cheap title model.
  - `POST /api/v1/messages` serves named Messages API SSE events, including thinking,
    unknown content blocks, usage, mid-stream errors, and the `[DONE]` tail. Non-streaming
    calls return text content blocks for titles.
  - `GET /api/v1/credits` and `GET /api/v1/key` return a fixed balance and key
    validation. The key `bad-key` gets a 401.
  - `GET /__requests` returns the requests it received, so tests can assert what
    Sprinter sent (image parts, PDF `file` parts with engine `cloudflare-ai`, inlined
    text, the model slug, system instructions). `POST /__reset` clears the log.
- **Scenarios** are chosen by magic tokens in the last user message:

  | Token | Behavior |
  |---|---|
  | _(none)_ | Echo-style reply: `You said: …`, streamed over about 300 ms |
  | `[[slow]]` | About 10 s stream, for reload/reattach and cancel tests |
  | `[[error]]` | Returns 429 mid-stream |
  | `[[rich]]` | Markdown table, code block, KaTeX math, and a Mermaid diagram |
  | `[[think]]` | About 2 s of silence before content, for the "Thinking…" state |

- The same crate is used **in-process** by the Rust integration tests and **as a binary**
  by the Playwright suite, so there is one fake with one set of scenarios. It's not part
  of the production image.
- Sprinter gets `OPENROUTER_BASE_URL` (default `https://openrouter.ai/api/v1`) so tests
  can point it at the fake.

## End-to-end harness

- The suite lives in `e2e/` (its own `package.json`). Playwright's `webServer` config
  starts:
  1. `fake-openrouter` on a random free port.
  2. `sprinter` (a **debug** build, for fast incremental compiles) with a fresh temp
     `DATA_DIR`, `SPRINTER_PASSWORD=test`, `SPRINTER_INSECURE_COOKIES=true`, and
     `OPENROUTER_BASE_URL` pointing at the fake. In debug builds `rust-embed` reads
     `web/dist` from disk, so after a frontend-only change you run `vite build` with no
     Rust recompile.
- **Global setup** logs in once, saves the key `test-key` through Settings, and stores
  the session as Playwright `storageState`. Specs start already logged in, except
  `auth.spec`.
- **Isolation:** every test creates its own chats, so tests run fully in parallel against
  one server. There's no DB reset between tests.
- **Projects:**
  - `desktop`: Desktop Chrome, runs **every** spec.
  - `mobile`: Pixel 7 emulation (Chromium), runs only tests tagged `@mobile`, which
    cover the drawer navigation, composer, attachments, and send flow. This keeps the
    run time close to desktop-only.
- **Selectors:** `getByRole`, `getByLabel`, and `getByText` first. `data-testid` only where
  there's no accessible handle (for example the streaming indicator). This keeps tests
  stable through visual changes and doubles as an accessibility check.
- **Waiting:** Playwright auto-waiting and `expect(...).toBeVisible()` / `toHaveText()`
  only. No fixed sleeps.
- **Failures:** keep a trace, screenshot, and video only on failure. `retries: 0`
  locally, so flakiness is visible and gets fixed rather than hidden.

### Commands

| Command | What it does |
|---|---|
| `npm run e2e` (in `e2e/`) | Builds web and debug server if needed, runs the full suite headless |
| `npm run e2e -- chat` | Runs one spec file (filter by name) |
| `npm run e2e:ui` | Playwright UI mode, for writing and debugging tests |
| `cargo test` | Backend integration tests |

**Budget:** the full E2E suite should finish in **under 60 s** on a dev machine once
built. If it grows past that, trim or merge journeys instead of adding infrastructure.

## E2E journeys (initial set)

| Spec | Journeys |
|---|---|
| `auth` | Wrong password shows an error. Login lands on a new chat. Logout returns to login. An unauthenticated deep link redirects to login and then back. |
| `chat` @mobile | Send a message and see the streamed reply. The Stop button appears and then goes away. The chat shows in the sidebar with its auto title. Rename a chat. Delete a chat. |
| `branches` | Edit a user message, `‹ 2 / 2 ›` appears, switching back restores the old reply. Regenerate creates a sibling. The branch survives a reload. |
| `streaming` | Reload during a `[[slow]]` reply: it reattaches and completes. Stop keeps the partial text and shows the cancelled state. `[[error]]` shows the error with Retry, and Retry works. `[[think]]` shows "Thinking…". |
| `files` @mobile | Image: a thumbnail chip appears, and the fake receives a base64 `image` block. PDF: the fake receives extracted text or a `document` block with the native parser for scans. A text file is inlined. An oversized file is rejected with its limit shown. An unsupported type is rejected. Remove a chip before sending. |
| `models` | Search the picker. Starring a favorite puts it first, and that persists. Switching model mid-chat makes the fake receive the new slug. Image attach is disabled for a text-only model. |
| `render` | `[[rich]]`: a table renders, the code block has a working Copy button, KaTeX output is present, and Mermaid renders an SVG. |
| `search` | Find a phrase from an earlier chat, and clicking the result opens that chat. |
| `settings` | A bad key is rejected. A good key shows its hint. Custom instructions reach the fake as a system message. Usage totals reflect the fake's costs, including after deleting a chat. Sessions list, and revoking another session. |
| `export` | Markdown and JSON downloads contain the chat text (JSON includes all branches). |
| `pwa` | The service worker registers. Offline (`context.setOffline`): the app shell loads, the banner shows, a previously opened chat is readable, and send is disabled. |

## Backend integration tests

Each test runs in a temp `DATA_DIR` with the in-process fake. They cover only:

- **Message tree:** `switch` resolves to the most recent leaf. Edits and regenerations
  create the correct siblings. The prompt equals the root-to-leaf path.
- **Generation lifecycle:** a late subscriber gets the snapshot plus live deltas. Two
  subscribers see the same stream. Cancel keeps the partial text. On restart, anything
  left `streaming` becomes `interrupted`.
- **Uploads:** content sniffing overrides the client MIME. Size limits are enforced
  during streaming. Identical content is deduplicated. GC removes orphans. SVG is
  rejected.
- **Auth:** rate limiting. `X-Forwarded-For` is honored only from `TRUSTED_PROXIES`.
  Changing the password invalidates sessions and the stored key.
- **Spend:** deleting a chat folds its cost into `usage_rollup`, and totals stay the same.
- **Migrations:** they apply cleanly to an empty DB.
- **Log redaction:** capture all log output during login, API key save, and a
  generation. Assert that the password, API key, and session token never appear, and
  that previews are truncated to 200 characters on a single line.

## Working agreement

- Write the E2E spec for a feature **once that feature's milestone works**, not before
  and not alongside every commit.
- When a feature changes on purpose, update or delete its tests in the same change.
  Don't keep brittle tests alive.
- A bug found by hand gets a regression test only if it's cheap to express as a journey
  or a backend test.
