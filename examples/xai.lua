-- Copy to ~/.lunar/control/init.lua (or $LUNAR_HOME/control/init.lua).
-- Then: export XAI_API_KEY=...
--
-- key_cmd / base_url_cmd run through sh -c before the TUI opens.
-- Or use /login xai and set key_in = "auth", auth_provider = "xai".

return {
  models = {
    grok46 = { id = "grok-4.6", window = 500000, api = "completions" },
  },

  providers = {
    xai = {
      base_url = "https://api.x.ai/v1",
      -- base_url_cmd = "pass lunar/xai-url"
      key_name = "XAI_API_KEY",
      -- key_cmd = "pass lunar/xai"
      -- key_in = "auth", auth_provider = "xai",
      thinking = "low",
      models = {
        "grok46",
        { id = "grok-4.5", api = "completions", thinking = "high" },
      },
    },
  },

  defaults = {
    provider = "xai",
    model = "grok46",
  },
}
