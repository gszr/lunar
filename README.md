<p align="center">
  <img src="docs/logos/lunar-logo-with-wordmark-transparent.png" alt="Lunar" width="220">
</p>

<p align="center">A terminal coding harness. You open <code>lunar</code> and talk.</p>

Rust host, Lua guest. Lunar gives the model four tools—`read`, `write`, `edit`, and `bash`—without imposing a workflow.

## Install

Requires Rust.

```bash
cargo install --git https://github.com/gszr/lunar
```

## Configure

Set an API key, an OpenAI Chat Completions base URL, and a model:

```bash
export LUNAR_API_KEY=...
export LUNAR_BASE_URL=https://api.x.ai/v1
export LUNAR_MODEL=grok-4.6
```

Any OpenAI Chat Completions-compatible endpoint should work.

For lasting configuration, create `~/.lunar/init.lua`:

```lua
lunar.models {
  grok46 = {
    id = "grok-4.6",
    window = 500000,
  },
}

lunar.providers {
  xai = {
    base_url = "https://api.x.ai/v1",
    key_name = "XAI_API_KEY",
    models = {
      "grok46"
    },
  },
}

lunar.defaults {
  provider = "xai",
  model = "grok46",
}
```

Then provide the named secret:

```bash
export XAI_API_KEY=...
```

## Run

```bash
lunar
```

Type a prompt and press Enter. Run `/help` to see the available commands. Use `lunar -c` to continue the latest mission for the current directory.

Lunar includes `AGENTS.md`, `CONTEXT.md`, and summaries from `.agents/skills/*/SKILL.md` in its context.

## Extensions

Lua extensions are planned. For example, you will be able to replace local `bash` with execution in a [Runta](https://runta.ai) runtime:

```lua
-- Planned API; not implemented yet.
lunar.tools.bash = function(cmd)
  local runtime = os.getenv("LUNAR_RUNTIME") or "demo"
  return lunar.sh { "runta", "exec", runtime, "--", "sh", "-lc", cmd }
end
```

## Status

Lunar is early software for macOS and Linux. It currently supports OpenAI Chat Completions endpoints; extensions, the Responses API, and built-in login are not implemented yet.

## License

[MIT](LICENSE)
