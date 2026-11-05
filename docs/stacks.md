# Stack references

Working on a backend and frontend in separate repositories? Add a `stack` table
alongside `models`, `providers`, and `defaults` in your `init.lua`:

```lua
stack = {
  components = {
    backend = { location = "~/Code/backend", notes = "Admin API and runtime" },
    frontend = { location = "~/Code/frontend", notes = "Web UI" },
  },
},
```

The model sees names, resolved paths, and notes—not repository contents. It can
inspect the components relevant to your task. `/context` shows the references.
Project components replace user components with the same name.

Relative project paths resolve from the directory containing the selected
`.lunar` directory. Relative user paths resolve from `~/.lunar/control/`.
Use `~/` or absolute paths for repositories elsewhere.

Unmatched user components remain available. Absolute paths stay absolute;
`~/` expands from your home directory. Paths do not need to exist at startup.
Stack references load at startup and configuration reload, not on each prompt.

See [Configuration](configuration.md) for config locations and reload behavior,
and [Context and skills](context.md) for what enters the model's context.
