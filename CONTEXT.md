# Lunar

A terminal coding harness. Rust host, Lua 5.5 guest. Pi’s philosophy, not a Pi port.

You open `lunar` and talk. The binary does not dictate workflow (no MCP, sub-agents, plan mode, todos, permission theatre, background bash). Capabilities are files (skills, context) or later Lua. The host stays small.

## How we build

- Simplicity. No speculative abstraction. No slot trait hierarchy until Lua actually replaces a slot.
- Shallow interfaces, deep implementations. A module hides a lot; its surface stays small.
- Do not code before asking questions.
- Small pieces. One vertical slice you can run, then the next.

## Locked decisions

| Decision | Choice |
|---|---|
| Product shape | Pi-shaped: Rust is the program, Lua is a guest |
| Workflow in the binary | None |
| Extension model | Slots (replaceable parts). Hook bus does **not** ship in v0 |
| Config | Lua, when present. Exact `setup` table unspecified |
| Lua load | `~/.lunar/init.lua`, then trusted `.lunar/init.lua`. No auto-load directories |
| Trust | Project Lua runs only after an explicit trust decision (`trust.json`) |
| Language | Lua 5.5.1, vendored via `mlua` (`lua55` + `vendored`) |
| Prompt conventions | Agent Skills + context files (`AGENTS.md` and the usual cousins), including `~/.agents/skills` |
| v0 goal | Daily driver for one user, not ecosystem parity |
| Model protocol | OpenAI Chat Completions **and** Responses |
| First brand | xAI (`api.x.ai/v1`). `/login xai` does SuperGrok/X Premium device-code **and** API key |
| Auth store | `~/.lunar/auth.json`. Env keys still work. Lunar needs its own xAI OAuth client |
| TUI | `ratatui` + `crossterm`. Pi’s four bands, skeleton chrome |
| Tools | `read` / `write` / `edit` (old_string/new_string) / `bash`. Default gate = allow |
| Missions | Linear append-only jsonl, not a tree, not Pi-compatible |
| Full context | Warn and refuse submit. No auto-compact. `/compact` only after this hurts |
| Entry | `lunar` always opens the TUI. No one-shot print mode in v0 |
| Providers in source | None hardcoded. Unconfigured is a valid first run |
| Look | Pi four bands + OpenCode empty-state. Splash (moon + astronaut) in an empty transcript, then gone. One dark lunar palette. No Pi hotkey novel, no session sidebar |

## On disk

```
~/.lunar/
  init.lua          -- optional user config
  lua/              -- package.path, require()d not auto-loaded
  missions/         -- linear jsonl; cwd is in the file header
    <timestamp>_<id>.jsonl
  trust.json
  auth.json

.lunar/
  init.lua          -- project config, after trust
```

`LUNAR_HOME` overrides `~/.lunar`.

## v0 product (day-1 binary)

Something you can live in for a week. Zero Lua files required.

- Skeleton TUI, top to bottom: **header** (what loaded) / **messages** / **editor** / **footer** (cwd, mission name, model, tokens used/window)
- Editor: multiline, `/` commands, paste text, Esc abort, Ctrl+C clear
- Stream assistant text, tool cards, thinking as collapsed-by-default transcript lines
- xAI login + static Grok model list + `/model` among those ids
- Four tools, allow-all gate, bash has a timeout, no background jobs
- Skills + context files in the default prompt (progressive disclosure)
- Missions: `/new`, `/resume`, `/name`, `lunar -c` continues last mission for this cwd
- Commands: `/quit` `/new` `/resume` `/name` `/model` `/session` `/reload` `/trust` `/help` `/login` `/logout`
- Full window: footer warning, submit refused
- Optional `init.lua` may overlay setup (model id, etc.). Slot *replacement* from Lua is not v0

### TUI is not

Replaceable editor, overlays, status-line kit, `@` file picker, `!` bash, message queue, image paste, thinking-level border as a feature, `terminal.lua` as the glass.

## Slots (foundation, Rust-filled in v0)

`model` · `tools` · `prompt` · `compact` · `session` · `ui` · `gate` · `commands` · `keys`

New customization = a new named slot, not an event. Lua fill comes after the binary is livable. `ui` must not leak `ratatui` types.

## Not v0

Package manager, print/RPC/SDK, Pi session compatibility, provider zoo, themes, prompt templates, `/tree` `/fork` `/clone`, auto-compact, hook bus (`on`), `terminal.lua` UI, stealing Pi’s xAI OAuth client id as a product strategy.

## Implementation order

1. Host loop + Completions/Responses stream + xAI auth (key + device-code)
2. Four tools + linear missions + skeleton TUI
3. Skills/context prompt + trust + `-c` + context meter + warn-and-stop
4. Optional Lua 5.5 loader for `init.lua` setup only
5. After the window hurts: dumb `/compact`
6. After you live in it: Lua slot replacement, then maybe a tiny lifecycle bus
