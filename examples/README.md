Copy one of these to `~/.lunar/control/init.lua` (or `$LUNAR_HOME/control/init.lua`).

- `xai.lua` — xAI Completions, API key from `XAI_API_KEY`
- `openai.lua` — ChatGPT Plus/Pro, credential from `/login openai`

`defaults` must name both `provider` and `model`. The token never sits in Lua.
