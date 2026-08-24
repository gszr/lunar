<p align="center">
  <img src="docs/logos/lunar-logo-with-wordmark-transparent.png" alt="Lunar" width="220">
</p>

<p align="center">A terminal coding harness. You open <code>lunar</code> and code</p>

<p align="center">Simple and fast like <a href="https://github.com/earendil-works/pi">Pi &lt;3</a>, but using Lua for extensions</p>

<p align="center">Lunar gives the model four tools—<code>read</code>, <code>write</code>, <code>edit</code>, and <code>bash</code>. No hidden system prompts</p>

<p align="center">No token wasting on hidden system prompts or excessive tool schema</p>

## Install

```bash
brew install --HEAD gszr/taps/lunar
```

Or with Cargo:

```bash
cargo install --git https://github.com/gszr/lunar
```

## Configure

Configure Lunar with `~/.lunar/init.lua`. A project can override model aliases and providers by key, and optionally replace `defaults`, in `.lunar/init.lua`. Copy an example from [`examples/`](examples/), or from Lunar, `/config` opens the user file with `$VISUAL` or `$EDITOR`, then reloads both files when the editor exits:

```lua
return {
  models = {
    grok46 = {
      id = "grok-4.6",
      window = 500000,
      api = "completions",
    },
  },

  providers = {
    xai = {
      base_url = "https://api.x.ai/v1",
      key_name = "XAI_API_KEY",
      models = { "grok46" },
    },
  },

  defaults = {
    provider = "xai",
    model = "grok46",
  },
}
```

```bash
export XAI_API_KEY=...
```

A provider can use `base_url_cmd = "pass lunar/xai-url"` instead of `base_url`, and `key_cmd = "pass my_key"` instead of `key_name`. Commands run through `sh -c` before the TUI opens, so interactive credential helpers such as GPG pinentry work normally. Lunar trims trailing whitespace from stdout. `base_url_cmd` takes precedence over `base_url`; `key_cmd` takes precedence over `key_name`.

Alternatively, let Lunar store the credential. Use `/login` in the TUI: xAI (subscription via device code, or a masked API key) or OpenAI (ChatGPT Plus/Pro via device code). Set `key_in = "auth"` and `auth_provider = "xai"` or `"openai"` on the provider. `/logout xai` and `/logout openai` remove the credential.

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

Lunar is early software for macOS and Linux. Model `api` selects Completions, Responses, or Messages; Completions and Responses send. Lua extensions are not implemented yet.

## License

[MIT](LICENSE)
