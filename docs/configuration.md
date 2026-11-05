# Configuration

Configure Lunar with `~/.lunar/control/init.lua`. A project can override model aliases and providers by key, and optionally replace `defaults`, in `.lunar/init.lua`. If it is missing in the current directory, Lunar tries the immediate parent only; `init.lua` may `require` modules from `~/.lunar/control/<name>.lua` or `~/.lunar/control/<name>/init.lua`. Copy an example from [`examples/`](../examples/). `/config` opens the user file with `$VISUAL` or `$EDITOR`, then reloads both files when the editor exits.

## Provider configuration

```lua
return {
  models = {
    grok46 = { id = "grok-4.6", window = 500000, api = "completions" },
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
    model = "grok46", -- alias, else wire id in that provider's list
  },
}
```

```bash
export XAI_API_KEY=...
```

`defaults.thinking` is optional. It takes priority over each model's own default when that model supports the value; otherwise Lunar uses the model-specific default.

A provider can use `base_url_cmd = "pass lunar/xai-url"` instead of `base_url`, and `key_cmd = "pass my_key"` instead of `key_name`. Commands run through `sh -c` before the TUI opens, so interactive credential helpers such as GPG pinentry work normally. Lunar trims trailing whitespace from stdout. `base_url_cmd` takes precedence over `base_url`; `key_cmd` takes precedence over `key_name`.

## Model discovery

For a Responses-compatible provider, set `models = "auto"` instead of a model list. Lunar fetches `{base_url}/models` in the background using the provider’s credentials and adds the results to `/model`. Discovered models offer `off`, `low`, `medium`, `high`, and `max` thinking levels (`off` by default); `defaults.model` uses the wire ID. Use an explicit list for other protocols or model-specific thinking levels.

`/models refresh` re-fetches discovered catalogs using the loaded configuration,
without reloading Lua or changing the selected model. Existing catalogs remain
available during refresh and are kept on failure.

## Overrides and modules

Project model aliases and providers replace matching user entries by key;
unmatched user entries remain. Project `defaults` replaces user `defaults` as a
whole. A present project config wins even when empty. Invalid or unreadable files
are errors, not a reason to try the parent directory.

`require` searches only the user control directory, not the project or current
directory. Project Lua runs automatically; use configuration you trust.

Invalid configuration stops startup before the terminal interface opens.
Missing configuration is valid; unavailable credentials produce a recoverable
notice. Invalid reloads leave the last valid configuration active.

## Files and environment

User configuration lives in `~/.lunar/control/`. Saved missions, credentials,
history, and logs live in `~/.lunar/recorder/`. Older layouts migrate without
overwriting existing files.

- `LUNAR_HOME` changes the root directory.
- `LUNAR_PROMPT_BUDGET` sets the startup context warning threshold (default 16,000).

Model configuration stays in Lua, not environment variables. See
[Models and subscriptions](models.md) for subscription defaults and local servers,
and [Stack references](stacks.md) for related repositories.
