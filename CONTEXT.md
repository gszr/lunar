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
| Model catalog | Global `lunar.models`: alias → `{ id, window? }`. `id` is the wire string; alias is a Lua name. Provider `models` is an ordered list: string = ref to a global alias, table = local `{ id, window? }`. Missing alias = notice, skip that entry. This slice reads **id** and optional **window** only |
| Live model | `lunar.defaults { provider, model }`. `provider` is a providers key; `model` matches that list as alias then wire `id`. Unknown provider or model is an error: notice, glass opens, cannot send. Omitted defaults = today’s env Config. Partial defaults (only one field) = present and invalid, no env fallback. On the Lua path the selected provider must have `base_url` and `key_name`; missing either = notice, cannot send. Model `window` if set, else the Grok-id guess; `LUNAR_CONTEXT_WINDOW` is env-path only. Footer provider is the providers key |
| Prompt conventions | CWD `AGENTS.md` + `CONTEXT.md` in full. Skill *summaries* from `.agents/skills/*/SKILL.md`. `~/.agents/skills` later |
| System prompt | None. Context is a leading user message, rebuilt every request, not persisted |
| v0 goal | Daily driver for one user, not ecosystem parity |
| Model protocol | Completions **and** Responses. **Only Completions is implemented** |
| First brand | xAI. Config is env, not a compiled-in brand |
| Auth | Env for now. A provider names a secret (`key_name` = env var, `key_in = "env"`). It does not hold the token. On the Lua path `key_name` is required; missing or empty lookup = notice, cannot send. `LUNAR_API_KEY` / `LUNAR_BASE_URL` / `LUNAR_MODEL` remain the no-Lua path. `/login xai` (device-code + key → `~/.lunar/auth.json`) is still todo. Own OAuth client; do not steal Pi’s |
| TUI | `ratatui` + `crossterm` |
| Transcript | The current mission. Scrollable: every message in that mission is reachable as painted (tool cards stay 8 lines, thinking stays a 3-line preview). Not a tail-only view. `/resume` switches missions; there is no Session history object |
| Tools | `read` / `write` / `edit` (`old_string`/`new_string`) / `bash`. Gate = allow. Bash timeout 60s, Esc kills. Calls in one assistant turn run in parallel |
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
  auth.json                  # not used yet

.lunar/
  init.lua                   # trusted project Lua; later slice
```

Default mission label is the filename. `/name` overrides. UI shows `mission: <name>`.

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
  grok46 = { id = "grok-4.6", window = 500000 },
}

lunar.providers {
  xai = {
    base_url = "https://api.x.ai/v1",
    key_name = "XAI_API_KEY",
    -- key_in = "env"  -- default if omitted
    models = {
      "grok46",            -- ref: alias → global catalog
      { id = "grok-4.5" }, -- local def
    },
  },
}

lunar.defaults {
  provider = "xai",
  model = "grok46",        -- alias, else wire id in that provider's list
}
```

- **Provider name** is the table key, not a `name` field.
- **Alias** is a Lua name (`grok46`). **`id`** is the wire string (`grok-4.6`). Every model def requires `id`; skip that entry if missing.
- Provider `models` is an **ordered list**. String = ref to a global alias (missing alias = notice, skip). Table = local `{ id, window? }`.
- This slice honors **`id`** and optional **`window`** only.
- **`key_name`** is the env var to read. Required on the selected provider. Token never sits in Lua.
- **`key_in`** defaults to `"env"`. Only `"env"` this slice; any other value = notice, cannot send.
- **`lunar.defaults`**: both `provider` and `model` required when the call is present. `model` matches that provider's list as alias, then as wire `id`.

**Resolve Config**

| Situation | Result |
|---|---|
| No `init.lua` | Env path |
| File exists, syntax or runtime error | Notice, glass opens, **no env fallback** |
| File loaded, `lunar.defaults` never called | Env Config (catalog unused) |
| `lunar.defaults` present, only one field | Present and invalid: notice, cannot send |
| Unknown provider or model | Notice, cannot send |
| Selected provider missing `base_url` or `key_name` | Notice, cannot send |
| `key_in` not `"env"`, or `key_name` lookup empty | Notice, cannot send |
| Defaults resolve | Live Config from Lua. Ignore `LUNAR_MODEL` / `LUNAR_BASE_URL` / `LUNAR_API_KEY` / `LUNAR_PROVIDER` / `LUNAR_CONTEXT_WINDOW` |
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
3. **Working** — `⠋ Thinking...` while a turn is in flight
4. **Editor** — top **and** bottom rules, char-wrap, grows/shrinks (max 8 lines), real cursor
5. **Footer (2 lines)** — cwd; then `↑in ↓out R W pct/window` left and `(provider) model • off` right

Readline: Ctrl+W / Alt+Backspace word-kill, Ctrl+U/K, Ctrl+A/E, arrows, Alt+B/F, Delete. Enter sends. Shift+Enter / Ctrl+J insert a newline. **Ctrl+C quits** (not clear). Esc aborts a turn or clears the editor.

Transcript scroll: PageUp / PageDown, mouse wheel, Ctrl+Home / Ctrl+End to top / bottom. Arrows, j/k, and Home/End stay with the editor (and `/resume`). No scroll mode. The wheel always moves the transcript; pointer position is ignored. PageUp / PageDown move one transcript pane minus one line. Wheel moves 3 painted lines per notch. Clamp at both ends. Follow the tail only when the viewport already shows the last line. Scrolled-up view stays put while the stream grows. Submit, `/new`, and `/resume` jump to the tail. A notice that arrives while scrolled up does not snap; it is another transcript line. Mouse capture is on so the wheel works; Shift-drag copies. No click-to-focus and no click-to-place the caret this slice.

`/` opens command completion under the editor. Tab / ↑↓ cycle; Enter accepts (runs, except `/name` which stays in the editor). `/q` is a hidden quit alias.

`/resume` is j/k + enter. PageUp / PageDown / wheel do nothing in the picker. `lunar -c` / `--continue` loads the latest mission for this cwd.

## Shipped vs not

**Shipped**

- Glass, Completions stream, reused HTTP agent, no global timeout. `max_tokens` 32768 (reasoning + answer). `finish_reason` ends the turn; leftover SSE is drained so the socket can return to the pool
- Four tools + continue-after-tools (20-round cap). Tools in one round run in parallel
- Missions: `/new` `/resume` `/name` `/session`, `-c`
- Token stats + refuse submit when last prompt ≥ window
- CWD `AGENTS.md` / `CONTEXT.md` + `.agents/skills` summaries as a leading user message
- Commands that exist: `/quit` `/q` `/help` `/new` `/resume` `/name` `/session` `/context`
- Lua 5.5 embed; user `~/.lunar/init.lua` (`lunar.models` / `lunar.providers` / `lunar.defaults`)

**Not shipped (still v0 intent)**

- Walk-up discovery, `~/.agents/skills`, skill bodies (only summaries ship)
- `/login` `/logout` `/model` `/reload` `/trust`
- Responses API
- Thinking level (footer says `off` and it is not wired). grok-4.6 ignores `reasoning_effort`; the `max_tokens` cap is the bound
- Cost in the footer, git branch, `$` prices
- `/compact`

## Slots (foundation, Rust-filled)

`model` · `tools` · `prompt` · `compact` · `session` · `ui` · `gate` · `commands` · `keys`

These are a *list*, not a trait hierarchy in the code. Do not introduce one until Lua replaces a slot. `ui` must not leak `ratatui` types.

## Not v0

Package manager, print/RPC/SDK, Pi session compatibility, provider zoo, themes, prompt templates, `/tree` `/fork` `/clone`, auto-compact, hook bus (`on`), `terminal.lua` as the glass, stealing Pi’s xAI OAuth client id.

## Next slices (recommended order)

1. `/login xai` (device-code + API key)
2. Responses, if you actually use a Responses-only id
3. Trusted project `.lunar/init.lua`
4. Dumb `/compact` after the window hurts
5. Walk-up context + `~/.agents/skills`

## Layout in the repo

`src/main.rs` app/TUI · `complete.rs` HTTP · `lua.rs` user `init.lua` · `tools.rs` four tools · `mission.rs` jsonl · `prompt.rs` CWD context + skill summaries · `render.rs` transcript paint · `splash.rs` art + colors
