# Models and subscriptions

Lunar supports Chat Completions, OpenAI Responses, and Anthropic Messages.
[Lua configuration](configuration.md) defines custom providers and API-key access.

## Selecting models

`/model` opens the picker. Press `/` to search provider, alias, or wire model ID.
Use arrows or j/k to select, Enter to apply, and left/right to change pages.
Esc clears search before closing the picker.

`/model <id>` sets a wire model ID verbatim on the current provider, even if it
is absent from the catalog. It does not resolve aliases or change credentials,
API format, context window, or thinking settings. A live model is required.

`/models refresh` refreshes discovered catalogs without changing the selected
model. Explicit Lua model lists are unchanged.

## Thinking

`/thinking` opens a picker of the selected model's allowed levels.
`/thinking <level>` applies one directly. The level is saved with the mission.
New missions and model selections return to the applicable default.

A model can declare its ordered levels and default in Lua:

```lua
thinking = { "off", "low", "high", default = "high" },
```

Omitting this field allows only `off`. Optional `defaults.thinking` takes
priority when supported, falling back to the model's own default otherwise.

## API format

Model `api` accepts `"completions"`, `"responses"`, or `"messages"`; omission
means Completions. Each model may also declare its context window with `window`.

`/format response|chat_completions|messages` overrides the live request format
until exit. It survives mission changes, model selection, and config reload.
OpenAI subscription auth requires Responses; Anthropic subscription auth requires
Messages. Model, format, and thinking changes cannot occur during a turn.

## Subscription sign-in

`/login` opens a provider picker. Choose xAI, OpenAI, or Anthropic; xAI also
supports saved API keys. Esc cancels sign-in.

For ChatGPT Plus/Pro, run `/login openai`, then choose a numbered subscription profile’s model with `/model`. No Lua entry is needed. Lunar discovers models at login and startup, and keeps a cached list if refresh fails.

Different OpenAI accounts receive stable names such as `openai-subscription-1` and `openai-subscription-2`; logging in again to the same account updates its existing profile. Select a default in user or project Lua:

```lua
return {
  defaults = {
    subscription = "openai-subscription-1", -- instead of provider
    model = "gpt-5.4", -- a wire model ID from /model
    thinking = "high", -- optional
  },
}
```

`/logout openai-subscription-1` disconnects just that profile; `/logout openai` disconnects all OpenAI profiles. Names are never renumbered. An unavailable subscription default shows a notice so you can recover with `/login` or `/model`.

The same subscription behavior applies to `/login anthropic` and `/login xai`: profiles are named `anthropic-subscription-N` and `xai-subscription-N`, appear in `/model`, and work with `defaults.subscription`. Use `/logout <profile>` for one account or `/logout anthropic|xai|openai` for all accounts of that integration. Lua providers remain available for API keys and custom endpoints. Discovery uses each integration's reported capabilities; xAI currently reports context windows but no thinking levels.

## Local models

For an unauthenticated local server, set `key_in = "none"` and an explicit HTTP or HTTPS `base_url`; Lunar sends no Authorization header:

```lua
ollama = {
  base_url = "http://localhost:11434/v1",
  key_in = "none",
  models = { { id = "qwen3", api = "completions" } },
}
```

[`examples/`](../examples/) has ready configs for the subscription cases: `openai.lua` (ChatGPT Plus/Pro, `api = "responses"`) and `anthropic.lua` (Claude Pro/Max, `api = "messages"`).
