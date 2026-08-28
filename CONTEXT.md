# Lunar

A terminal coding harness. Rust host, Lua 5.5 guest. Pi’s philosophy, not a Pi port.

You open `lunar` and talk. The binary does not dictate workflow (no MCP, sub-agents, plan mode, todos, permission theatre, background bash). Capabilities are files (skills, context) or later Lua. The host stays small.

## How we build

- Simplicity. No speculative abstraction. No slot trait hierarchy until Lua actually replaces a slot.
- Shallow interfaces, deep implementations. A module hides a lot; its surface stays small.
- Do not code before asking questions.
- Small pieces. One vertical slice you can run, then the next.
- Commits are conventional (`feat:`, `fix:`, `docs:`).

## Locked decisions

| Decision | Choice |
|---|---|
| Product shape | Pi-shaped: Rust is the program, Lua is a guest |
| Workflow in the binary | None |
| Extension model | Slots (replaceable parts). Hook bus does **not** ship in v0. S only for now |
| Config | User `~/.lunar/control/init.lua`, overridden by CWD `.lunar/init.lua`. No model configuration via `LUNAR_*`. `LUNAR_HOME` and `LUNAR_PROMPT_BUDGET` stay env. On startup, legacy root files move into `control/` and `recorder/`; existing destinations are not overwritten, directory contents merge around conflicts, and unresolved legacy paths remain readable |
| Lua load | `~/.lunar/control/init.lua`, then CWD `.lunar/init.lua` (or `$LUNAR_HOME/control/init.lua` for user config). No auto-load directories. Syntax/runtime error in either = notice, glass opens, cannot send |
| Trust | Not implemented. Project `.lunar/init.lua` runs automatically this slice |
| Language | Lua 5.5.1, vendored via `mlua` (`lua55` + `vendored`). Embed this slice |
| Lua guest API | `init.lua` returns one table containing optional `models`, `providers`, and `defaults` tables. The registrar form does not exist. **No `lunar.on`** (hook bus is not v0) |
| Model catalog | Top-level `models`: alias → `{ id, window?, api?, thinking? }`. `id` is the wire string; alias is a Lua name. Provider `models` is an ordered list: string = ref to a global alias, table = local `{ id, window?, api?, thinking? }`. Missing alias = notice, skip that entry. Missing `id`, unknown `api`, or invalid `thinking` = notice, skip that entry. Model `thinking` is an ordered table of model-specific wire values plus `default`, for example `{ "low", "high", "max", default = "high" }`; `default` must be listed. Omitted `thinking` resolves to `{ "off", default = "off" }`. Provider-level and scalar `thinking` do not exist. Omitted `api` is Completions |
| Live model | Top-level `defaults = { provider, model }`. `provider` is a providers key; `model` matches that list as alias then wire `id`. Unknown provider or model is an error: notice, glass opens, cannot send. Omitted defaults = no live Config; the catalog remains available in `/model`. Partial defaults (only one field) = present and invalid. The selected provider must have `base_url_cmd` or `base_url` unless `key_in = "auth"` and `auth_provider` is `xai` or `openai` (then `https://api.x.ai/v1` or `https://chatgpt.com/backend-api`). `key_in = "none"` requires an explicit `base_url`; HTTP and HTTPS are accepted. Plus `key_cmd` or `key_name` (`key_in = "env"`), or `auth_provider` (`key_in = "auth"`); missing the applicable source = notice, cannot send. `base_url_cmd` takes precedence over `base_url`, which takes precedence over an auth default. Live `api` `"messages"` is resolve-time refuse: notice, cannot send, entry stays in `/model`. Completions and Responses send. Live Config carries optional `auth_provider` from Lua `key_in = "auth"`. `"openai"` means Codex Responses + JWT account header; Completions on that auth is refuse. Do not sniff `base_url`. Model `window` if set, else the Grok-id guess. Footer provider is the providers key |
| Prompt conventions | Rules come from CWD `AGENTS.md`, falling back to `~/.agents/AGENTS.md`; project rules replace global rules. CWD `CONTEXT.md` is included in full. Skill *summaries* merge from `~/.agents/skills/*/SKILL.md` and `.agents/skills/*/SKILL.md`; project skills replace global skills with the same directory name. Optional bundled skills live in `skills/` and are installed manually; none are enabled by default. |
| System prompt | None. Context is a leading user message, snapshotted at each user submit, held for the tool loop, not persisted |
| v0 goal | Daily driver for one user, not ecosystem parity |
| Model protocol | Completions, Responses, and Messages. Selected by model `api`: `"completions"` · `"responses"` · `"messages"`. Case-sensitive, no aliases. Omitted `api` is Completions. Completions and Responses send. Responses stays `store: false`, requests an automatic reasoning summary for the thinking preview, and replays the full converted history each round. When a mission exists, Responses also sends `prompt_cache_key` (mission id, 64 chars max) and Pi affinity headers `session_id` / `x-client-request-id`. No `previous_response_id`. ChatGPT Plus (`auth_provider = "openai"`) is Responses-only: POST `{base_url}/codex/responses` (not `{base_url}/responses`), plus `chatgpt-account-id` decoded from the access JWT at send and refresh (`https://api.openai.com/auth` → `chatgpt_account_id`; missing or empty = notice, cannot send) and `originator: lunar`. No extra `auth.json` field. Completions on that auth is resolve-time refuse. No websocket and no zstd this slice. Messages is not implemented |
| First brand | xAI. No provider is compiled into configuration |
| Auth | Env named by Lua, shell command, Lunar-managed auth, or none. With `key_in = "env"`, `key_cmd` runs through `sh -c` and supplies the secret, otherwise `key_name` names an env secret. `key_in = "auth"` names a canonical built-in integration with `auth_provider` (`xai` or `openai`) and resolves API-key or OAuth credentials from `~/.lunar/recorder/auth.json`. `key_in = "none"` sends no Authorization header and requires an explicit `base_url`; `key_name`, `key_cmd`, and `auth_provider` are ignored. `/login` is a provider picker (`xAI`, `OpenAI`). Enter on xAI opens the existing method picker. Enter on OpenAI starts device-code immediately. `/login xai` opens the xAI method picker. `/login openai` is ChatGPT Plus/Pro subscription OAuth only (no stored platform API key this slice): device-code, same glass as xAI (open URL, show user code, poll, Esc cancels). No browser PKCE and no localhost callback this slice. `/logout xai` / `/logout openai` remove that credential; `/logout` with no argument notices usage. The xAI device flow uses the public client ID distributed by Pi. The OpenAI device flow uses the public Codex client ID distributed by Pi |
| Thinking | Each model defines its ordered allowed wire values and default with `thinking = { "low", "high", "max", default = "high" }`. `/thinking` opens a one-line picker containing only the live model’s values; left/right selects and Enter applies. `/thinking <level>` applies directly only when that level is allowed by the live model. It changes the running Config for the current mission and is appended to that mission’s JSONL; reopening the mission restores its last level when the selected model still allows it. Before the first prompt it is held in memory and written when the mission is created. `/new` and model selection return to that model’s configured default. Omitted model `thinking` resolves to only `off`. Completions sends `reasoning_effort`, Responses sends `reasoning.effort`; the literal `off` omits effort. Footer shows the live level |
| TUI | `ratatui` + `crossterm` |
| Transcript | The current mission. Scrollable: every message in that mission is reachable as painted (tool cards stay 8 lines, thinking stays a 3-line preview). Not a tail-only view. `/resume` switches missions; there is no Session history object |
| Tools | `read` / `write` / `edit` (`old_string`/`new_string`) / `bash`. Gate = allow. Bash timeout 60s, Esc kills the process group. Bash stdin is null; on Unix the child is a new session so a nested TUI cannot take the glass. Tool results cap 50KB or 2000 lines per result. `read` keeps the head and gives the next offset. `bash` keeps the tail; truncated bash output is saved under `~/.lunar/recorder/tool-output/` and the path is included in the result. Files older than seven days are deleted at startup. `finish_reason=length` does not execute tool calls. Calls in one assistant turn run in parallel. Tool loops pause after 50 rounds; submitting `continue` starts a fresh turn |
| Missions | Linear append-only jsonl. Not a tree. Not Pi-compatible. Each model usage result is appended; reopening sums the mission totals for the footer and restores the latest prompt size. Mission lists are ordered by file modification time, newest first; opening or listing a mission does not modify it, while prompts, model/thinking changes, and renames bump it |
| Full context | Warn and refuse submit. No auto-compact. `/compact` only after this hurts |
| Entry | `lunar` always opens the TUI. No print mode in v0 |
| Providers in source | None hardcoded. Unconfigured is a valid first run |

## On disk

```
~/.lunar/                    # or $LUNAR_HOME
  control/                   # user-authored Lua
    init.lua                 # user setup; this slice
    lua/
  recorder/                  # Lunar-owned files
    missions/                # flat; cwd is in the jsonl header
      2026-08-19-1.jsonl     # date-local N, monotonic for the day
    trust.json               # not used yet
    auth.json                # Lunar-managed API keys and OAuth tokens
    debug.log                # model HTTP traffic when started with --debug
    history.jsonl            # submitted-line history
    tool-output/             # full truncated bash output; 7-day startup cleanup

.lunar/
  init.lua                   # project config overrides
```

Mission headers persist a short semantic `name` derived locally from the first user prompt. The displayed session title is `<id> - <name>`; `/name` rewrites the header name. Existing mission headers may be backfilled in place. UI shows `mission: <id> - <name>`.

## Environment

Model configuration lives in `init.lua`. These host settings remain environment variables:

| | |
|---|---|
| `LUNAR_PROMPT_BUDGET` | optional; warn at startup if context files + skill summaries exceed it (default 16000) |
| `LUNAR_HOME` | overrides `~/.lunar` |

## User and project `init.lua`

Host runs `~/.lunar/control/init.lua` (or `$LUNAR_HOME/control/init.lua`) and then CWD `.lunar/init.lua` once at startup. Both return the same shape. Project `models` and `providers` replace matching user entries by key while unmatched user entries remain. Project `defaults`, when present, replaces user `defaults`; when omitted, user defaults remain. There is no trust check this slice. The optional top-level fields are `models`, `providers`, and `defaults`; the old registrar form does not exist. **No `lunar.on`.**

```lua
return {
  models = {
    grok46 = {
      id = "grok-4.6",
      window = 500000,
      api = "completions",
      thinking = { "low", "high", "max", default = "high" },
    },
  },

  providers = {
    xai = {
      base_url = "https://api.x.ai/v1",
      -- base_url_cmd = "pass lunar/xai-url" -- shell command; takes precedence over base_url
      key_name = "XAI_API_KEY",
      -- key_cmd = "pass my_key" -- shell command; takes precedence over key_name
      -- key_in = "env", -- default if omitted
      -- key_in = "auth", auth_provider = "xai", -- ~/.lunar/recorder/auth.json via /login
      models = {
        "grok46", -- ref: alias → global catalog
        {
          id = "grok-4.5",
          api = "completions",
          thinking = { "off", "low", "high", default = "low" },
        },
      },
    },
  },

  defaults = {
    provider = "xai",
    model = "grok46", -- alias, else wire id in that provider's list
  },
}
```

- **Provider name** is the table key, not a `name` field.
- **Alias** is a Lua name (`grok46`). **`id`** is the wire string (`grok-4.6`). Every model def requires `id`; skip that entry if missing. `api` is `"completions"` · `"responses"` · `"messages"`; unknown value = notice, skip that entry (same as missing `id`). Omitted `api` is Completions, not an error.
- Provider `models` is an **ordered list**. String = ref to a global alias (missing alias = notice, skip). Table = local `{ id, window?, api?, thinking? }`.
- This slice honors **`id`**, optional **`window`**, optional **`api`**, and optional model **`thinking`**. `api` and `thinking` are only on a model def. `thinking` is an ordered list of that model’s non-empty wire values with a required `default` that must appear in the list. Duplicate values keep their first position. Omitted `thinking` resolves to `{ "off", default = "off" }`. Provider-level and scalar `thinking` are invalid. A string ref inherits the alias’s values. Omitted `api` is Completions. xAI models set `api` explicitly.
- **`key_name`** is the env var to read when `key_in` is `"env"`. Token never sits in Lua.
- **`key_cmd`** is an alternative when `key_in` is `"env"`. It runs through `sh -c` at config resolution, before Lunar enters TUI mode, so interactive credential helpers such as GPG pinentry can use the terminal; trailing whitespace is trimmed. Non-zero exit, non-UTF-8 output, or an empty key = notice, cannot send. When both are set, `key_cmd` wins.
- **`key_in`** defaults to `"env"`. `"env"` reads `key_cmd` or `key_name`; `"auth"` reads `~/.lunar/recorder/auth.json` for `auth_provider` (`xai` or `openai`); `"none"` sends no Authorization header, ignores credential fields, and requires an explicit `base_url`. Any other value = notice, cannot send.
- **`defaults`**: both `provider` and `model` are required when the table is present. `model` matches that provider's list as alias, then as wire `id`.

**Resolve Config**

| Situation | Result |
|---|---|
| No `init.lua` | Unconfigured; glass opens, cannot send |
| File exists, syntax or runtime error | Notice, glass opens, cannot send |
| Returned table omits `defaults` | Catalog loads; no live Config until `/model` selects one |
| `defaults` contains only one field | Present and invalid: notice, cannot send |
| Unknown provider or model | Notice, cannot send |
| Selected provider missing both `base_url_cmd` and `base_url` (unless `key_in = "auth"` and `auth_provider` is `xai` or `openai`), an explicit `base_url` (`none`), both `key_cmd` and `key_name` (`env`), or `auth_provider` (`auth`) | Notice, cannot send |
| `key_in` not `"env"`, `"auth"`, or `"none"`, env lookup empty, `base_url_cmd` or `key_cmd` fails/returns no value, or no saved auth | Notice, cannot send |
| Defaults resolve | Live Config from Lua |
| Live model `api` is `"messages"` | Resolve-time refuse: no live Config. Notice (`claude uses messages, not implemented`). Entry stays in `/model`, dimmed. Completions and Responses siblings stay pickable |
| Lua `window` set | Use it |
| Lua `window` omitted | Grok-id guess from wire `id` (same as today) |
| Footer provider (Lua path) | The providers key |

Glass always opens. Unconfigured (or Lua error) still cannot send.

## TUI (what is on screen)

Top to bottom:

1. **Header (1 line)** — `lunar <ver>` left, `mission: <name>` right
2. **Transcript** — empty state is the Lua-logo moon (disk + gold satellite), not an astronaut. After the first **prompt** (not `/help`), splash goes away
   - User: Pi `#343541` bar, `#d4d4d4` text, 1-cell pad
   - Thinking: 3-line italic ash preview + `...` (from `reasoning_content` / `reasoning` / `reasoning_text`). Not persisted
   - Tools: green card, title + 8 lines of result
   - Assistant: terminal-friendly CommonMark (gold headings, emphasis, inline/fenced code, valid fenced `json` pretty-printed, links with visible destinations, lists, blockquotes, rules). Tables use aligned Unicode borders and wrap cells to fit the transcript. Images render alt text plus URL
3. **Working** — `⠋ Thinking...` while the model streams; `⠋ Running tools...` while tools run
4. **Editor** — top **and** bottom rules, char-wrap, grows/shrinks (max 8 lines), real cursor
5. **Footer (2 lines)** — cwd; then cumulative `↑total (Uuncached Rread Wwrite) ↓output` and latest `ctx tokens/window (pct)` left, `(provider) model • off` right. The cache split appears only when cache activity is reported; zero `R`/`W` components are omitted

Readline: Ctrl+W / Alt+Backspace word-kill, Ctrl+U/K, Ctrl+A/E, arrows, Alt+B/F, Delete. Enter sends. Shift+Enter / Ctrl+J insert a newline. **Ctrl+C quits** (not clear). Esc aborts a turn or clears the editor.

Transcript scroll: PageUp / PageDown, mouse wheel, Ctrl+Home / Ctrl+End to top / bottom. Arrows, j/k, and Home/End stay with the editor (and `/resume`). No scroll mode. The wheel always moves the transcript; pointer position is ignored. PageUp / PageDown move one transcript pane minus one line. Wheel moves 3 painted lines per notch. Clamp at both ends. Follow the tail only when the viewport already shows the last line. Resize and editor grow keep the top painted line, or the tail if you were already there, then clamp. Scrolled-up view stays put while the stream grows. Submit, `/new`, and `/resume` jump to the tail. A notice that arrives while scrolled up does not snap; it is another transcript line. Mouse capture is on so the wheel works; Shift-drag copies. No click-to-focus and no click-to-place the caret this slice.

`/` opens command completion under the editor. Tab / ↑↓ cycle; Enter accepts (runs, except `/name` which stays in the editor). `/q` is a hidden quit alias.

`/resume` is j/k + enter. Press `/` to search mission ID, name, cwd, and body with a case-insensitive substring; arrows move through matches, Enter opens one, and Esc clears search before closing the picker. PageUp / PageDown / wheel do nothing in the picker. `lunar -c` / `--continue` loads the latest mission for this cwd. `lunar -m` / `--mission` without an argument opens the mission log; with an argument it resumes an exact filename (with or without `.jsonl`) or the newest exact label for this cwd. If no mission matches, it opens the mission log. A date (`YYYY-MM-DD`) opens that day's filtered log, unless an exact label matches first. `-c` and `-m` are mutually exclusive.

## Shipped vs not

**Shipped**

- Glass, Completions and Responses streams, reused HTTP agent, no global timeout. Completions `max_tokens` 32768 (reasoning + answer). POST retries 429/5xx/reset (3 times, 0.5s…8s, Esc cancels the wait). Completions `finish_reason` / Responses `response.completed` ends the turn; leftover Completions SSE is drained for usage (1s cap) and `[DONE]` in the background so the socket can return to the pool. A computer resume detected during an active turn interrupts the stale turn, preserves partial output, and returns control to the user. Transcript scroll: PageUp / PageDown, wheel, Ctrl+Home / Ctrl+End; follow only at the tail. History paint is cached; only the streaming tail is re-wrapped
- Four tools + continue-after-tools (50-round cap; submit `continue` to proceed). Tools in one round run in parallel. Results cap 50KB or 2000 lines; truncated bash output is saved under `~/.lunar/recorder/tool-output/`. Truncated completions do not run tool calls
- Missions: `/new` `/resume` `/name` `/mission`, `-c`
- Token stats + refuse submit when last prompt ≥ window
- Global/project `AGENTS.md`, CWD `CONTEXT.md`, and merged global/project skill summaries as a leading user message, snapshotted per user turn
- Commands that exist: `/quit` `/q` `/help` `/new` `/resume` `/model` `/thinking` `/login` `/logout` `/name` `/mission` `/context`. `/context` opens a component summary of the live preamble and current history, with count and estimated-token breakdowns for user messages, assistant messages, tool calls, and tool results, plus a preamble + history total; `/context raw` shows their full contents, including tool calls and results. Both use a pager: PageUp/PageDown, j/k, wheel, and Ctrl+Home/End scroll, while Esc or q closes it
- Lua 5.5 embed; user `~/.lunar/control/init.lua` returns `{ models, providers, defaults }`
- Thinking levels: each model supplies its ordered values and default; `/thinking` only accepts and displays those values; Completions and Responses wire mappings; live level in footer

**Not shipped (still v0 intent)**

- Walk-up discovery, skill bodies (only summaries ship)
- `/reload` `/trust`
- Messages API (catalog accepts `api`; Completions and Responses send)
- Cost in the footer, git branch, `$` prices
- `/compact`

## Slots (foundation, Rust-filled)

`model` · `tools` · `prompt` · `compact` · `session` · `ui` · `gate` · `commands` · `keys`

These are a *list*, not a trait hierarchy in the code. Do not introduce one until Lua replaces a slot. `ui` must not leak `ratatui` types.

## Not v0

Package manager, print/RPC/SDK, Pi session compatibility, provider zoo, themes, prompt templates, `/tree` `/fork` `/clone`, auto-compact, hook bus (`on`), `terminal.lua` as the glass.

## Next slices (recommended order)

1. Messages, if you actually use a Messages-only id
2. Dumb `/compact` after the window hurts
3. Walk-up context

## Layout in the repo

`src/main.rs` app/TUI · `auth.rs` managed credentials + xAI / OpenAI OAuth · `protocol/` HTTP (`stream` + Completions / Responses adapters) · `lua.rs` user + project `init.lua` · `tools.rs` four tools · `tool_output.rs` truncated bash files · `mission.rs` jsonl · `prompt.rs` CWD context + skill summaries · `render.rs` transcript paint · `splash.rs` art + colors
