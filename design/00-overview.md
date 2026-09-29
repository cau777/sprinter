# Sprinter: Overview

Sprinter is a self-hosted, single-user AI chat web app with the core capabilities of
ChatGPT / Claude web. It talks to models through OpenRouter, so it isn't tied to one
model provider.

## Goals

- Chat with LLMs, with streaming responses.
- Persist conversations with full CRUD (create, list, read, rename, delete).
- Upload files and attach them to messages.
- Run as one small, fast container.
- Work as an installable PWA on mobile and desktop.
- Use a dark-only UI that looks distinctive, not like a stock admin template.

## Non-goals

- Multi-user support, accounts, sharing, or roles.
- Integrations with model providers other than OpenRouter.
- Light mode.

## Decided constraints

| Area | Decision |
|---|---|
| Users | Single user. |
| Auth | One master password, set with an env var. There are no other auth mechanisms. |
| LLM access | OpenRouter only. The API key is set in the in-app **Settings** page, not in an env var. |
| Backend | Rust, shipped as one server binary in a minimal container image. |
| Storage | Everything is written under `DATA_DIR`. The deployment is expected to mount this path on persistent storage. |
| `DATA_DIR` default | `/var/lib/sprinter` (FHS location for variable state data). |
| Delivery | PWA served by the Rust server, with aggressive caching through a service worker. |
| Frontend stack | React + Vite SPA, assistant-ui primitives, HeroUI, and Tailwind. See [03](03-frontend.md). |
| Backend stack | Axum + SQLite (sqlx). See [04](04-backend.md). |
| Sessions | 30-day sliding session cookie. See [05](05-auth.md). |
| Theme | Dark mode only, Midnight neon direction. See [06](06-visual.md). |

## Design documents

| Doc | Topic | Status |
|---|---|---|
| [00-overview.md](00-overview.md) | Scope and decided constraints | Decided |
| [01-chat.md](01-chat.md) | Chat features, message tree, server-owned generation | Decided |
| [02-files.md](02-files.md) | File upload lifecycle, storage, limits | Decided |
| [03-frontend.md](03-frontend.md) | Frontend stack, PWA/offline | Decided |
| [04-backend.md](04-backend.md) | Backend stack, memory, config | Decided |
| [05-auth.md](05-auth.md) | Master password and sessions | Decided |
| [06-visual.md](06-visual.md) | Visual design: Midnight neon tokens, type, components | Decided |
| [07-settings.md](07-settings.md) | Settings page, spend view, key encryption | Decided |
| [08-deployment.md](08-deployment.md) | Reverse proxy, image, env vars, backups, export | Decided |
| [09-data-model.md](09-data-model.md) | SQLite schema | Decided |
| [10-api.md](10-api.md) | HTTP API and SSE protocol | Decided |
| [11-testing.md](11-testing.md) | E2E (Playwright + fake OpenRouter), backend tests | Decided |
| [12-implementation-plan.md](12-implementation-plan.md) | Milestones M0–M9, repo layout, risks | Active |
| [13-logging.md](13-logging.md) | Server logging: files, format, levels, event catalog, redaction | Decided |
| [14-pdf-text.md](14-pdf-text.md) | PDF text extraction in the browser with pdf.js; no third-party parsing | Proposed |
| [15-messages-api-migration.md](15-messages-api-migration.md) | Move all OpenRouter requests from Chat Completions to the Messages API | Proposed |
| [16-openrouter-tools.md](16-openrouter-tools.md) | OpenRouter server tools as composer pills: web search, bash, local date | Proposed |

## Open questions

None for v1. Items explicitly deferred:
- System prompt and model presets.
- Default generation parameters (temperature, max tokens, reasoning effort).
- Importing chats, and full data export/import.
- Office documents, and using OpenRouter's Files API through the `openrouter:files` tool
  (see [02-files.md](02-files.md)).
- Deleting individual messages or branches.
