# Missions

- **Saved locally.** Each mission is a readable JSONL file: one JSON record per line. Names are generated locally from the first prompt. `/new` starts another mission, `/name` renames it, and `/mission` shows its file path.
- **Find previous work.** `/resume` searches mission IDs, names, directories, and conversation text. Recently active missions appear first.
- **Resume from your shell.** `lunar -c` continues the latest mission for the current directory. `lunar -m` opens the list; add a filename or label to resume one, or a date to filter the list.
- **Keep usage and reasoning settings.** Reopening restores total token usage, the latest prompt size, and the saved thinking level when the selected model still supports it.
- **Side questions stay separate.** Run one `/btw` question at a time alongside an active task. It sees the preceding context plus current tools, elapsed phase time, and partial output, without adding its answer to future model context.

## Launch options

```bash
lunar -c                  # latest mission for this directory
lunar -m                  # mission log
lunar -m 2026-08-19-1      # exact filename, with or without .jsonl
lunar -m "fix login"       # newest exact label for this directory
lunar -m 2026-08-19        # that day's mission log
```

An exact label takes precedence over a date filter. If nothing matches, Lunar
opens the mission log. `-c` and `-m` are mutually exclusive.

## Side questions

`/btw <prompt>` runs one tool-free question alongside an active turn. The aside
and answer are displayed and saved, but neither enters future model context.
Only one aside can run at a time.
