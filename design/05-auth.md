# 05: Authentication

Status: **Decided**

## Model

- One master password, provided through `SPRINTER_PASSWORD`. There are no user accounts.
- The `settings` table stores an Argon2id PHC hash of the password (`password_hash`).
  At startup the server checks `SPRINTER_PASSWORD` against it:
  - **No hash yet** (first start): hash the password and store it.
  - **Mismatch** (the password was changed): delete all sessions, mark the stored
    OpenRouter key as unreadable (see [07-settings.md](07-settings.md)), and store the
    new hash.
- Login verifies the submitted password against this hash. Argon2 verification is
  constant-time.
- On login the server issues an opaque random session token (256-bit), stored **hashed**
  in a `sessions` table along with created/last-seen time and the user agent.

## Session cookie

- Name `__Host-sprinter` with `HttpOnly; Secure; SameSite=Strict; Path=/`. With
  `SPRINTER_INSECURE_COOKIES` it becomes `sprinter` without `Secure`, because the
  `__Host-` prefix requires `Secure`.
- **Sliding 30-day expiry:** activity refreshes the expiry, at most once per hour to
  limit DB writes.
- `Secure` can be relaxed for plain-HTTP localhost development with a dev flag.

## Protection

- Failed login attempts are rate-limited **per IP**: 5 per minute, then exponential
  backoff capped at 15 minutes. A looser **global** limit (30 failures per minute) slows
  distributed guessing without locking you out, because existing sessions are never
  affected by either limit. The client IP comes from `TRUSTED_PROXIES` rules in
  [08-deployment.md](08-deployment.md).
- CSRF: `SameSite=Strict` plus requiring `Content-Type: application/json` or a custom
  header on mutating requests.
- Changing `SPRINTER_PASSWORD` invalidates all sessions (see Model above).

## UX

- Log in once per device and stay logged in.
- Settings lists active sessions (device and user agent, last seen) with **Log out** and
  **Log out all devices**.
- The PWA shell loads from cache without auth. API calls that return 401 redirect to the
  login screen.
