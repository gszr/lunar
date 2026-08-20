# Lunar

A terminal coding harness. Rust host, Lua 5.5 guest. Pi’s philosophy, not a Pi port.

```bash
cargo run
```

`lunar` always opens the TUI. With no model configured you still get the glass.

## Talk

```bash
export LUNAR_API_KEY=...
export LUNAR_BASE_URL=https://api.x.ai/v1
export LUNAR_MODEL=grok-4.6
cargo run
```

Any OpenAI Chat Completions endpoint works. Nothing is hardcoded.

Or put the same thing in `~/.lunar/init.lua` (or `$LUNAR_HOME/init.lua`). When `lunar.defaults` resolves, `LUNAR_MODEL` / `LUNAR_BASE_URL` / `LUNAR_API_KEY` are ignored. The token still comes from the environment (`key_name`).

```lua
lunar.models {
  grok46 = { id = "grok-4.6", window = 500000 },
}

lunar.providers {
  xai = {
    base_url = "https://api.x.ai/v1",
    key_name = "XAI_API_KEY",
    models = { "grok46" },
  },
}

lunar.defaults {
  provider = "xai",
  model = "grok46",
}
```

A syntax or runtime error in `init.lua` is a notice; the glass still opens and does not fall back to env.

## Tools

The model has `read`, `write`, `edit`, and `bash`. Esc aborts a turn and kills bash.

## Keys

| | |
|---|---|
| Enter | send |
| Shift+Enter / Ctrl+J | newline |
| Esc | abort / clear |
| Ctrl+C | quit |
| `/quit` | quit |
| `/new` | start a new mission |
| `/resume` | pick a mission in this directory |
| `/name` | name the current mission |
| `/session` | show mission path |
| `/context` | files and skills sent every prompt |
| `/help` | commands |
| `/` then Tab | cycle commands |

Missions are linear jsonl under `~/.lunar/missions/` (or `$LUNAR_HOME`). `lunar -c` continues the last one for this directory.

CWD `AGENTS.md` and `CONTEXT.md` are sent in full. Skill name + description from `.agents/skills/*/SKILL.md` are listed; read the file to use one.

`/login` is not in yet.
