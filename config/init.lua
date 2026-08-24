return {
  models = {
    grok46 = { id = "grok-4.6", window = 500000 },
  },

  providers = {
    xai = {
      base_url = "https://api.x.ai/v1",
      key_name = "LUNAR_API_KEY",

      -- key_in = "env"  -- default if omitted
      models = {
        "grok46", -- ref: alias → global catalog

        { -- local models
          id = "grok-4.5"
        },
      },
    },
  },

  defaults = {
    provider = "xai",
    model = "grok46", -- alias, else wire id in that provider's list
  },
}
