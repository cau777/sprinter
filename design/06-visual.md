# 06: Visual design

Status: **Decided.** The direction is **Midnight neon**. The reference sketch is option A in
[sketches/looks.html](sketches/looks.html).

## Principles

- **Dark only.** There is no light theme or theme switch. `color-scheme: dark` is set on the
  root.
- There is **one accent (cyan)**, used only to show where to act or look: the primary
  action, the active item, the streaming reply, and inline code. Everything else uses
  blue-tinted neutrals.
- Panels are **glass** (translucent with a backdrop blur) over a background with two faint
  radial glows. Content surfaces such as code blocks and message bubbles are solid, so
  they stay readable.
- **Glow** means the assistant is working. It isn't decoration anywhere else.

## Tokens

Defined once as CSS variables and mapped into the Tailwind and HeroUI theme config.

| Token | Value | Use |
|---|---|---|
| `--bg` | `#080b12` | App background |
| `--bg-glow-1` | `rgba(61,232,255,.10)` radial, top-right | Background atmosphere |
| `--bg-glow-2` | `rgba(90,110,255,.10)` radial, bottom-left | Background atmosphere |
| `--panel` | `rgba(20,27,43,.55)` + `backdrop-filter: blur(14px)` | Sidebar, composer, popovers, modals |
| `--surface` | `#141b2b` | User bubbles, cards, inputs |
| `--code-bg` | `rgba(0,0,0,.35)` | Code blocks |
| `--border` | `rgba(120,160,220,.12)` | Hairlines and dividers |
| `--accent` | `#3de8ff` | Primary action, active state, streaming, links |
| `--accent-soft` | `rgba(61,232,255,.08)` | Active item fill, attachment chips |
| `--accent-line` | `rgba(61,232,255,.25)` | Accent borders |
| `--on-accent` | `#061018` | Text and icons on accent fills |
| `--text` | `#d7deea` | Body text |
| `--text-strong` | `#ffffff` | Titles, active items |
| `--text-muted` | `#6b7690` | Metadata, placeholders, group labels |
| `--danger` | `#ff5d7a` | Errors, destructive actions |
| `--warning` | `#ffc857` | Interrupted or cancelled generations, limits |
| `--success` | `#4ade9a` | Confirmations |

Semantic colors are separate from the accent. All text/background pairs must meet WCAG AA,
so muted text is checked against `--surface`, not just `--bg`.

## Typography

| Role | Face | Notes |
|---|---|---|
| Display / brand / chat titles | **Sora** 500/700 | Wordmark is uppercase with 0.18em tracking |
| UI and message body | **Manrope** 400/600 | 14–15px base, line-height 1.6 |
| Code, model IDs, file chips, numbers | **JetBrains Mono** 400 | Model slugs and costs are always mono and tabular |

The fonts are **self-hosted** from the embedded build (via `@fontsource`), not loaded from
Google Fonts, so the PWA works offline and makes no third-party requests.

## Key components

- **Sidebar:** a glass panel with a "New chat" button outlined in the accent, search, chats
  grouped by date (Today / Yesterday / Last 7 days / older), and Settings pinned to the
  bottom. The active chat gets an `--accent-soft` fill and a 2px inset accent bar on the
  left. On mobile the sidebar becomes a slide-over drawer.
- **Top bar:** the chat title (Sora) and a pill-shaped **model picker** showing the
  OpenRouter slug in mono. The picker opens a searchable HeroUI autocomplete that shows
  context length, price per million tokens, and modality badges (image/pdf).
- **User message:** a right-aligned `--surface` bubble with an `--accent-line` border and a
  radius of `14 14 4 14`. Attachments sit above it as chips.
- **Assistant message:** no bubble, left-aligned, with a 2px accent rule on the left.
  - **Streaming:** the rule has a cyan glow (`box-shadow`) and a glowing dot at the end
    of the text.
  - **Complete:** the glow fades out and the rule dims to `--border`, so only the reply
    being generated glows.
  - **Error, cancelled, or interrupted:** the rule uses the semantic color, with an inline
    note and a Retry button.
- **Message footers:**
  - Assistant: `‹ 2 / 3 ›` branch picker (accent), Copy, and Retry, where Retry has a
    dropdown to retry with a different model. On the right, `tokens · $cost` in mono,
    taken from OpenRouter's usage data.
  - User: Edit and Copy, shown on hover (desktop) or long-press (touch), plus the branch
    picker when the message has sibling edits.
- **Code blocks:** `--code-bg`, a language label, and a Copy button. Syntax colors are
  tinted toward cyan and blue, with no rainbow theme.
- **Composer:** a floating glass panel with a soft cyan shadow underneath, an auto-growing
  textarea, attachment chips, an attach button, and a square cyan send button, which turns
  into a Stop button while the reply is generating. Bottom padding respects
  `env(safe-area-inset-bottom)`.
- **Attachment chips:** mono filename · size on `--accent-soft`. Images show a thumbnail.
  Chips for uploads in progress show a progress bar.
- **Login screen:** the wordmark over the background glows and a single password field.
  Nothing else.

## Motion

- The streaming glow pulses gently (about 2 s ease-in-out on opacity) and is the only
  continuous animation.
- The glow fades out over about 400 ms when a reply completes.
- The sidebar drawer, modals, and popovers use HeroUI's default transitions, about
  150–200 ms.
- New messages don't bounce or slide in; the text simply appears.
- `prefers-reduced-motion`: the pulse and fade are removed and the glow stays static while
  streaming.

## PWA chrome

- `theme_color` / `background_color`: `#080b12`.
- The icon is the Sprinter mark in cyan on `--bg`, with a maskable variant.
- `display: standalone`. On iOS, `apple-mobile-web-app-status-bar-style` is
  `black-translucent`.

## M0 implementation notes

The web scaffold maps the design tokens into Tailwind 4 using `@theme inline`, and into
HeroUI 3's semantic palette through CSS custom properties. Sora, Manrope, and JetBrains
Mono are bundled from `@fontsource`. The empty chat shell currently includes the glass
sidebar, private workspace indicator, prompt suggestions, and responsive composer; later
milestones will wire the controls to the API.
