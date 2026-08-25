# Issue #25: investigating Lunar's "Thinking..." stalls

## Summary

Users currently see only a coarse working state:

- `Thinking...` while a model turn is in flight
- `Running tools...` after the model emitted tool calls

That is simple, but it makes several different situations look identical:

- the provider is slow but healthy
- Lunar is retrying a failed request
- the provider accepted the request and then stopped sending bytes
- a tool is still running
- the model is bouncing through repeated tool rounds

The code already has some protections, but very little runtime visibility. The best path is to add **phase-level runtime status** first, then **idle/stuck detection**, and only then an opt-in **deeper trace/log view** if needed.

## Current behavior in the codebase

### What the UI shows today

`src/view.rs` reduces the active state to two strings:

- `Thinking...`
- `Running tools...`

The decision is based on whether the latest assistant message has pending tool calls. That means request setup, retry backoff, stream wait, tool execution, and tool-loop continuation are all flattened into one of those two labels.

### What Lunar already captures

The runtime is not completely blind:

- `src/complete.rs` streams assistant deltas, optional reasoning text, usage, and tool calls
- `src/turn.rs` caps tool loops at 50 rounds
- `src/tools.rs` gives `bash` a default 60 second timeout
- `Esc` can abort a running turn
- tool results are rendered as cards once they finish

So the issue is less "Lunar has no internal state" and more "the UI does not expose enough of it while a turn is running."

### Where the current design can feel stuck

1. **Hidden retries**
   - `src/complete.rs` retries some failures (`408`, `409`, `429`, `5xx`, connection resets).
   - During backoff, the UI still just says `Thinking...`.

2. **No global or idle timeout on model streaming**
   - the shared HTTP agent uses `timeout_global(None)`
   - connect timeout is set, but once connected, a provider can keep the turn open indefinitely
   - if the server stops sending bytes without closing the stream, Lunar has no visible countdown and no automatic detection

3. **Long-running tools are opaque until completion**
   - `bash` times out eventually, but the user does not see which tool is currently running unless the model explained it in text before the call
   - `read`, `write`, and `edit` are usually quick, but a shell command can sit behind `Running tools...` for a while

4. **Tool-loop progress is invisible**
   - Lunar continues automatically after each tool round
   - the user does not see whether it is on round 1 or round 17 unless they infer it from transcript cards

5. **Reasoning preview does not answer "what is Lunar doing?"**
   - `src/render.rs` shows only a short, ephemeral thinking preview
   - that may describe the model's internal text, but not Lunar's runtime state

## Goals for a fix

Any solution should answer two different user questions:

1. **What is Lunar doing right now?**
2. **Does it still look healthy, or has it likely stalled?**

It should also preserve current project constraints:

- keep the binary simple
- avoid noisy workflow machinery
- do not expose secrets in diagnostics
- avoid depending on raw chain-of-thought for observability

## Options

### Option A: richer working line

Replace the binary `Thinking...` / `Running tools...` state with more precise phases.

Examples:

- `Connecting to provider...`
- `Retrying request (2/3) in 1.0s...`
- `Waiting for model output... 18s`
- `Streaming model response...`
- `Running 2 tools...`
- `Running bash cargo test... 11s`
- `Continuing after tools (round 4)...`

#### Pros

- smallest change to the product shape
- answers the user's question immediately
- no extra panes or commands required
- aligns well with Lunar's minimalist TUI

#### Cons

- still only shows the latest phase
- limited post-mortem/debug value
- may become crowded if too much detail is packed into one line

#### Assessment

This is the highest-value first step.

---

### Option B: runtime event trace in memory

Keep a small ring buffer of structured runtime events and surface it on demand.

Possible events:

- request started
- retry scheduled
- first stream byte received
- reasoning delta received
- tool call parsed
- tool started / finished
- tool round incremented
- idle threshold crossed
- stream ended / failed / aborted

Possible UI surfaces:

- a `/status` command that opens a lightweight panel
- a transcript-style expandable "runtime" card
- a toggle for a second line under the working status

#### Pros

- tells the user exactly what happened recently
- useful for debugging reports
- can power the richer working line too

#### Cons

- more implementation and UI design work
- needs a decision on whether events are persisted in missions
- risks transcript noise if shown too aggressively

#### Assessment

Good medium-term foundation. Best paired with Option A, not used alone.

---

### Option C: idle/stuck detection

Track time since the last meaningful progress event and warn when a turn looks stalled.

Possible thresholds:

- **soft warning** after 15-30s with no provider bytes or tool completion
- **strong warning** after 60-120s
- optional hard timeout later, if configured

Example messages:

- `No provider output for 30s. Still waiting; Esc aborts.`
- `bash cargo test has produced no result for 60s.`

This can be implemented separately for:

- request setup / retry backoff
- model stream idleness
- tool execution idleness

#### Pros

- directly addresses the "is it stuck?" part of the issue
- especially important because model streaming currently has no global timeout
- can remain conservative and non-destructive at first

#### Cons

- false positives are possible for genuinely slow providers or commands
- threshold tuning matters
- a hard timeout could be surprising if enabled by default too early

#### Assessment

Strong second step. Start with warnings, not automatic termination.

---

### Option D: opt-in debug log file

Write runtime events to a local log file, for example under `~/.lunar/logs/`, with secret redaction.

Potential contents:

- phase transitions
- retries and HTTP status classes
- tool start/end + durations
- idle warnings
- abort reasons

#### Pros

- excellent for bug reports
- no transcript clutter
- works even when the interactive UI is gone

#### Cons

- adds file management questions
- must be very careful about redacting secrets and not dumping sensitive prompt data
- does not help much unless the user knows the feature exists

#### Assessment

Useful as an opt-in diagnostic layer, but not the first fix users should rely on.

---

### Option E: expose more chain-of-thought/reasoning

Show more of the streamed reasoning text and treat that as the explanation of what Lunar is doing.

#### Pros

- almost no extra runtime instrumentation needed

#### Cons

- reasoning text is not the same thing as runtime state
- some providers/models do not expose it reliably
- it risks over-coupling the UX to chain-of-thought availability
- it does not explain retries, idle sockets, or tool execution progress

#### Assessment

Not a good primary solution. It is orthogonal at best.

## Recommended direction

### Phase 1: add explicit runtime phases

Recommended as the first implementation slice.

Add a small structured runtime status model in app state and show it in the existing working area.

Suggested phases:

- request start
- retry wait
- waiting for first model bytes
- streaming assistant output
- parsing/running tool calls
- waiting for tool results
- continuing after tool round
- completed / failed / aborted

Useful metadata to display:

- elapsed time in the current phase
- retry attempt count
- tool round count
- active tool name when there is exactly one long-running tool
- count of tools when there are several

This alone would make many "stuck" reports self-explanatory.

### Phase 2: add idle warnings

Track the timestamp of the last progress event and surface a warning when a threshold is crossed.

Recommendation:

- soft warning only at first
- no automatic kill for model turns by default
- reuse `Esc aborts` language so the next action is obvious

This addresses the specific case where a provider keeps the connection open but stops making progress.

### Phase 3: add an opt-in runtime trace

Once phases and idle detection exist, expose the last N runtime events through a command or temporary panel.

That gives maintainers and users a better debugging tool without making the default transcript noisy.

## Proposed scope for a follow-up implementation

A practical follow-up PR could stay small if it does all of this without changing mission persistence yet:

1. add a `RuntimeStatus` / `RuntimeEvent` model in app state
2. emit events from `complete.rs`, `turn.rs`, and `tools.rs`
3. render the latest phase in the working line with elapsed seconds
4. show hidden retry/backoff explicitly
5. warn on stream idle time
6. defer persistent logging and transcript integration

## Open questions

- Should idle warnings become notices, working-line text, or both?
- Should runtime events be persisted to missions, or remain ephemeral?
- Do we want user configuration for thresholds, or fixed defaults first?
- Should tool progress show command text in full, or a shortened/sanitized preview?
- Is a `/status` command enough, or is a passive working-line upgrade sufficient for v1?

## Conclusion

The problem is real, and the current code explains why it feels ambiguous: Lunar knows more than it shows.

The most balanced solution is:

1. **better phase visibility now**
2. **idle/stuck warnings next**
3. **deeper opt-in runtime tracing later**

That keeps Lunar simple while giving users a direct answer to both parts of issue #25: what the agent is doing, and whether it likely stopped making progress.
