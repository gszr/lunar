# Investigation: making Lunar's `Thinking...` state observable

Issue: #23

## Why this came up

Today Lunar only exposes a very small amount of live turn state:

- `Thinking...` while a model turn is in flight
- `Running tools...` once tool calls have been parsed
- the partial assistant answer, if the provider has started streaming it
- the 3-line thinking preview, if the provider emits reasoning text

That is enough when a turn is quick, but it is not enough to answer the user's real question: **is the agent still working, and on what?**

## What the current code does

### UI surface today

The working row is only shown while `app.cancel.is_some()` is set, and the label is derived from a binary check in `src/view.rs`:

- if the last assistant message has tool calls: `Running tools...`
- otherwise: `Thinking...`

Relevant code:

- `src/view.rs:95-145`
- `src/turn.rs:219-232`

### What can happen under the single `Thinking...` label

Several distinct states are currently collapsed into the same text:

1. sending the HTTP request
2. retry backoff after a transient `429`/`5xx`/connection reset
3. waiting for the first SSE chunk
4. streaming reasoning deltas
5. streaming answer deltas
6. waiting briefly for trailing usage / `[DONE]`
7. continuing to the next assistant round after tools

Relevant code:

- request + retry loop: `src/complete.rs`
- finish handling + tool loop: `src/turn.rs`

### What can happen under `Running tools...`

Tool execution also compresses multiple situations into one label:

- one quick `read`
- several tool calls running in parallel
- one long `bash` command approaching the 60s timeout
- a tool loop where the assistant keeps calling tools for many rounds

Relevant code:

- tool definitions and `bash` timeout: `src/tools.rs`
- parallel tool execution: `src/turn.rs:118-149`

### Why this feels like a "stuck" loop

From the user's perspective, the ambiguous cases are:

- **slow model start**: nothing has streamed yet, so the screen just spins
- **retrying the request**: Lunar may still be healthy, but the UI looks identical
- **provider emits no reasoning**: the thinking preview stays empty, so there is no clue
- **long-running tools**: `Running tools...` does not say which tool is active
- **many tool rounds**: the user does not know whether Lunar is progressing or looping

So the issue is less "the app is definitely hung" and more "the app does not expose enough turn-phase detail to distinguish healthy waiting from a bad loop."

## Options

### Option A — richer live phase text in the working row

Replace the current two-state label with a small explicit turn-phase state machine.

Possible phases:

- `Connecting...`
- `Retrying request (2/3)...`
- `Waiting for first token...`
- `Thinking...`
- `Answering...`
- `Running 2 tools...`
- `Waiting for tool results...`
- `Continuing tool round 3/50...`

Add an elapsed timer, for example:

- `⠋ Waiting for first token... 12s`
- `⠋ Running 3 tools... 41s`

#### Pros

- smallest UX change
- low visual noise
- directly addresses the ambiguity in the current spinner row
- does not require mission format changes

#### Cons

- still only shows one line of status
- does not answer "which tool?" unless extra detail is added
- less useful after the fact because it is ephemeral

#### Implementation sketch

- add a `TurnPhase` enum to app state
- update it from `complete.rs` and `turn.rs`
- include retry count, round count, tool count, and `started_at`
- render the phase instead of deriving text from `tool_calls`

### Option B — show the active tool names and completion counts

Keep Option A's phase row, but make tool execution more specific.

Examples:

- `⠋ Running tools (0/2 done): bash cargo test, read Cargo.toml`
- `⠋ Running tools (2/5 done): edit src/view.rs, bash cargo test, ...`

This needs progress events from tool threads back to the app.

#### Pros

- directly answers "what exactly is the agent doing?"
- especially helpful for slow `bash` commands
- still relatively compact

#### Cons

- more plumbing than Option A
- tool titles may need truncation
- parallel execution means there may be multiple "active" tools at once

#### Implementation sketch

- emit `ToolStarted` / `ToolFinished` events from `run_tools_parallel`
- track active and completed tools in app state
- surface concise titles in the working row

### Option C — append ephemeral status lines/cards to the transcript

Treat turn progress as first-class UI output.

Examples:

- `status: request started`
- `status: retrying after HTTP 429`
- `status: running bash cargo test`
- `status: bash cargo test finished in 18.4s`

These could be ephemeral-only, or persisted to the mission log.

#### Pros

- best observability
- gives a timeline instead of a single snapshot
- useful when users scroll back and ask why a turn took a long time

#### Cons

- noisier UI
- risks cluttering the transcript
- persisting status lines would affect mission format and transcript rendering

#### Implementation sketch

- emit structured status events from request + tool code
- render them as a lightweight system/status style
- keep them non-persistent first; only persist later if users want auditability

### Option D — explicit stuck heuristics and warnings

Instead of only exposing phases, detect suspicious inactivity.

Examples:

- `still waiting for first token after 15s`
- `tool bash cargo test still running after 45s`
- `no stream activity for 30s; Esc aborts`

This is not true deadlock detection; it is a heuristic that tells the user that Lunar is still waiting on something external.

#### Pros

- matches the issue language around "if it's indeed stuck"
- gives confidence without promising perfect detection
- works well combined with Option A or B

#### Cons

- heuristics can false-positive on legitimately slow work
- requires per-phase timestamps
- warning thresholds will need tuning

#### Implementation sketch

- track `last_activity_at`
- define per-phase thresholds
- change the spinner text or show a notice when thresholds are exceeded

### Option E — a power-user `/status` or `/debug` command

Expose current turn internals on demand.

Possible output:

- current phase
- elapsed time
- retry attempt
- tool round
- active tools
- last stream activity time

#### Pros

- minimal always-on UI impact
- good for advanced users
- useful alongside the existing compact interface

#### Cons

- does not help users who do not know the command exists
- slower to discover than visible status text
- not enough on its own for the original complaint

## Recommendation

The best path looks incremental:

### 1. Start with Option A

This is the highest-value, lowest-risk change.

Lunar already has enough internal boundaries to distinguish:

- request/retry
- waiting for first chunk
- streaming model output
- running tools
- continuing tool rounds

Even that alone would make `Thinking...` much less opaque.

### 2. Pair it with the useful slice of Option B

For tools, users specifically want to know *what* is happening. The most useful addition is:

- total tool count
- completed count
- short titles for active tools

This gives a concrete answer when the model is blocked on a long tool.

### 3. Add Option D only after phases exist

Once phases and timestamps are in place, inactivity warnings become straightforward and much more credible.

## Suggested product shape

A compact design that fits Lunar's current UI philosophy:

- normal case:
  - `⠋ Waiting for first token... 3s`
  - `⠋ Thinking... 7s`
  - `⠋ Running tools (1/3): bash cargo test, read Cargo.toml 14s`
- suspiciously long case:
  - `⠋ Running tools (0/1): bash cargo test 48s`
  - second line in gold: `still running; Esc aborts`

This keeps the interface small while still making the turn legible.

## Concrete code implications

A likely implementation would touch:

- `src/app.rs`
  - add live turn status fields
- `src/complete.rs`
  - emit request/retry/stream activity events
- `src/turn.rs`
  - track round count and tool progress
- `src/view.rs`
  - render richer working text instead of the current binary label

No mission-log format change is required for the recommended first slice.

## Bottom line

The issue is real: Lunar currently has only a **binary spinner label** for a process that actually has several distinct phases. The clearest fix is not a large debug mode first; it is to expose the existing phases in the main UI, then add per-tool progress and inactivity warnings as follow-ups.
