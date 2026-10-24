<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/logos/dark/lunar-dark-theme-with-wordmark-transparent.svg">
    <source media="(prefers-color-scheme: light)" srcset="docs/logos/light/lunar-logo-with-wordmark-transparent.svg">
    <img src="docs/logos/light/lunar-logo-with-wordmark-transparent.svg" alt="Lunar" width="220">
  </picture>
</p>

<p align="center">
  <a href="https://github.com/gszr/lunar/actions/workflows/ci.yml"><img src="https://github.com/gszr/lunar/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/gszr/lunar/releases"><img src="https://img.shields.io/github/v/release/gszr/lunar?sort=semver" alt="Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/gszr/lunar" alt="License"></a>
</p>

<p align="center">A terminal coding harness. You open <code>lunar</code> and code</p>

<p align="center">Inspired by <a href="https://github.com/earendil-works/pi">Pi &lt;3</a>. Configured with Lua. Built for context you can inspect.</p>

<p align="center"><img src="docs/demo.gif" alt="Lunar fixing a failing test: read, edit, bash, confirm" width="760"></p>

## Lunar Agent Harness

Lunar is deliberately small: a Rust host owns the terminal, model transport, and four tools, while a Lua guest supplies model and provider configuration. It does not impose plan modes, sub-agents, approval prompts, or another workflow between you and the model.

Lunar sends your rules and context files as a leading user message that you can inspect with `/context raw`. Conversations are saved locally as missions you can leave and resume.

## Features

### What makes Lunar different

Lunar focuses on control over what the model reads, what it remembers, and how you work.

- **Your instructions, not a built-in workflow.** Rules come from files you control, not a hidden coding-agent system prompt. No enforced plan mode, sub-agents, or approval prompts. Anthropic subscription access has one exception: a required identity message.[^anthropic-identity]
- **Work across repositories without loading them all.** Define your backend, frontend, and other components in a stack, with a path and a short note for each. The model gets a map and can inspect the repositories relevant to your task. Only the map enters context automatically—not the code.
- **See where your context and tokens go.** `/context` shows your instructions, skills, stack references, and conversation breakdown. `/context raw` shows their contents. The footer separates total token use, cache reads and writes, and current context size. `/debug` shows usage for individual responses.
- **Configure your setup with Lua.** Define models, providers, reasoning levels, and related repositories in Lua 5.5. Share configuration through modules and override it per project.
- **Use subscriptions, API keys, or local models.** Sign in with xAI, ChatGPT Plus/Pro, or Claude Pro/Max. Bring API credentials or connect to a compatible local model server instead.
- **Decide when to summarize your conversation.** Lunar never compacts automatically. `/compact` summarizes older history while keeping recent messages. The original transcript remains available to scroll through.

<details>
<summary><strong>All supported features</strong></summary>

### Models and sign-in

- **Cloud and local models.** Stream responses through Chat Completions, OpenAI Responses, or Anthropic Messages. Connect to compatible local servers with no authentication required.
- **Subscription sign-in.** Use `/login` for xAI, ChatGPT Plus/Pro, or Claude Pro/Max. xAI also supports saved API keys. `/logout <provider>` removes a saved credential.
- **Your credential tools.** Read API keys from environment variables or shell commands. Resolve provider URLs with shell commands too, including helpers that need terminal input at startup.
- **Searchable model picker.** `/model` searches by provider, alias, or model ID. Each model can have its own API format and context-window size.
- **Model-specific reasoning.** `/thinking` offers the levels configured for the selected model. Set a preferred default across models, with a fallback for models that do not support it. Your selection is saved with the mission.
- **Temporary API switching.** `/format response|chat_completions` changes the request format until exit, without editing configuration. Subscription authentication restrictions still apply.
- **Subscription limits.** With ChatGPT sign-in, see remaining usage and reset times in the footer.

### Context and skills

- **Rules from files.** Load project `AGENTS.md`, falling back to `~/.agents/AGENTS.md`, plus project `CONTEXT.md`. Lunar sends these as a leading user message.
- **Skills without preloading every instruction.** Load summaries from global and project skill directories. Project skills replace matching global skills; full instructions stay on disk until read.
- **Related repositories.** `stack.components` lists repositories by name, location, and optional notes. Lunar resolves their paths without loading their contents.
- **Inspectable context.** `/context` reports message counts and estimated tokens. `/context raw` includes the instructions, conversation, tool calls, and results in a scrollable viewer.
- **Stable instructions during a task.** Context files are refreshed when you submit a prompt, then held unchanged through that turn's tool calls.
- **Manual compaction.** `/compact [instructions]` summarizes older history and keeps roughly 20,000 recent estimated tokens. The original transcript remains available. Cancelling or failing leaves the previous context active.
- **Context limits.** Get a startup warning when context files exceed `LUNAR_PROMPT_BUDGET`. Lunar refuses a new prompt when the last request already filled the model's context window.
- **Optional bundled skills.** Install the included standup-update and Lunar-attribution skills yourself. Neither is enabled by default.

### Coding tools

- **Read, write, edit, and run commands.** Four tools, with parallel execution of calls from the same response. Tools run without approval prompts; use Lunar only in environments you trust.
- **Stop work with Esc.** Cancel a turn or running command. Shell commands time out after 60 seconds, and cancellation stops their process group. Interactive programs cannot take over terminal input.
- **Manage long output.** Results are limited to 50 KB or 2,000 lines. File reads include the next offset. Long shell output shows its tail and links to the saved full output, with files older than seven days cleaned up at startup.
- **Bounded tool loops.** After 100 rounds, submit `continue` to proceed. Tool calls from a response cut short by its length limit are not executed.

### Conversations, called missions

- **Saved locally.** Each mission is a readable JSONL file: one JSON record per line. Names are generated locally from the first prompt. `/new` starts another mission, `/name` renames it, and `/mission` shows its file path.
- **Find previous work.** `/resume` searches mission IDs, names, directories, and conversation text. Recently active missions appear first.
- **Resume from your shell.** `lunar -c` continues the latest mission for the current directory. `lunar -m` opens the list; add a filename or label to resume one, or a date to filter the list.
- **Keep usage and reasoning settings.** Reopening restores total token usage, the latest prompt size, and the saved thinking level when the selected model still supports it.
- **Side questions stay separate.** Run one `/btw` question at a time alongside an active task. It sees the preceding context plus current tools, elapsed phase time, and partial output, without adding its answer to future model context.

### Working in the terminal

- **Streaming answers.** Markdown, wrapped tables, formatted JSON code blocks, and links with visible destinations. Tool results and reasoning previews are separate, and each response shows the model name reported by the provider.
- **Plain-text replies.** Use `/markdown` or `/md` between turns to toggle formatting off or on. See the original Markdown without changing saved conversations. The setting lasts until you exit.
- **Scroll without losing your place.** Use PageUp/PageDown, the mouse wheel, or Ctrl+Home/End. New output does not pull you back to the bottom while you read earlier messages.
- **Multiline editing.** Familiar word and line editing shortcuts, multiline paste, and persistent prompt history. Shift+Enter or Ctrl+J inserts a newline.
- **Command and path completion.** `/` suggests commands. `@` inserts a file path without attaching its contents. Find files in the current directory or an explicit path, grouped by file type with recent files first. Existing paths are highlighted.
- **Help on demand.** `/help` lists commands; `lunar --help` explains launch options.

### Configuration and diagnostics

- **Shared and project settings.** User Lua configuration supports reusable modules, model aliases, and defaults. Project settings override it. If the current directory has no config, Lunar checks the immediate parent so sibling projects can share one.
- **Edit and reload.** `/config` opens your user config in your editor and reloads settings afterward. Invalid reloads keep the last valid configuration. Startup errors identify the file and show a line number and source line when available.
- **Separate settings and history.** User configuration lives in `~/.lunar/control/`; saved conversations, credentials, and logs live in `~/.lunar/recorder/`. `LUNAR_HOME` changes the root. Older layouts migrate without overwriting existing files.
- **Clear token accounting.** See total input and output, uncached input, cache reads and writes, and the latest request's context usage. These are provider-reported counts, not billing estimates. See [Token usage](#token-usage).
- **Troubleshoot requests.** `/debug` shows response headers and token usage inline, without saving those cards or sending them to the model. `--debug` records model HTTP traffic in a log.
- **Recover from interruptions.** Temporary request failures retry automatically, and Esc cancels the wait. Connection errors name the provider and cause. A stale turn after system sleep stops while preserving partial output.

</details>

[^anthropic-identity]: Anthropic OAuth requests include the required system message: “You are Claude Code, Anthropic's official CLI for Claude.” Lunar does not add its own coding-workflow system prompt.

## Install

```bash
brew install --HEAD gszr/taps/lunar
```

Or with Cargo:

```bash
cargo install --git https://github.com/gszr/lunar
```

## Configure

Configure Lunar with `~/.lunar/control/init.lua`. A project can override model aliases and providers by key, and optionally replace `defaults`, in `.lunar/init.lua`. If it is missing in the current directory, Lunar tries the immediate parent only; relative stack paths resolve from the directory containing the selected `.lunar` directory. `init.lua` may `require` modules from `~/.lunar/control/<name>.lua` or `~/.lunar/control/<name>/init.lua`. Copy an example from [`examples/`](examples/), or from Lunar, `/config` opens the user file with `$VISUAL` or `$EDITOR`, then reloads both files when the editor exits:

```lua
return {
  models = {
    grok46 = { id = "grok-4.6", window = 500000, api = "completions" },
  },

  providers = {
    xai = {
      base_url = "https://api.x.ai/v1",
      key_name = "XAI_API_KEY",
      models = { "grok46" },
    },
  },

  defaults = {
    provider = "xai",
    model = "grok46", -- alias, else wire id in that provider's list
  },
}
```

```bash
export XAI_API_KEY=...
```

`defaults.thinking` is optional. It takes priority over each model's own default when that model supports the value; otherwise Lunar uses the model-specific default.

A provider can use `base_url_cmd = "pass lunar/xai-url"` instead of `base_url`, and `key_cmd = "pass my_key"` instead of `key_name`. Commands run through `sh -c` before the TUI opens, so interactive credential helpers such as GPG pinentry work normally. Lunar trims trailing whitespace from stdout. `base_url_cmd` takes precedence over `base_url`; `key_cmd` takes precedence over `key_name`.

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

For an unauthenticated local server, set `key_in = "none"` and an explicit HTTP or HTTPS `base_url`; Lunar sends no Authorization header:

```lua
ollama = {
  base_url = "http://localhost:11434/v1",
  key_in = "none",
  models = { { id = "qwen3", api = "completions" } },
}
```

[`examples/`](examples/) has ready configs for the subscription cases: `openai.lua` (ChatGPT Plus/Pro, `api = "responses"`) and `anthropic.lua` (Claude Pro/Max, `api = "messages"`).

## Run

```bash
lunar
```

Type a prompt and press Enter. Type `@` to place a file path in the prompt; Lunar shows that the file exists and leaves reading it to the model. Run `/help` to see the available commands. Use `/config` to edit and reload `init.lua`, `lunar -c` to continue the latest mission for the current directory, or `lunar -m` to open the mission log (`lunar -m <mission>` resumes a filename or label). Passing a date such as `lunar -m 2026-08-19` opens that day's mission log.

Lunar includes project `AGENTS.md` (or global `~/.agents/AGENTS.md`), project `CONTEXT.md`, and merged skill summaries from `~/.agents/skills` and `.agents/skills`. Project skills override global skills with the same directory name.

## Token usage

Some of Lunar's goals are transparency and token efficiency. Token accounting can be confusing, and most harnesses do little to explain what their numbers mean. Lunar separates cumulative mission traffic from current context occupancy, and breaks input down into ordinary, cache-read, and cache-write tokens. This makes it clear how much work the model has processed over the mission and how much of its context window the latest request occupies.

```text
↑2.1M (U30k R2.0M W100k) ↓8.5k  ctx 150k/1.0M (15.0%)
```

- `↑` is total input processed across the mission: `U + R + W`
- `U` is ordinary input, neither read from nor written to the prompt cache
- `R` is input read from the prompt cache
- `W` is input written to the prompt cache
- `↓` is generated output across the mission
- `ctx` is the latest request's input relative to the model's context window

These are provider-reported token statistics, not billing estimates. Lunar uses the same accounting for API keys and subscriptions, and omits cache components the provider reports as zero or does not report. Reopening a mission restores the cumulative totals and latest request size from its JSONL transcript; the context-window denominator comes from the current model configuration.

Optional skills live in [`skills/`](skills/) and are not enabled by default.
To enable an optional skill for a project:

```bash
mkdir -p .agents/skills
cp -R /path/to/lunar/skills/lunar-attribution .agents/skills/
cp -R /path/to/lunar/skills/standup-update .agents/skills/
```

## Status

Lunar is early software for macOS and Linux.

## License

[MIT](LICENSE)
