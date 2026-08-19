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
| Config | Lua, when present. Exact `setup` table unspecified |
| Lua load | `~/.lunar/init.lua`, then trusted `.lunar/init.lua`. No auto-load directories |
| Trust | Project Lua runs only after an explicit trust decision (`trust.json`) |
| Language | Lua 5.5.1, vendored via `mlua` (`lua55` + `vendored`) — **not embedded yet** |
| Prompt conventions | CWD `AGENTS.md` + `CONTEXT.md` in full. Skill *summaries* from `.agents/skills/*/SKILL.md`. `~/.agents/skills` later |
| System prompt | None. Context is a leading user message, rebuilt every request, not persisted |
| v0 goal | Daily driver for one user, not ecosystem parity |
| Model protocol | Completions **and** Responses. **Only Completions is implemented** |
| First brand | xAI. Config is env, not a compiled-in brand |
| Auth | Env for now. `/login xai` (device-code + key → `~/.lunar/auth.json`) is still todo. Own OAuth client; do not steal Pi’s |
| TUI | `ratatui` + `crossterm` |
| Tools | `read` / `write` / `edit` (`old_string`/`new_string`) / `bash`. Gate = allow. Bash timeout 60s, Esc kills |
| Missions | Linear append-only jsonl. Not a tree. Not Pi-compatible |
| Full context | Warn and refuse submit. No auto-compact. `/compact` only after this hurts |
| Entry | `lunar` always opens the TUI. No print mode in v0 |
| Providers in source | None hardcoded. Unconfigured is a valid first run |

## On disk

```
~/.lunar/                    # or $LUNAR_HOME
  missions/                  # flat; cwd is in the jsonl header
    2026-08-19-1.jsonl       # date-local N, monotonic for the day
  init.lua                   # not loaded yet
  lua/
  trust.json                 # not used yet
  auth.json                  # not used yet

.lunar/
  init.lua                   # not loaded yet
```

Default mission label is the filename. `/name` overrides. UI shows `mission: <name>`.

## Env (what actually talks)

| | |
|---|---|
| `LUNAR_API_KEY` | required to send |
| `LUNAR_BASE_URL` | required (e.g. `https://api.x.ai/v1`) |
| `LUNAR_MODEL` | required (e.g. `grok-4.6`) |
| `LUNAR_PROVIDER` | optional label; else inferred from URL |
| `LUNAR_CONTEXT_WINDOW` | optional; else inferred for some Grok ids |
| `LUNAR_PROMPT_BUDGET` | optional; warn at startup if context files + skill summaries exceed it (default 16000) |
| `LUNAR_HOME` | overrides `~/.lunar` |

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

Readline: Ctrl+W / Alt+Backspace word-kill, Ctrl+U/K, Ctrl+A/E, arrows, Alt+B/F, Delete. **Ctrl+C quits** (not clear). Esc aborts a turn or clears the editor.

`/resume` is j/k + enter. `lunar -c` / `--continue` loads the latest mission for this cwd.

## Shipped vs not

**Shipped**

- Glass, Completions stream, reused HTTP agent, no global timeout
- Four tools + continue-after-tools (20-round cap)
- Missions: `/new` `/resume` `/name` `/session`, `-c`
- Token stats + refuse submit when last prompt ≥ window
- CWD `AGENTS.md` / `CONTEXT.md` + `.agents/skills` summaries as a leading user message
- Commands that exist: `/quit` `/q` `/help` `/new` `/resume` `/name` `/session`

**Not shipped (still v0 intent)**

- Walk-up discovery, `~/.agents/skills`, skill bodies (only summaries ship)
- `/login` `/logout` `/model` `/reload` `/trust`
- Responses API
- Lua 5.5 embed / `init.lua`
- Thinking level (footer says `off` and it is not wired)
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
3. Lua loader for `init.lua` setup only
4. Dumb `/compact` after the window hurts
5. Walk-up context + `~/.agents/skills`

## Layout in the repo

`src/main.rs` app/TUI · `complete.rs` HTTP · `tools.rs` four tools · `mission.rs` jsonl · `prompt.rs` CWD context + skill summaries · `render.rs` transcript paint · `splash.rs` art + colors
