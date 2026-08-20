

lunar.models {
  grok46 = { id = "grok-4.6", window = 500000 },
}


lunar.providers {
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
}


lunar.defaults {
  provider = "xai",
  model = "grok46",        -- alias, else wire id in that provider's list
}


--
-- lunar.on is not v0. Left here to look at, not to run.
--
-- lunar.on("load", function() end)
-- lunar.on("prompt_send", function() end)
-- lunar.on("completion_received", function() end)
--
-- A "cmd_run" hook that shells out to Runta is not an observer: it *is* bash.
-- Timeout, Esc, cwd, and the allow-gate all have to be re-stated or they drift.
-- Routing every bash call remotely also lies about the workspace (local git vs remote cwd).
--
-- If we ever replace the bash tool, a slot is the honest API:
--
--   lunar.tools.bash = function(cmd)
--     local runtime = os.getenv("LUNAR_RUNTIME") or "demo"
--     return lunar.sh { "runta", "exec", runtime, "--", "sh", "-lc", cmd }
--   end
--
-- The hook-shaped version of the same idea (do not implement this bus for it):
--
--   lunar.on("cmd_run", function(cmd)
--     local runtime = os.getenv("LUNAR_RUNTIME") or "demo"
--     return lunar.sh { "runta", "exec", runtime, "--", "sh", "-lc", cmd }
--   end)
--


