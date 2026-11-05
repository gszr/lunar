# Context and skills

## Instructions from files

Lunar loads `AGENTS.md` from the current working directory, falling back to
`~/.agents/AGENTS.md`. Project rules replace global rules. It also includes the
current directory's `CONTEXT.md` in full.

These files become a leading user message, not a hidden coding-workflow system
prompt. Anthropic OAuth requires one exception: the system identity message
“You are Claude Code, Anthropic's official CLI for Claude.”

A Runtime section identifies Lunar and the selected wire model ID. Context files
are refreshed on each prompt submission, then held unchanged through that turn's
tool loop. The preamble is not saved to the mission.
[Stack references](stacks.md) join this preamble without loading repository files.

## Skills

Lunar merges skill summaries from `~/.agents/skills/*/SKILL.md` and
`.agents/skills/*/SKILL.md`. A project skill replaces a global skill with the same
directory name. Global summaries include absolute paths; project paths stay
relative. Full instructions stay on disk until read.

Optional bundled skills live in [`skills/`](../skills/). None are enabled by
default. To install them for a project, copy their directories into
`.agents/skills/`:

```bash
mkdir -p .agents/skills
cp -R /path/to/lunar/skills/lunar-attribution .agents/skills/
cp -R /path/to/lunar/skills/standup-update .agents/skills/
```

## Inspecting context

`/context` shows the preamble components and estimated-token breakdowns for user
messages, assistant messages, tool calls, and tool results. `/context raw` shows
their full contents. Both reflect the context used for requests, including any
compaction checkpoint.

Use PageUp/PageDown, j/k, the wheel, or Ctrl+Home/End to scroll. Esc or q closes
the viewer. These estimates differ from the provider-reported counts in the
[footer](diagnostics.md#token-usage).

## Compaction and limits

Lunar never compacts automatically. `/compact [instructions]` summarizes older
history using the live model without tools, retaining roughly 20,000 recent
estimated tokens. The original transcript remains visible. Cancelling, failure,
or a truncated summary leaves the previous context active.

A startup warning reports when context files, skill summaries, and stack
references exceed `LUNAR_PROMPT_BUDGET` (default 16,000). Lunar refuses a new prompt
when the last request already filled the model's context window.
