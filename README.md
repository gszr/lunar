<p align="center">
  <img src="docs/logos/lunar-logo-with-wordmark-transparent.png" alt="Lunar" width="220">
</p>

<p align="center">
  A terminal coding harness.<br>
  You open <code>lunar</code> and talk.
</p>

Rust host, Lua 5.5 guest. No MCP, no sub-agents, no plan mode, no permission theatre. Capabilities are files — or Lua. The host stays small.

## Run

```bash
cargo run
```

Or install it and open the glass from anywhere:

```bash
cargo install --path .
lunar
```

The TUI always opens. With no model configured you can look around; you just cannot send.

Continue the last mission for this directory:

```bash
lunar -c
```

See all supported command-line options:

```bash
lunar --help
```

## Configure

Two ways. Pick one — they are not mixed.

### Environment

Fastest path to a first message:

```bash
export LUNAR_API_KEY=...
export LUNAR_BASE_URL=https://api.x.ai/v1
export LUNAR_MODEL=grok-4.6
lunar
```

Any OpenAI Chat Completions endpoint works. Nothing is hardcoded to a brand.

| Variable | |
|---|---|
| `LUNAR_API_KEY` | required to send |
| `LUNAR_BASE_URL` | required |
| `LUNAR_MODEL` | required |
| `LUNAR_PROVIDER` | optional label; else inferred from the URL |
| `LUNAR_CONTEXT_WINDOW` | optional; else inferred for some Grok ids |
| `LUNAR_PROMPT_BUDGET` | optional, default `16000` |
| `LUNAR_HOME` | optional, default `~/.lunar` |

`LUNAR_HOME` and `LUNAR_PROMPT_BUDGET` always apply. The rest are ignored once Lua defaults resolve.

### `~/.lunar/init.lua`

The lasting way. Create `~/.lunar/init.lua` (or `$LUNAR_HOME/init.lua`). When `lunar.defaults` resolves, that is the live config — `LUNAR_MODEL` / `LUNAR_BASE_URL` / `LUNAR_API_KEY` are ignored. The API token still comes from the environment. Lua never holds it.

```lua
lunar.models {
  grok46 = { id = "grok-4.6", window = 500000 },
}

lunar.providers {
  xai = {
    base_url = "https://api.x.ai/v1",
    key_name = "XAI_API_KEY",
    models = {
      "grok46",            -- alias from lunar.models
      { id = "grok-4.5" }, -- local def
    },
  },
}

lunar.defaults {
  provider = "xai",
  model = "grok46",
}
```

- **Alias** (`grok46`) is the Lua name. **`id`** (`grok-4.6`) is what goes on the wire.
- `key_name` is the env var to read. Required on the selected provider.
- Both `provider` and `model` are required in `defaults`. `model` matches that provider's list as an alias, then as a wire id.

A syntax or runtime error is a notice. The glass still opens; it does not fall back to env.

## Talk

Type and press **Enter** to send. **Shift+Enter** or **Ctrl+J** inserts a newline.

| Key | |
|---|---|
| Enter | send |
| Shift+Enter / Ctrl+J | newline |
| Esc | abort a turn, or clear the editor |
| Ctrl+C | quit |
| `/` then Tab | cycle commands |

Readline works as you'd hope: Ctrl+A/E, Ctrl+W, Alt+Backspace, Ctrl+U/K, arrows, Alt+B/F, Delete.

### Commands

| | |
|---|---|
| `/help` | list commands |
| `/new` | start a new mission |
| `/resume` | pick a mission in this directory |
| `/name` | name the current mission |
| `/session` | show the mission path |
| `/context` | files and skills sent every prompt |
| `/quit` | quit (`/q` works too) |

`/login` is not in yet.

## Tools

The model can `read`, `write`, `edit`, and `bash`. They run when asked. Bash times out at 60s; Esc kills it.

## Context

Every request rebuilds a leading user message from the repo. It is not persisted.

- `AGENTS.md` and `CONTEXT.md` in the current directory, in full
- Skill summaries from `.agents/skills/*/SKILL.md` (name + description). Lunar reads the file when it uses one.

If that plus your history would exceed the model window, submit is refused. There is no auto-compact yet.

## Missions

Conversations are linear, append-only jsonl under `~/.lunar/missions/` (or `$LUNAR_HOME/missions/`). Not a tree. The default name is the filename; `/name` overrides it. The UI shows `mission: <name>`.

## Not yet

Walk-up discovery, skill bodies, `/login` `/logout` `/model` `/reload` `/trust`, Responses API, thinking level, cost in the footer, `/compact`.

Pi's philosophy, not a Pi port.
