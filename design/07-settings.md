# 07: Settings

Status: **Decided**

Settings are stored in the DB (`settings` table, one JSON value per key) and edited on the
Settings page. They are not env vars.

## Contents (v1)

| Section | Setting | Notes |
|---|---|---|
| **OpenRouter** | API key | Write-only in the UI: after saving, only `sk-or-…last4` is shown. Encrypted at rest (see below). Saving tests the key against OpenRouter. |
| **Models** | Default model | Used for new chats. |
| | Title model | A cheap, fast model for automatic titles. If none is set, the chat's own model is used. |
| | Favorite models | An ordered shortlist that the model picker shows first. It's also editable from the picker (star icon). |
| **Behavior** | Global custom instructions | Sent as the system message on every request. Empty means no system message. |
| | PDF parsing engine | `cloudflare-ai` (default, free), `mistral-ocr`, or `native`. See [02-files.md](02-files.md). |
| | Upload limits | Per-type size limits, up to the server's hard ceilings. |
| **Usage** | Spend view (read-only) | See below. |
| **Sessions** | Active sessions | List of sessions, log out one, log out all. See [05-auth.md](05-auth.md). |

Default generation parameters (temperature, max tokens, reasoning effort) are **not**
in v1. Requests use OpenRouter and provider defaults.

## Usage and spend view

- **Balance:** per-key `limit_remaining` from `GET /api/v1/key`, cached for about a
  minute. OpenRouter's `GET /api/v1/credits` returns the account's total purchased and
  used credits but requires a separate Management API key, which v1 does not request.
  When the regular API key has no spend limit (`limit_remaining: null`), show the balance
  as unavailable instead of presenting the account-wide credit balance.
- **Spend:** aggregated from the per-message `cost` and token counts we store. OpenRouter
  always returns usage, including in the final streamed chunk; `cost` is optional.
  - Totals for today, the last 7 days, the last 30 days, and all time.
  - Breakdown **by model** and **by chat** (top N), with a daily spend sparkline.
  - Title-generation spend is included, attributed to the chat with a `title` source tag.
- Spend from **deleted chats is kept** in totals and the by-model breakdown (through
  `usage_rollup`, see [09-data-model.md](09-data-model.md)). It only disappears from the
  by-chat breakdown.
- Messages created before the key was set, or ones that failed, have no cost and are
  shown as "—" rather than 0.
- **Day boundaries:** the client sends its IANA time zone (`?tz=`), and "today" and the
  daily series use that zone. `usage_rollup` (deleted chats) is bucketed by UTC day, so
  spend from deleted chats can shift by up to a day in the daily chart. That's accepted.
- The balance needs a valid key. Without one, the section shows "Set an API key to see
  your balance".

## API key at rest

The key is encrypted with a key derived from `SPRINTER_PASSWORD` (Argon2id with its
own random salt, separate from the login hash, then XChaCha20-Poly1305). The
encryption key is derived once at startup and kept in memory. A leaked copy of `DATA_DIR` or a backup therefore
doesn't expose the OpenRouter key. Changing the master password makes the stored key
unreadable. When that happens, Settings asks you to enter the key again, and nothing else
breaks.
