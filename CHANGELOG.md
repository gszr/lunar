# Lunar Changelog

All notable changes to **Lunar** are documented in this file.

## [0.2.0] - Lunar [UNRELEASED] | 2026-09-29

### 🚀 Added

- **Pi-shaped terminal glass**: Lunar opens directly into a four-band terminal UI with a header, scrollable transcript, growing editor, and two-line footer. The empty transcript uses Lunar's Lua-logo moon splash and disappears after the first prompt.
- **Streaming Chat Completions**: assistant text and reasoning stream into the glass through a reused HTTP agent, with visible working states and cancellable turns.
- **OpenAI Responses support**: models can select the Responses protocol in Lua. Lunar replays converted history, requests reasoning summaries, disables response storage, and sends mission cache and affinity identifiers.
- **Four coding tools**: models can call `read`, `write`, `edit`, and `bash`; tool results return to the model until the turn completes.
- **Parallel tool execution**: independent calls from one assistant turn run concurrently, with a 100-round safety limit and an explicit `continue` path after the limit.
- **Cancellable, isolated shell commands**: `bash` has null stdin, runs in its own Unix session, times out after 60 seconds, and kills its process group when cancelled so nested processes and TUIs cannot take over the glass.
- **Bounded tool output**: each result is capped at 50 KB or 2,000 lines. `read` retains the head and reports the next offset; `bash` retains the tail and stores complete truncated output for seven days under Lunar's recorder directory.
- **Linear missions**: conversations persist as append-only JSONL files named `YYYY-MM-DD-N`. Mission headers retain the working directory and a short local name derived from the first prompt.
- **Mission controls**: `/new`, `/resume`, `/name`, and `/mission` create, find, rename, and inspect missions; `lunar -c` resumes the newest mission for the current directory.
- **Mission selection from the CLI**: `lunar -m` opens the mission log, while `lunar -m <mission>` resumes an exact filename, exact label, or date-filtered result.
- **Searchable mission picker**: `/resume` can search mission IDs, names, working directories, and transcript bodies with a case-insensitive substring.
- **Persistent prompt history**: submitted input is stored in the recorder and can be recalled from the editor.
- **Scrollable full transcript**: PageUp, PageDown, the mouse wheel, and Ctrl+Home/Ctrl+End navigate every painted line while preserving tail-follow behavior during streaming.
- **Transcript surfaces**: user prompts render as muted bars, tool calls as green cards with fixed previews, thinking as a collapsed three-line italic preview, and assistant answers as terminal-friendly Markdown.
- **Markdown rendering**: headings, emphasis, inline and fenced code, links, lists, blockquotes, rules, tables, and image alt text render in the terminal; valid fenced JSON is pretty-printed.
- **Provider response metadata**: every assistant response can show the model name returned by the provider in a dim metadata line.
- **Readline-style editor**: cursor movement, word movement and deletion, line kills, Home/End behavior, Delete, multiline navigation, and a real terminal cursor are supported.
- **Multiline input**: Shift+Enter and Ctrl+J insert hard newlines, pasted multiline text is preserved, and the editor wraps and grows up to eight lines.
- **Slash command completion**: typing `/` opens command suggestions; Tab or arrows cycle matches and Enter accepts them.
- **Built-in help and CLI help**: `/help` lists the complete command surface and `lunar --help` documents launch options.
- **Lua 5.5 configuration**: Lunar embeds vendored Lua and loads a returned table containing model aliases, providers, and optional defaults from `~/.lunar/control/init.lua`.
- **Project configuration overrides**: CWD `.lunar/init.lua` loads after user configuration and replaces matching models and providers while retaining unmatched user entries.
- **Model catalog and picker**: `/model` lists configured providers and ordered model entries, supports local and globally aliased model definitions, and applies a selected model to the running mission.
- **Searchable, scrolling model picker**: large catalogs remain navigable and can be filtered by provider, alias, or wire model ID.
- **Per-model protocol selection**: model definitions select `completions`, `responses`, or catalog-only `messages`; omitted APIs use Completions.
- **Runtime API format switching**: `/format response|chat_completions` overrides the live request format until Lunar exits, without changing Lua configuration or mission history.
- **Per-model thinking levels and preferred default**: each model defines its ordered wire values and fallback default, while `defaults.thinking` can choose a preferred default whenever the selected model supports it. `/thinking` exposes only the model’s values, persists the selected level in the mission, and maps it to each supported protocol.
- **Flexible provider credentials**: providers can read an environment variable, run a shell command, use Lunar-managed authentication, or explicitly send no Authorization header.
- **Dynamic provider URLs**: `base_url_cmd` resolves a provider URL through `sh -c`, allowing credential and environment tooling to choose an endpoint before the TUI opens.
- **Unauthenticated providers**: `key_in = "none"` supports local and private HTTP or HTTPS model servers without an Authorization header.
- **xAI managed authentication**: `/login xai` offers device-code subscription authentication or masked API-key entry, stores credentials in Lunar's recorder, and refreshes OAuth tokens when needed.
- **OpenAI subscription authentication**: `/login openai` runs the ChatGPT Plus/Pro device-code flow and sends Responses requests through the Codex endpoint with the required account and origin headers.
- **Logout controls**: `/logout xai` and `/logout openai` remove the corresponding Lunar-managed credential.
- **Configuration editing and reload**: `/config` opens the user Lua file through `$VISUAL` or `$EDITOR`, then reloads user and project configuration after the editor exits.
- **Visible project context**: CWD `AGENTS.md`, CWD `CONTEXT.md`, and skill summaries are sent as a leading user message instead of a hidden system prompt.
- **Global context fallback and skill merging**: `~/.agents/AGENTS.md` is used when the project has no rules file; global and project skill summaries merge by directory name, with project skills winning conflicts.
- **Prompt snapshots for tool loops**: the context preamble is rebuilt for every user submission and then held stable across that turn's tool rounds.
- **Context inspection**: `/context` reports the files, skills, message classes, counts, and estimated tokens currently visible to the model; `/context raw` opens their complete contents in a pager.
- **Prompt-budget warning**: `LUNAR_PROMPT_BUDGET` warns when instruction files and skill summaries exceed the configured startup budget.
- **Context-window protection**: the footer displays current context occupancy and Lunar refuses a new submission when the latest prompt already fills the model window.
- **Manual context compaction**: `/compact [instructions]` asks the live model for a tool-free summary of older history, retains roughly 20,000 recent estimated tokens, and records an append-only checkpoint without hiding the full transcript.
- **Concurrent asides**: `/btw <prompt>` runs one tool-free question alongside an active turn using a snapshot of the turn's current phase, tools, completed rounds, and partial output; aside content is displayed and persisted but excluded from future model context.
- **Token accounting**: the footer separates cumulative input into uncached, cache-read, and cache-write tokens, shows output totals, and restores mission usage and latest prompt size on resume.
- **OpenAI subscription limits**: the footer can show each rolling usage window's remaining percentage and relative reset time, refreshed at startup and after completed turns.
- **HTTP diagnostics**: `--debug` records model request and response traffic, including HTTP error bodies, in the recorder's debug log.
- **Transient request diagnostics**: `/debug` toggles cards for provider response headers and each response's parsed token usage, including retries and tool-loop rounds, without persisting them or adding them to model context.
- **Request identification**: outbound model requests identify Lunar to providers.
- **Resilient HTTP streaming**: transient POST failures retry up to three times with cancellable exponential backoff; completed Completions streams are briefly drained for usage and then in the background so pooled sockets can be reused.
- **Computer-resume detection**: an active stream interrupted by system sleep is stopped as stale while preserving partial output and returning control to the user.
- **Storage separation and migration**: user-authored Lua lives under `control/`, Lunar-owned missions, auth, history, logs, and tool output live under `recorder/`, and legacy root paths migrate without overwriting existing destinations.
- **Optional Lunar attribution skill**: a bundled, manually installed skill adds Lunar attribution when an agent creates a pull request or issue.
- **Optional standup skill**: a bundled, manually installed skill summarizes recent GitHub activity into a concise standup update.
- **Release automation**: GitHub Actions run checks and build release artifacts, with Homebrew and Cargo installation documented.

### 🔄 Changed

- **Configuration is Lua-only**: model and provider selection moved out of `LUNAR_*` model environment variables and into `init.lua`; only host settings such as `LUNAR_HOME` and `LUNAR_PROMPT_BUDGET` remain environment-driven.
- **Lua uses one returned table**: the initial registrar-style setup was replaced by a single transparent `{ models, providers, defaults }` return value.
- **Completions is the default API**: models that omit `api` now use Chat Completions rather than Responses.
- **Missions replace sessions**: user-facing commands and labels consistently use Lunar's mission vocabulary.
- **Storage paths are ownership-based**: configuration moved to `~/.lunar/control/`, while generated state moved to `~/.lunar/recorder/`.
- **Provider secrets resolve once**: endpoint and credential commands are evaluated before the TUI starts rather than repeatedly during a mission.
- **Command implementations are domain modules**: slash-command behavior was split out of the application entry path while keeping `main` thin.
- **Protocol implementations are adapters**: shared HTTP streaming and retry behavior is separated from Completions and Responses wire formats.
- **TUI internals are split by responsibility**: terminal lifecycle, app state, view rendering, transcript viewport, editor input, turn orchestration, actions, events, Lua loading, missions, and auth were extracted from the original skeleton without expanding the product surface.

### 🐞 Fixed

- **Stream completion no longer waits forever**: a provider `finish_reason` ends the visible turn even if `[DONE]` or EOF is delayed, while trailing usage is still collected.
- **Usage after `finish_reason` is retained**: Lunar waits briefly for the provider's final usage chunk before returning the connection to the pool.
- **OpenAI tool calls use the correct wire shape**: Chat Completions tools interoperate with OpenAI-compatible providers.
- **Responses accounting and call IDs are correct**: converted tool history and provider usage use the Responses protocol's identifiers and token fields.
- **Responses reasoning summaries stay separate**: summary deltas no longer leak into the final answer stream.
- **Reasoning previews are readable**: Markdown syntax is stripped from the compact thinking preview while the full answer keeps Markdown rendering.
- **Aborted tools complete coherently**: cancellation leaves a valid tool result in the transcript instead of a dangling call.
- **Truncated model output cannot execute tools**: `finish_reason = "length"` records calls but does not run potentially incomplete arguments.
- **Tool cards are terminal-safe**: ANSI control sequences and unsafe terminal bytes are removed, and cards fill the available transcript width.
- **Large shell trees stop on cancel**: Esc and timeout terminate the whole process group, not just the immediate child.
- **Nested terminal programs cannot steal input**: shell stdin is null and Unix children are detached from the glass's terminal session.
- **Context remains cache-stable during a turn**: file changes made by tools are not injected until the next user submission.
- **Transcript streaming stays responsive**: completed message paint is cached and only the in-flight tail is rewrapped each frame.
- **Scrolling preserves reading position**: streamed output and notices do not snap a scrolled-up transcript to the tail.
- **Multiline editing follows visual intent**: arrow keys navigate wrapped and hard-newline prompts correctly.
- **Prompt history remains reachable**: Up moves into prior submissions after a unique slash-command match instead of cycling a one-item completion list.
- **Mission recency is accurate**: `/resume` orders missions by modification time so recently active work appears first.
- **Mission usage survives resume**: cumulative totals and current context size are reconstructed from JSONL records.
- **Model picker stays usable in small terminals**: selection remains visible while scrolling a catalog taller than the glass.
- **Interrupted streams are explained clearly**: sleep/resume interruption notices distinguish stale transport from ordinary model errors.
- **Terminal Markdown layout is complete**: block spacing, fenced code, tables, links, and adjacent elements render without collapsed or doubled gaps.
- **Fenced JSON is formatted safely**: valid JSON is indented while invalid blocks remain untouched.
- **Provider configuration errors fail visibly**: Lua syntax/runtime errors, invalid defaults, unsupported live APIs, missing credentials, and failed shell commands open the glass with a notice but prevent sending.
