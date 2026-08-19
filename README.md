# Lunar

A terminal coding harness. Rust host, Lua later. Pi’s philosophy, not a Pi port.

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

## Tools

The model has `read`, `write`, `edit`, and `bash`. Esc aborts a turn and kills bash.

## Keys

| | |
|---|---|
| Enter | send |
| Esc | abort / clear |
| Ctrl+C | quit |
| `/quit` | quit |
| `/help` | commands |

Missions, `/login`, and Lua config are not in yet.
