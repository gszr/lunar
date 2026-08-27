<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/logos/dark/lunar-dark-theme-with-wordmark-transparent.png">
    <source media="(prefers-color-scheme: light)" srcset="docs/logos/light/lunar-logo-with-wordmark-transparent.png">
    <img src="docs/logos/light/lunar-logo-with-wordmark-transparent.png" alt="Lunar" width="220">
  </picture>
</p>

<p align="center">A terminal coding harness. You open <code>lunar</code> and code</p>

<p align="center">Simple and fast like <a href="https://github.com/earendil-works/pi">Pi &lt;3</a>, but using Lua for extensions</p>

<p align="center">Lunar gives the model four tools—<code>read</code>, <code>write</code>, <code>edit</code>, and <code>bash</code>. No hidden system prompts</p>

<p align="center">No token wasting on hidden system prompts: a harness that works for <em>you</em></p>

## Lunar Agent Harness

Lunar is deliberately small: a Rust host owns the terminal, model transport, and four tools, while a Lua guest supplies model and provider configuration. It does not impose plan modes, sub-agents, approval prompts, or another workflow between you and the model.

Context stays inspectable. Lunar sends your project's `AGENTS.md`, `CONTEXT.md`, and skill summaries as a visible user message—never a hidden system prompt. Conversations are durable, linear missions you can leave and resume.

## Features

- Streaming terminal UI with Markdown rendering, reasoning previews, and full transcript scrolling
- Parallel `read`, `write`, `edit`, and `bash` tool calls with cancellable turns
- Lua 5.5 model catalogs, provider configuration, defaults, and project overrides
- Provider URLs and tokens resolved from Lua, environment variables, shell commands, or Lunar-managed authentication
- Completions and Responses APIs, including xAI and ChatGPT Plus/Pro authentication
- Linear JSONL missions (Lunar's sessions! :) with naming, resume, token usage, and context-window protection
- Project instructions and skill summaries loaded directly from files you control

## Install

```bash
brew install --HEAD gszr/taps/lunar
```

Or with Cargo:

```bash
cargo install --git https://github.com/gszr/lunar
```

## Configure

Configure Lunar with `~/.lunar/control/init.lua`. A project can override model aliases and providers by key, and optionally replace `defaults`, in `.lunar/init.lua`. Copy an example from [`examples/`](examples/), or from Lunar, `/config` opens the user file with `$VISUAL` or `$EDITOR`, then reloads both files when the editor exits:

```lua
return {
  models = {
    grok46 = { id = "grok-4.6", window = 500000, api = "completions" },
    gpt56sol = { id = "gpt-5.6-sol", window = 1000000, api = "responses" },
    -- gpt56sol = { id = "gpt-5.6-sol", window = 1000000, api = "completions" },
  },

  providers = {
    ollama = {
      base_url = "http://127.0.0.1:11434/v1",
      models = {
        { id = "gemma4:12b", window = 10000, }
      },
      key_in = "none",
    },

    openai = {
      key_in = "auth",
      auth_provider = "openai",
      models = {
        "gpt56sol",
      },
    },

    openai_api = {
      base_url = "https://api.openai.com/v1",
      key_cmd = "pass openai_api_key",
      models = {
        "gpt56sol",
      },
    },

    dev2 = {
      base_url_cmd = "pass jss_dev2_url",
      key_in = "env",
      key_cmd = "pass jss_dev2",
      models = {
        "gpt56sol",
      },
    },

    stag2 = {
      base_url_cmd = "pass jss_stag2_url",
      key_in = "env",
      key_cmd = "pass jss_stag2",
      models = {
        "gpt56sol",
      },
    },

    xai = {
      base_url = "https://api.x.ai/v1",
      key_in = "auth",
      auth_provider = "xai",
      models = {
        "grok46",
      },
    },

    cheapinf = {
      base_url = "https://api.cheaperinference.com/v1",
      key_name = "CHEAP_INF_API_KEY",
      models = {
        "gpt56sol",
      },
    },
  },

  defaults = {
    provider = "openai",
    model = "gpt-5.6-sol", -- alias, else wire id in that provider's list
  },
}
```

```bash
export XAI_API_KEY=...
```

A provider can use `base_url_cmd = "pass lunar/xai-url"` instead of `base_url`, and `key_cmd = "pass my_key"` instead of `key_name`. Commands run through `sh -c` before the TUI opens, so interactive credential helpers such as GPG pinentry work normally. Lunar trims trailing whitespace from stdout. `base_url_cmd` takes precedence over `base_url`; `key_cmd` takes precedence over `key_name`.

Alternatively, let Lunar store the credential. Use `/login` in the TUI: xAI (subscription via device code, or a masked API key) or OpenAI (ChatGPT Plus/Pro via device code). Set `key_in = "auth"` and `auth_provider = "xai"` or `"openai"` on the provider. `/logout xai` and `/logout openai` remove the credential.

For an unauthenticated local server, set `key_in = "none"` and an explicit HTTP or HTTPS `base_url`. Lunar sends no Authorization header:

```lua
ollama = {
  base_url = "http://localhost:11434/v1",
  key_in = "none",
  models = { { id = "qwen3", api = "completions" } },
}
```

ChatGPT Plus/Pro models must set `api = "responses"`. Omitted `base_url` is `https://chatgpt.com/backend-api`; Lunar posts to `{base_url}/codex/responses`:

```lua
return {
  providers = {
    openai = {
      key_in = "auth",
      auth_provider = "openai",
      models = {
        { id = "gpt-5.4", api = "responses" },
      },
    },
  },
  defaults = {
    provider = "openai",
    model = "gpt-5.4",
  },
}
```

## Run

```bash
lunar
```

Type a prompt and press Enter. Run `/help` to see the available commands. Use `/config` to edit and reload `init.lua`, `lunar -c` to continue the latest mission for the current directory, or `lunar -m` to open the mission log (`lunar -m <mission>` resumes a filename or label). Passing a date such as `lunar -m 2026-08-19` opens that day's mission log.

Lunar includes `AGENTS.md`, `CONTEXT.md`, and summaries from `.agents/skills/*/SKILL.md` in its context.

Optional skills live in [`skills/`](skills/) and are not enabled by default. To enable Lunar attribution on pull requests and issues for a project:

```bash
mkdir -p .agents/skills
cp -R /path/to/lunar/skills/lunar-attribution .agents/skills/
```

## Status

Lunar is early software for macOS and Linux.

## License

[MIT](LICENSE)
