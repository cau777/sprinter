# 05: Authentication

Status: **Decided**

## Model

- One master password, provided through `SPRINTER_PASSWORD`. There are no user accounts.
- At startup the server derives an Argon2id hash of the password and keeps only that hash
  in memory. Login compares against it in constant time.
- On login the server issues an opaque random session token (256-bit), stored **hashed**
  in a `sessions` table along with created/last-seen time and the user agent.

## Session cookie

- `HttpOnly; Secure; SameSite=Strict; Path=/`
- **Sliding 30-day expiry:** activity refreshes the expiry, at most once per hour to
  limit DB writes.
- `Secure` can be relaxed for plain-HTTP localhost development with a dev flag.

## Protection

- Failed login attempts are rate-limited, both in total and per IP (for example 5 per
  minute, then exponential backoff).
- CSRF: `SameSite=Strict` plus requiring `Content-Type: application/json` or a custom
  header on mutating requests.
- Changing `SPRINTER_PASSWORD` invalidates all sessions. The server stores a fingerprint
  of the password hash and wipes sessions when it changes.

## UX

- Log in once per device and stay logged in.
- Settings lists active sessions (device and user agent, last seen) with **Log out** and
  **Log out all devices**.
- The PWA shell loads from cache without auth. API calls that return 401 redirect to the
  login screen.
