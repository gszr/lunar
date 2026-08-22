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
| Config | Lua, when present. User `~/.lunar/init.lua` first; project later. If defaults resolve, that is the live Config — `LUNAR_MODEL` / `LUNAR_BASE_URL` / `LUNAR_API_KEY` are ignored. No Lua or no defaults = today’s env path. `LUNAR_HOME` and `LUNAR_PROMPT_BUDGET` stay env |
| Lua load | `~/.lunar/init.lua` this slice (or `$LUNAR_HOME/init.lua`). Trusted `.lunar/init.lua` later. No auto-load directories. Syntax/runtime error = notice, glass opens, no env fallback |
| Trust | Project Lua runs only after an explicit trust decision (`trust.json`) |
| Language | Lua 5.5.1, vendored via `mlua` (`lua55` + `vendored`). Embed this slice |
| Lua guest API | Host injects `lunar`. `lunar.models { }`, `lunar.providers { }`, and `lunar.defaults { }` are dump-table registrars; last call wins, no merge. **No `lunar.on`** (hook bus is not v0) |
| Model catalog | Global `lunar.models`: alias → `{ id, window?, api? }`. `id` is the wire string; alias is a Lua name. Provider `models` is an ordered list: string = ref to a global alias, table = local `{ id, window?, api? }`. Missing alias = notice, skip that entry. Missing `id` or unknown `api` = notice, skip that entry. This slice reads **id**, optional **window**, and optional **api**. Omitted `api` is Completions |
| Live model | `lunar.defaults { provider, model }`. `provider` is a providers key; `model` matches that list as alias then wire `id`. Unknown provider or model is an error: notice, glass opens, cannot send. Omitted defaults = today’s env Config. Partial defaults (only one field) = present and invalid, no env fallback. On the Lua path the selected provider must have `base_url`, plus `key_name` (`key_in = "env"`) or `auth_provider` (`key_in = "auth"`); missing either = notice, cannot send. Live `api` `"messages"` is resolve-time refuse: notice, cannot send, entry stays in `/model`. Completions and Responses send. Model `window` if set, else the Grok-id guess; `LUNAR_CONTEXT_WINDOW` is env-path only. Footer provider is the providers key |
| Prompt conventions | CWD `AGENTS.md` + `CONTEXT.md` in full. Skill *summaries* from `.agents/skills/*/SKILL.md`. `~/.agents/skills` later |
| System prompt | None. Context is a leading user message, snapshotted at each user submit, held for the tool loop, not persisted |
| v0 goal | Daily driver for one user, not ecosystem parity |
| Model protocol | Completions, Responses, and Messages. Selected by model `api`: `"completions"` · `"responses"` · `"messages"`. Case-sensitive, no aliases. Omitted `api` is Completions. Completions and Responses send. Responses stays `store: false` and replays the full converted history each round. When a mission exists, Responses also sends `prompt_cache_key` (mission id, 64 chars max) and Pi affinity headers `session_id` / `x-client-request-id`. No `previous_response_id`. Messages is not implemented. Env path stays Completions until that path is removed |
| First brand | xAI. Config is env, not a compiled-in brand |
| Auth | Env or Lunar-managed auth. `key_in = "env"` names an env secret with `key_name`. `key_in = "auth"` names a canonical built-in integration with `auth_provider` (initially `xai`) and resolves API-key or OAuth credentials from `~/.lunar/auth.json`. `/login xai` supports xAI device-code subscription auth and masked API-key entry; `/logout [xai]` removes it. The xAI device flow uses the public client ID distributed by Pi. `LUNAR_API_KEY` / `LUNAR_BASE_URL` / `LUNAR_MODEL` remain the no-Lua path |
| TUI | `ratatui` + `crossterm` |
| Transcript | The current mission. Scrollable: every message in that mission is reachable as painted (tool cards stay 8 lines, thinking stays a 3-line preview). Not a tail-only view. `/resume` switches missions; there is no Session history object |
| Tools | `read` / `write` / `edit` (`old_string`/`new_string`) / `bash`. Gate = allow. Bash timeout 60s, Esc kills the process group. Bash stdin is null; on Unix the child is a new session so a nested TUI cannot take the glass. Tool results cap 50KB or 2000 lines per result. `read` keeps the head and gives the next offset. `bash` keeps the tail; truncated bash output is saved under `~/.lunar/tool-output/` and the path is included in the result. Files older than seven days are deleted at startup. `finish_reason=length` does not execute tool calls. Calls in one assistant turn run in parallel. Tool loops pause after 50 rounds; submitting `continue` starts a fresh turn |
| Missions | Linear append-only jsonl. Not a tree. Not Pi-compatible |
| Full context | Warn and refuse submit. No auto-compact. `/compact` only after this hurts |
| Entry | `lunar` always opens the TUI. No print mode in v0 |
| Providers in source | None hardcoded. Unconfigured is a valid first run |

## On disk

```
~/.lunar/                    # or $LUNAR_HOME
  missions/                  # flat; cwd is in the jsonl header
    2026-08-19-1.jsonl       # date-local N, monotonic for the day
  init.lua                   # user setup; this slice
  lua/
  trust.json                 # not used yet
  auth.json                  # Lunar-managed API keys and OAuth tokens
  tool-output/               # full truncated bash output; 7-day startup cleanup

.lunar/
  init.lua                   # trusted project Lua; later slice
```

Mission headers persist a short semantic `name` derived locally from the first user prompt. The displayed session title is `<id> - <name>`; `/name` rewrites the header name. Existing mission headers may be backfilled in place. UI shows `mission: <id> - <name>`.

## Env (no-Lua path)

Used when `~/.lunar/init.lua` is missing, or the file loaded and `lunar.defaults` was never called. Not mixed with a resolved Lua Config.

| | |
|---|---|
| `LUNAR_API_KEY` | required to send |
| `LUNAR_BASE_URL` | required (e.g. `https://api.x.ai/v1`) |
| `LUNAR_MODEL` | required (e.g. `grok-4.6`) |
| `LUNAR_PROVIDER` | optional label; else inferred from URL |
| `LUNAR_CONTEXT_WINDOW` | optional; else inferred for some Grok ids |
| `LUNAR_PROMPT_BUDGET` | optional; warn at startup if context files + skill summaries exceed it (default 16000). Always env |
| `LUNAR_HOME` | overrides `~/.lunar`. Always env |

## User `init.lua` (this slice)

Host injects `lunar` and runs `~/.lunar/init.lua` (or `$LUNAR_HOME/init.lua`) once at startup. No project file, no `trust.json`, no `/reload`. `lunar.models { }`, `lunar.providers { }`, and `lunar.defaults { }` are dump-table functions: last call replaces the whole table, no merge. **No `lunar.on`.**

```lua
lunar.models {
  grok46 = { id = "grok-4.6", window = 500000, api = "completions" },
}

lunar.providers {
  xai = {
    base_url = "https://api.x.ai/v1",
    key_name = "XAI_API_KEY",
    -- key_in = "env"  -- default if omitted
    -- key_in = "auth", auth_provider = "xai"  -- ~/.lunar/auth.json via /login
    models = {
      "grok46",            -- ref: alias → global catalog
      { id = "grok-4.5", api = "completions" }, -- local def
    },
  },
}

lunar.defaults {
  provider = "xai",
  model = "grok46",        -- alias, else wire id in that provider's list
}
```

- **Provider name** is the table key, not a `name` field.
- **Alias** is a Lua name (`grok46`). **`id`** is the wire string (`grok-4.6`). Every model def requires `id`; skip that entry if missing. `api` is `"completions"` · `"responses"` · `"messages"`; unknown value = notice, skip that entry (same as missing `id`). Omitted `api` is Completions, not an error.
- Provider `models` is an **ordered list**. String = ref to a global alias (missing alias = notice, skip). Table = local `{ id, window?, api? }`.
- This slice honors **`id`**, optional **`window`**, and optional **`api`**. `api` is only on a model def (global catalog or a local listed table). Provider tables and `lunar.defaults` have no `api`. A string ref inherits the alias’s `api`. Omitted `api` is Completions. xAI models set `api` explicitly.
- **`key_name`** is the env var to read when `key_in` is `"env"`. Token never sits in Lua.
- **`key_in`** defaults to `"env"`. `"env"` reads `key_name`; `"auth"` reads `~/.lunar/auth.json` for `auth_provider` (initially `xai`). Any other value = notice, cannot send.
- **`lunar.defaults`**: both `provider` and `model` required when the call is present. `model` matches that provider's list as alias, then as wire `id`.

**Resolve Config**

| Situation | Result |
|---|---|
| No `init.lua` | Env path |
| File exists, syntax or runtime error | Notice, glass opens, **no env fallback** |
| File loaded, `lunar.defaults` never called | Env Config (catalog unused) |
| `lunar.defaults` present, only one field | Present and invalid: notice, cannot send |
| Unknown provider or model | Notice, cannot send |
| Selected provider missing `base_url`, or `key_name` (`env`) / `auth_provider` (`auth`) | Notice, cannot send |
| `key_in` not `"env"` or `"auth"`, env lookup empty, or no saved auth | Notice, cannot send |
| Defaults resolve | Live Config from Lua. Ignore `LUNAR_MODEL` / `LUNAR_BASE_URL` / `LUNAR_API_KEY` / `LUNAR_PROVIDER` / `LUNAR_CONTEXT_WINDOW` |
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
   - Assistant: bone prose, gold headings, fenced code as a dim block
3. **Working** — `⠋ Thinking...` while the model streams; `⠋ Running tools...` while tools run
4. **Editor** — top **and** bottom rules, char-wrap, grows/shrinks (max 8 lines), real cursor
5. **Footer (2 lines)** — cwd; then `↑in ↓out R W pct/window` left and `(provider) model • off` right

Readline: Ctrl+W / Alt+Backspace word-kill, Ctrl+U/K, Ctrl+A/E, arrows, Alt+B/F, Delete. Enter sends. Shift+Enter / Ctrl+J insert a newline. **Ctrl+C quits** (not clear). Esc aborts a turn or clears the editor.

Transcript scroll: PageUp / PageDown, mouse wheel, Ctrl+Home / Ctrl+End to top / bottom. Arrows, j/k, and Home/End stay with the editor (and `/resume`). No scroll mode. The wheel always moves the transcript; pointer position is ignored. PageUp / PageDown move one transcript pane minus one line. Wheel moves 3 painted lines per notch. Clamp at both ends. Follow the tail only when the viewport already shows the last line. Resize and editor grow keep the top painted line, or the tail if you were already there, then clamp. Scrolled-up view stays put while the stream grows. Submit, `/new`, and `/resume` jump to the tail. A notice that arrives while scrolled up does not snap; it is another transcript line. Mouse capture is on so the wheel works; Shift-drag copies. No click-to-focus and no click-to-place the caret this slice.

`/` opens command completion under the editor. Tab / ↑↓ cycle; Enter accepts (runs, except `/name` which stays in the editor). `/q` is a hidden quit alias.

`/resume` is j/k + enter. PageUp / PageDown / wheel do nothing in the picker. `lunar -c` / `--continue` loads the latest mission for this cwd. `lunar -m` / `--mission` without an argument opens the mission log; with an argument it resumes an exact filename (with or without `.jsonl`) or the newest exact label for this cwd. If no mission matches, it opens the mission log. A date (`YYYY-MM-DD`) opens that day's filtered log, unless an exact label matches first. `-c` and `-m` are mutually exclusive.

## Shipped vs not

**Shipped**

- Glass, Completions and Responses streams, reused HTTP agent, no global timeout. Completions `max_tokens` 32768 (reasoning + answer). POST retries 429/5xx/reset (3 times, 0.5s…8s, Esc cancels the wait). Completions `finish_reason` / Responses `response.completed` ends the turn; leftover Completions SSE is drained for usage (1s cap) and `[DONE]` in the background so the socket can return to the pool. Transcript scroll: PageUp / PageDown, wheel, Ctrl+Home / Ctrl+End; follow only at the tail. History paint is cached; only the streaming tail is re-wrapped
- Four tools + continue-after-tools (50-round cap; submit `continue` to proceed). Tools in one round run in parallel. Results cap 50KB or 2000 lines; truncated bash output is saved under `~/.lunar/tool-output/`. Truncated completions do not run tool calls
- Missions: `/new` `/resume` `/name` `/mission`, `-c`
- Token stats + refuse submit when last prompt ≥ window
- CWD `AGENTS.md` / `CONTEXT.md` + `.agents/skills` summaries as a leading user message, snapshotted per user turn
- Commands that exist: `/quit` `/q` `/help` `/new` `/resume` `/model` `/login` `/logout` `/name` `/mission` `/context`
- Lua 5.5 embed; user `~/.lunar/init.lua` (`lunar.models` / `lunar.providers` / `lunar.defaults`)

**Not shipped (still v0 intent)**

- Walk-up discovery, `~/.agents/skills`, skill bodies (only summaries ship)
- `/reload` `/trust`
- Messages API (catalog accepts `api`; Completions and Responses send)
- Thinking level (footer says `off` and it is not wired). grok-4.6 ignores `reasoning_effort`; the `max_tokens` cap is the bound
- Cost in the footer, git branch, `$` prices
- `/compact`

## Slots (foundation, Rust-filled)

`model` · `tools` · `prompt` · `compact` · `session` · `ui` · `gate` · `commands` · `keys`

These are a *list*, not a trait hierarchy in the code. Do not introduce one until Lua replaces a slot. `ui` must not leak `ratatui` types.

## Not v0

Package manager, print/RPC/SDK, Pi session compatibility, provider zoo, themes, prompt templates, `/tree` `/fork` `/clone`, auto-compact, hook bus (`on`), `terminal.lua` as the glass.

## Next slices (recommended order)

1. Messages, if you actually use a Messages-only id
2. Trusted project `.lunar/init.lua`
3. Dumb `/compact` after the window hurts
4. Walk-up context + `~/.agents/skills`

## Layout in the repo

`src/main.rs` app/TUI · `auth.rs` managed credentials + xAI OAuth · `protocol/` HTTP (`stream` + Completions / Responses adapters) · `lua.rs` user `init.lua` · `tools.rs` four tools · `tool_output.rs` truncated bash files · `mission.rs` jsonl · `prompt.rs` CWD context + skill summaries · `render.rs` transcript paint · `splash.rs` art + colors
tools · `tool_output.rs` truncated bash files · `mission.rs` jsonl · `prompt.rs` CWD context + skill summaries · `render.rs` transcript paint · `splash.rs` art + colors
