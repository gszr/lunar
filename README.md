<p align="center">
  <img src="docs/logos/lunar-logo-with-wordmark-transparent.png" alt="Lunar" width="220">
</p>

<p align="center">A terminal coding harness. You open <code>lunar</code> and code.</p>

<p align="center">Simple and fast like <a href="https://github.com/earendil-works/pi">Pi &lt;3</a>, but using Lua for extensions.</p>

<p align="center">Lunar gives the model four tools—<code>read</code>, <code>write</code>, <code>edit</code>, and <code>bash</code>—without imposing a workflow.</p>

## Install

```bash
brew install --HEAD gszr/taps/lunar
```

Or with Cargo:

```bash
cargo install --git https://github.com/gszr/lunar
```

## Configure

With no `~/.lunar/init.lua`, set an API key, an OpenAI Chat Completions base URL, and a model:

```bash
export LUNAR_API_KEY=...
export LUNAR_BASE_URL=https://api.x.ai/v1
export LUNAR_MODEL=grok-4.6
```

Any OpenAI Chat Completions-compatible endpoint should work.

For lasting configuration, create `~/.lunar/init.lua`. From Lunar, `/config` opens this file with `$VISUAL` or `$EDITOR`, then reloads it when the editor exits. Read the key from the environment:

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

```bash
export XAI_API_KEY=...
```

Or let Lunar store the credential. Use `/login` in the TUI (xAI subscription via device code, or a masked API key). `/logout` removes it.

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
    key_in = "auth",
    auth_provider = "xai",
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

## Run

```bash
lunar
```

Type a prompt and press Enter. Run `/help` to see the available commands. Use `/config` to edit and reload `init.lua`, `lunar -c` to continue the latest mission for the current directory, or `lunar -m` to open the mission log (`lunar -m <mission>` resumes a filename or label). Passing a date such as `lunar -m 2026-08-19` opens that day's mission log.

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

Lunar is early software for macOS and Linux. It currently supports OpenAI Chat Completions endpoints. Lua extensions and the Responses API are not implemented yet.

## License

[MIT](LICENSE)

<!-- Runta PR after stub fix. -->
