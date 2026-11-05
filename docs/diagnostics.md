# Diagnostics and token usage

## Response diagnostics

`/debug` toggles inline cards showing provider response headers and parsed token
usage for each response, including retries and tool-loop rounds. These cards are
not saved or included in model context.

Start Lunar with `--debug` to record model HTTP traffic in
`~/.lunar/recorder/debug.log`. Treat traffic logs as sensitive.
Use [Request export](request-export.md) to inspect a runnable request without
sending it.

Temporary request failures retry automatically; Esc cancels the wait.
Connection errors name the provider and underlying cause. A stale turn detected
after system sleep stops while preserving partial output.

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

## Subscription limits

With OpenAI subscription sign-in, the footer shows each reported rolling window's
percentage remaining and relative reset time. It refreshes at startup and after
completed turns. Failed refreshes retain the previous snapshot.

For an estimated breakdown of the current instructions and conversation, use
[`/context`](context.md#inspecting-context).
