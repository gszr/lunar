-- ChatGPT Plus/Pro via /login openai (device-code OAuth).
-- Copy to ~/.lunar/init.lua (or $LUNAR_HOME/init.lua).
-- Models on this auth must set api = "responses".
-- Omitted base_url is https://chatgpt.com/backend-api.

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
