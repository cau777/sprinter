# 16: OpenRouter tools

Status: **Proposed**. Assumes these are implemented:
- [14-pdf-text.md](14-pdf-text.md): PDFs reach the model as text.
- [15-messages-api-migration.md](15-messages-api-migration.md): every request goes through
  `/api/v1/messages` and has a per-chat `session_id`.

OpenRouter offers server tools such as `openrouter:web_search`, `openrouter:bash` and
`openrouter:image_generation`. A request lists them in `tools`, and the model decides when
to call them. Sprinter exposes the ones it supports as **pills in the composer**. When
the chat's model supports a tool, its pill appears and the user can turn it on or off.
Pills are **off by default**. Tools OpenRouter offers but Sprinter doesn't support are
summarized in a disabled **+** pill.

The main constraint is privacy. A tool must not quietly send chat content to a party
the user didn't choose. OpenRouter's ZDR routing covers inference only: *"ZDR
enforcement only applies to provider routing for inference requests. It does not apply
to plugins and tools you choose to enable, such as web search."* Each supported tool
therefore has a stated **execution location**, and Sprinter never lets a tool fall back
to a third-party engine (Exa, Parallel, Perplexity, Firecrawl) without saying so.

## Tool catalog

Every OpenRouter server tool, and how Sprinter handles it:

| Tool | Handling | Runs where | Notes |
|---|---|---|---|
| `openrouter:web_search` | **Supported** | Inside the model provider, on a ZDR endpoint (`engine: "native"`) | Offered only for models whose ZDR endpoints list it in `native_tools`. See [Web search](#web-search). |
| `openrouter:bash` | **Supported** | OpenRouter's hosted sandbox (`engine: "openrouter"`), no network | Any model that supports tools. See [Bash](#bash). |
| `openrouter:datetime` | **Replaced** | Not sent | The user's local date goes in the system prompt instead. See [Date](#date). Not listed in the **+** pill. |
| `openrouter:apply_patch` | Deferred | Client-side patches | Needs a workspace to apply patches to. |
| `openrouter:advisor` | Deferred | Another model, via OpenRouter | Sends the conversation to a second model, which is a new data flow and cost. |
| `openrouter:subagent` | Deferred | Other models, via OpenRouter | Same concern as `advisor`. |
| `openrouter:image_generation` | Deferred | An image model, via OpenRouter | Needs image output in messages and storage. |
| `openrouter:web_fetch` | Not planned | OpenRouter or third-party engines | Adding it makes OpenRouter run the tool loop, which switches search to Exa (see the spike). |
| `openrouter:shell` | Not planned | OpenAI's shell or OpenRouter's sandbox | Overlaps with `bash`. On OpenAI models it runs in OpenAI's shell, which is a separate environment. |
| `openrouter:fusion` | Not planned | A panel of models | Sends the conversation to several models. |
| `openrouter:experimental__search_models`, `openrouter:tool_search` | Not planned | OpenRouter | Meta-tools for agent setups. They don't help in a chat. |

"Deferred" and "Not planned" tools behave the same way in the UI: they're listed in the
**+** pill's tooltip when the model could use them upstream.

## Decisions

| Area | Decision |
|---|---|
| Default | Every supported tool is off in every chat. |
| Scope of the toggle | Per chat, stored with the chat like `model`. New chats start with everything off. |
| Model without support | No pill, and the tool isn't sent. The chat keeps its stored toggle, so it applies again after switching back to a model that supports it. |
| Routing | Every request with a tool sends `provider.zdr: true`. Requests without tools are unchanged. |
| Mixing tools | `web_search` (native) and `bash` (OpenRouter sandbox) **can't be on together**. Turning one on turns the other off. Mixing them makes OpenRouter run the loop and send search to Exa. |
| Silent fallback | Detected from usage fields, logged as a warning, and shown on the message. |
| Spend | Tool fees are part of `usage.cost`, so the existing spend view already covers them. |

## Capability discovery

Sprinter decides per model which pills to show, using two OpenRouter lists that it
fetches together with the model list. They share its 1 h in-memory cache and `?refresh=1`
bypass (see [10-api.md](10-api.md)):

- `GET /api/v1/models`: `supported_parameters` tells whether a model accepts `tools`.
- `GET /api/v1/endpoints/zdr`: every ZDR endpoint (917 on 2026-09-29), with a
  `native_tools` map, for example:

  ```json
  { "model_id": "openai/gpt-6-luna", "provider_name": "Azure", "tag": "azure",
    "native_tools": { "openrouter:web_search": {"type": "web_search"},
                      "openrouter:apply_patch": {"type": "apply_patch"} } }
  ```

How availability is decided for each tool:

| Tool | Available when |
|---|---|
| `web_search` | ZDR endpoints of the model list `openrouter:web_search` with an accepted native type (`web_search`, `google_search`). Coverage is `full` when every ZDR endpoint lists it, and `partial` when some do. |
| `bash` | The model has at least one ZDR endpoint and supports `tools`. The tool runs in OpenRouter's sandbox, not in the model provider. |
| Deferred and not-planned tools (for the **+** pill) | Tools that run natively (e.g. `apply_patch`): listed in `native_tools` on a ZDR endpoint. Tools OpenRouter runs itself: the model supports `tools`. |

- `GET /api/models` gains `tools: [{id, coverage}]` (supported tools) and `upstream_tools:
  [id]` (unsupported tools available upstream) per model.
- **Fail closed:** if `/endpoints/zdr` can't be fetched, every model reports no tools and
  the **+** pill is hidden. The rest of the model list still works.
- Two sources look useful but must **not** be used:
  - The `native_tools` entry `bash_20250124` means Anthropic's client-side bash, not the
    sandbox. It doesn't affect `bash` availability.
  - `pricing.web_search` is `"0.01"` even for endpoints without native search, such as
    Claude Haiku 4.5 on Bedrock.
- The account's own privacy toggles can't be read through the API. Sending
  `provider.zdr: true` makes routing use exactly the `/endpoints/zdr` list, so discovery
  and routing agree whatever the account settings are.

## Composer UI

- Pills sit in the composer's bottom row after **Attach**, in catalog order, then the
  **+** pill.
- **Tool pill:** a HeroUI toggle rendered as a pill, with an icon and label ("Web search"
  with `Globe`, "Bash" with `SquareTerminal`) and `aria-pressed`.
  - Off: outlined in the muted border.
  - On: `--accent-soft` fill and the accent outline.
  - Tooltip: where the tool runs and what it costs (see each tool below).
- **+ pill:** disabled, showing a `Plus` icon only, with the tooltip
  "{Image generation, Advisor, …} are supported upstream but not by Sprinter". It lists
  this model's `upstream_tools` by label and is hidden when the list is empty. It's
  focusable, so the tooltip works with the keyboard and on touch screens.
- Only tools the chat's current model supports are shown. On narrow screens tool pills
  show only the icon, and the label moves to the tooltip.
- Turning on `web_search` while `bash` is on (or the reverse) turns the other one off.
  The pill's tooltip says so: "Can't be combined with Bash: search would run through a
  third-party engine."
- Toggling saves right away with `PATCH /api/chats/:id {tools}`. In a new, unsaved chat,
  the state is held in the composer and sent with the first message.
- Pills are disabled offline and while a generation is streaming, like **Attach**.
- The regenerate menu uses the chat's toggles. If the model picked for the retry doesn't
  support a tool that's on, the tool is dropped for that retry.
- The message footer (see [01-chat.md](01-chat.md)) lists the tools that were enabled,
  the number of searches or commands, and the tool fees.

## Web search

### Request

Added to the request from [15-messages-api-migration.md](15-messages-api-migration.md):

```json
{
  "provider": { "zdr": true },
  "tools": [ { "type": "openrouter:web_search", "parameters": { "engine": "native" } } ],
  "max_tool_calls": 5
}
```

- For `partial` coverage, `provider.only` lists the capable endpoints. Whether `only`
  accepts endpoint tags such as `google-vertex/global` is unverified (see the open
  questions). If it doesn't, `partial` is treated as unavailable.
- `max_tool_calls` caps cost. One question led to four native searches ($0.04) in the spike.
- Tooltip: "Runs inside {provider} under zero data retention routing. ~$0.01 per search."

Coverage on 2026-09-29:

| Family | ZDR provider | Native type | Coverage |
|---|---|---|---|
| OpenAI gpt-5.5, gpt-5.6 / gpt-6 Luna, Sol, Terra, Astra (+Pro) | Azure | `web_search` | full |
| Google Gemini 3.x Flash, Pro, Flash-Lite | Vertex | `google_search` | full |
| xAI Grok 4.20, 4.3, 4.5, grok-build | xAI | `web_search` | full |
| xAI Grok 4.6, 4.7 | xAI | `web_search` | partial (2/4, 2/3) |
| Anthropic claude-opus-4.5 | Vertex global | `web_search_20260209` | none: type not accepted |
| Other Anthropic models | Bedrock, Vertex, Azure | none | none |

Claude's native search exists only on the first-party `anthropic` endpoint, which is not
ZDR. The Vertex `web_search_20260209` version uses dynamic filtering, which Anthropic says
is not ZDR-eligible by default, so it isn't an accepted type.

### Stream

A native search looks like this on the Messages API:

1. One `server_tool_use` block per search (`name: "openrouter:web_search"`). Its `input` is
   always `{}`, so the queries are never exposed.
2. `text` blocks with the reply.
3. One `citations_delta` per source inside a text block, arriving **after** that block's
   text:

   ```json
   {"type": "citations_delta", "citation": {"type": "web_search_result_location",
     "url": "https://spectrum.ieee.org/delhi-electricity-loss",
     "title": "Here’s How Delhi Achieved Its Epic Power-Grid Fix",
     "cited_text": "", "encrypted_index": ""}}
   ```

   There's no position in the text and no excerpt.

### Results in the UI

- **Progress:** each `server_tool_use` start sends a `tool_step` event. Until the first
  text arrives, the reply shows "Searching… (2)" instead of "Thinking…".
- **Citations:** stored as `{url, title}`, de-duplicated by URL. They show as a row of
  source chips under the reply: mono domain plus title, each linking to the URL with
  `rel="noopener noreferrer"`. The inline `([domain](url))` links the model writes are
  left as they are in the Markdown.

### Fallback detection

OpenRouter doesn't report which engine it used in the response. The final usage is
enough to tell:

| Signal in `message_delta.usage` | Native run | OpenRouter-run (third-party) |
|---|---|---|
| `server_tool_use` | `web_search_requests` (and `web_fetch_requests: 0`) only | Also `tool_calls_requested` / `tool_calls_executed` |
| `cost - cost_details.upstream_inference_cost` | 0 | The engine fee, e.g. $0.007 per Exa search |

If either signal shows an OpenRouter-run search, Sprinter:
1. Logs a `WARN` in the generation span.
2. Stores `tool_fallback = 1` on the message. The footer then shows "Search was handled
   by a third-party engine."
3. Marks the model's `web_search` coverage as `none` until the next catalog refresh.

The search has already happened by then, so this can't prevent it. The `native_tools`
pre-check, `provider.zdr` and the rule against combining tools are what keep it rare.

## Bash

### How it runs

- `openrouter:bash` with `engine: "openrouter"` runs commands in an **OpenRouter-hosted
  Linux sandbox**. It's not run by Sprinter or the model provider.
  - The other engines (`auto`, `native`) return the command to the caller to run, which
    Sprinter will never do.
  - Containers are account-scoped and never shared across tenants.
  - It works for any model that supports tools (tested with Claude, GPT and Gemini).
- **Network is always disabled** (`network_policy: {type: "disabled"}`), so a command
  can't send chat content anywhere. Allowlisted network access (e.g. for `pip`) is
  deferred.
- The sandbox is keyed by the request's `session_id`, which is the chat's
  `session_id` from [15-messages-api-migration.md](15-messages-api-migration.md). Files
  persist across turns of a chat and aren't shared with other chats.
  - Branches of a chat share the sandbox. Its state isn't branch-aware, which is accepted.
  - Containers sleep after 5 minutes of inactivity. What happens to their files after
    that, and whether they can be deleted, is unverified (see the open questions).
- **Cost:** $0.0001 per second of sandbox time. Starting or waking a container is billed
  at least 30 seconds ($0.003), while a warm container bills only the time used. The fee
  arrives as `usage.cost_details.server_tool_cost`, and it's included in `usage.cost`.
- Tooltip: "Runs commands in an OpenRouter sandbox with no internet access. Files persist
  within this chat. ~$0.003 per session start."
- OpenRouter documents the tool as beta. It only works on the global endpoint
  (`openrouter.ai`), not on in-region endpoints.

### Request

Added to the request from [15-messages-api-migration.md](15-messages-api-migration.md):

```json
{
  "provider": { "zdr": true },
  "tools": [ { "type": "openrouter:bash", "parameters": {
      "engine": "openrouter",
      "environment": { "type": "container_auto", "network_policy": { "type": "disabled" } } } } ],
  "max_tool_calls": 10
}
```

### Results

The stream carries each step as content blocks:

1. `server_tool_use` (`name: "openrouter:bash"`), with `input_json_delta` deltas
   building `{"command": "…"}`.
2. `openrouter_bash_tool_result`, with `content: {command, stdout, stderr, exitCode}`
   and `container_id`.
3. `text` blocks with the reply. Text can come before, between and after steps.

Sprinter stores each step with the **content offset** at which it happened. The UI
renders it inline at that point as a collapsed block ("Ran `printf … | sha256sum`" plus
an exit-code badge). Expanding it shows stdout and stderr in mono. While a command is
running, the block shows a spinner.

Earlier turns are sent back to the model as text only. Tool steps from previous turns
are not replayed, because the files they produced are still in the sandbox and the reply
text describes what happened.

## Date

`openrouter:datetime` isn't used. Every generation request instead ends the `system`
field with the user's **local date** (no time of day):

```
Today's date for the user: Tuesday, 2026-09-29 (time zone America/Sao_Paulo).
```

- **Date only, to keep prompt caching.** The system prompt is the start of the prompt,
  and providers cache prompts by matching prefixes. A date changes it at most once a day,
  while a time of day would change it on every turn and invalidate the cache for the
  whole conversation. The model can't answer "what time is it", which is accepted.
- **It must be the user's local date, not the server's.** The server container runs in
  UTC, and near midnight the two dates differ.
  - The client sends its IANA zone
    (`Intl.DateTimeFormat().resolvedOptions().timeZone`) as `timezone` on send, edit and
    regenerate.
  - The server validates it with `chrono-tz` and takes the current date in that zone.
- If `timezone` is missing or invalid, the line is **left out**. It never falls back to
  the server's zone. The event is logged at `DEBUG`.
- The date is taken when the request is built. A regenerate gets the date of the
  regeneration.
- `system` is always sent now, even with empty custom instructions. This changes
  "Empty means no system message" in [07-settings.md](07-settings.md).
- Title generation doesn't get the line.

## Stream handling

These plug into the parser from
[15-messages-api-migration.md](15-messages-api-migration.md):

| Stream content | Handling |
|---|---|
| `server_tool_use` start, `openrouter:web_search` | `tool_step` event with status `running`, for search progress |
| `server_tool_use` start and `input_json_delta`, `openrouter:bash` | Build the command. A `tool_step` with status `running` is sent at `content_block_stop`. |
| `openrouter_bash_tool_result` start | `tool_step` with status `done` and the output |
| `citations_delta` of type `web_search_result_location` | Add `{url, title}` to the message's citations, and send a `citations` event |

New SSE events:

```
event: citations   data: {"items":[{"url":"…","title":"…"}]}
event: tool_step   data: {"id":"toolu_…","tool":"openrouter:bash","offset":0,"status":"running","input":{"command":"…"}}
event: tool_step   data: {"id":"toolu_…","tool":"openrouter:bash","offset":0,"status":"done","output":{"stdout":"…","stderr":"","exit_code":0}}
event: tool_step   data: {"id":"ws_…","tool":"openrouter:web_search","offset":0,"status":"running"}
```

`snapshot` gains `citations` and `tool_steps`, so late subscribers get everything.

## Data model changes

```sql
ALTER TABLE chats ADD COLUMN tools TEXT NOT NULL DEFAULT '[]';   -- JSON array of enabled tool ids

ALTER TABLE messages ADD COLUMN tools TEXT;              -- assistant: JSON array of tool ids sent with this request
ALTER TABLE messages ADD COLUMN citations TEXT;          -- assistant: JSON [{url,title}]
ALTER TABLE messages ADD COLUMN tool_steps TEXT;         -- assistant: JSON [{id,tool,offset,input,output}]
ALTER TABLE messages ADD COLUMN web_search_requests INTEGER;
ALTER TABLE messages ADD COLUMN tool_cost REAL;          -- USD, cost minus upstream inference cost
ALTER TABLE messages ADD COLUMN tool_fallback INTEGER NOT NULL DEFAULT 0;
```

- Citations and tool output are **not** indexed in `search_fts`. Search covers what the
  user and model wrote.
- Export (Markdown and JSON) includes citations as a list under the reply, and bash steps
  as fenced blocks at their offsets.
- Tool steps are flushed with the content (every ~1 s) during streaming, so a crash keeps
  completed steps.

## API changes

| Endpoint | Change |
|---|---|
| `GET /api/models` | Each model gains `tools: [{id, coverage}]` and `upstream_tools: [id]`. |
| `PATCH /api/chats/:id` | Accepts `tools: string[]`. Unknown ids and ids that aren't supported tools return `400 unknown_tool`. `web_search` together with `bash` returns `400 incompatible_tools`. Tools the current model doesn't support are accepted and stored. |
| `POST /api/chats/:id/messages`, `/api/chats/new/messages` | Accept `timezone?` (IANA). The new-chat call also accepts `tools?`. |
| `POST /api/messages/:id/regenerate` | Accepts `timezone?`. |
| `GET /api/chats/:id` | Returns `tools` on the chat, and `tools`, `citations`, `tool_steps`, `web_search_requests`, `tool_cost`, `tool_fallback` on messages. |
| SSE | New `citations` and `tool_step` events, and both fields in `snapshot`. |

## Logging

Added to the event catalog in [13-logging.md](13-logging.md):

| Event | Level | Fields |
|---|---|---|
| Generation started | INFO | adds `tools=[…]`, `tz` |
| Tool step | INFO | `tool`, `exit_code`, `duration_ms`, `stdout_bytes`, `stderr_bytes`. The command preview follows the content-preview rules. |
| Generation completed | INFO | adds `web_search_requests`, `citations`, `tool_steps`, `tool_cost` |
| Tool fallback | WARN | `tool`, `model`, `provider`, `tool_cost` |
| ZDR endpoint catalog refreshed | INFO | `endpoints`, `models_with_tools` |
| ZDR endpoint catalog failed | WARN | `error`. Pills are hidden until the next successful refresh. |

Citation URLs and full command output are logged at `DEBUG` only.

## Testing

- The fake OpenRouter gains:
  - `/api/v1/endpoints/zdr` with fixed data: a `full` web search model, a `partial` one, a
    tools model without native search, and a model without tools.
  - A native search stream (`server_tool_use` blocks with `{}` input, text, then
    `citations_delta`) with native-style usage, and a `[[tool-fallback]]` trigger that
    returns Exa-style usage.
  - A bash stream with a `server_tool_use` block, a result block and text.
- Backend tests:
  - Availability per tool.
  - Fail-closed behavior when `/endpoints/zdr` errors.
  - Request bodies with `provider.zdr`, the tool parameters and the network policy.
  - Citation de-duplication, tool step offsets and fallback detection.
  - The date line: the user's date differs from the UTC date near midnight, and there's
    no line for a missing or invalid zone.
- E2E (`tools` spec):
  - Pills appear only for models that support them and default to off, and the **+** pill
    tooltip lists the upstream-only tools.
  - Turning a pill on persists across a reload.
  - Web search and bash turn each other off.
  - Switching to a model without support hides the pill and drops `tools` from the
    request.
  - Search progress, citation chips and bash steps render, and the fallback warning shows.

## Implementation order

1. The date in the system prompt. It's independent and small.
2. Capability discovery, the pills (web search plus the **+** pill), the web search
   request, progress and citations, and fallback detection.
3. Bash: tool steps and their rendering.

## Open questions

1. **`provider.only` with endpoint tags.** `partial` web search routing needs to restrict
   to specific endpoints such as `google-vertex/global`. Verify that `only` accepts tags.
   If it doesn't, treat `partial` as unavailable.
2. **Sandbox lifetime.** How long a sleeping container keeps its files, and whether it can
   be deleted when the chat is deleted. OpenRouter documents container file APIs
   (list, download, promote to workspace documents), which may help.
3. **Toggle scope.** Per chat for now. A global default in Settings plus a per-chat
   override could come later.
4. **`max_uses` for native search.** Check whether `parameters.max_uses` caps native
   searches, which would be tighter than `max_tool_calls`.
5. **Local tools.** A self-hosted SearXNG search plus a local fetch tool would cover
   models with no ZDR native search, such as Claude. It's out of scope here, but the
   catalog and pills should be able to hold local tools later.

## OpenRouter server tools spike (2026-09-29)

These tests ran against the live API with an account whose privacy settings enforce ZDR
for the OpenAI and Anthropic model groups. Scripts and raw captures were kept outside the
repo. The first rows used Chat Completions, before the migration was decided. They're
kept because they show the same engine behavior.

| Request | Endpoint used | Result |
|---|---|---|
| Chat Completions, `gpt-6-luna`, `web_search` (native) only | Azure (ZDR) | Native search: `web_search_engine: "native"`, no fee beyond upstream |
| Chat Completions, `gpt-6-luna`, `web_search` (native) + `web_fetch` (`openrouter`) | Azure (ZDR) | Exa: $0.007 per search, `tool_calls_requested` present |
| Chat Completions, `claude-haiku-4.5`, `web_search` (native) | Bedrock (ZDR) | Exa |
| Chat Completions, `provider.only` set to first-party `openai` / `anthropic` | none | 404: "ZDR violation (account settings)" |
| Chat Completions, `claude-haiku-4.5`, `bash` (`openrouter`) | none | 400: only supported for `anthropic-messages` |
| Messages API, `claude-haiku-4.5`, `bash` (`openrouter`), streaming | Bedrock (ZDR) | Command ran. Result block with `stdout`/`stderr`/`exitCode`. Sandbox fee $0.003 (30 s minimum). |
| Messages API, `gpt-6-luna` and `gemini-3.5-flash`, `bash` (`openrouter`), same `session_id` | ZDR | Both ran. The second request reused the warm container: fee $0.0002. |
| Messages API, `gpt-6-luna`, `web_search` (native) only, streaming | Azure (ZDR) | Native search, no fee beyond upstream. Two `server_tool_use` blocks with `{}` input, then text, then one `citations_delta` (`web_search_result_location`, empty `cited_text`). |
| Messages API, `gpt-6-luna`, `web_search` (native) + `bash` (`openrouter`) | ZDR | `tool_calls_requested` present and upstream cost near zero: the pattern of an OpenRouter-run (third-party) search |

- `engine: "native"` never errors. When native search isn't available, it silently
  becomes Exa.
- Any tool OpenRouter runs itself (`web_fetch`, `bash`) makes OpenRouter run the loop for
  the whole request, and native search stops being native.
- Native search never exposes its queries or results. One question led to four searches
  ($0.041), and `max_results` had no visible effect.

Sources: [server tools](https://openrouter.ai/docs/guides/features/server-tools),
[web search server tool](https://openrouter.ai/docs/guides/features/server-tools/web-search),
[bash server tool](https://openrouter.ai/docs/guides/features/server-tools/bash),
[shell server tool](https://openrouter.ai/docs/guides/features/server-tools/shell),
[web fetch server tool](https://openrouter.ai/docs/guides/features/server-tools/web-fetch),
[web search plugin](https://openrouter.ai/docs/guides/features/plugins/web-search),
[ZDR](https://openrouter.ai/docs/guides/features/zdr), and
[Anthropic feature ZDR eligibility](https://platform.claude.com/docs/en/build-with-claude/overview).
