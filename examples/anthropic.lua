-- Claude Pro/Max via /login anthropic (copy-code OAuth).
-- Copy to ~/.lunar/control/init.lua (or $LUNAR_HOME/control/init.lua).
-- Models on this auth must set api = "messages".
-- Omitted base_url is https://api.anthropic.com.

return {
  providers = {
    anthropic = {
      key_in = "auth",
      auth_provider = "anthropic",
      models = {
        {
          id = "claude-opus-4-6",
          api = "messages",
          thinking = { "off", "low", "high", default = "high" },
        },
      },
    },
  },

  defaults = {
    provider = "anthropic",
    model = "claude-opus-4-6",
  },
}
