# How to work here

Lunar is a small Rust host with a Lua guest. Keep it that way.

## Our goals

- We are a barebones coding harness.
- We aim at transparency context efficiency: no hidden system prompts that waste
tokens and confuse the LLM.

## Before you type

- Read `CONTEXT.md`. Locked decisions stay locked. If something is unclear, ask.
- Look at the code — and at Pi when the behavior is “like Pi” — before asking. If the tree answers it, do not ask.
- Unsettled product shape: ask one question, recommend an answer, wait.
- Once the shape is agreed, or the user says implement, stop asking and ship a small slice. Iterate later.

## Code

- Simplest thing that works. No speculative abstraction, no “for later” types, no unused flags.
- Small modules, small surface. Split a file before it becomes a junk drawer. `main` stays a thin entry.
- Match the code that is already here. Same names, same patterns, same density.
- Precise names from the domain. `/mission` not `/session`. Do not invent vocabulary.
- Do not add workflow to the binary. Do not mix the Lua config path with the env path.

## Scope

- Smallest useful diff for this request. One vertical slice you can run.
- Do not “improve” nearby code, comments, or formatting.
- Do not touch unrelated files. Leave workflow, generated, and personal files alone.

## Check your work

- Investigate yourself. Do not ask the user to run something you can run.
- Prefer a real run over a mock. If it fails, find the root cause.
- Add a test only when it protects a behavior that could break. No tests that only prove a library works.
- When a product decision changes, update `CONTEXT.md` in the same turn. Keep the README short enough to start.

## Git

- Commit only when asked.
- Conventional commits (`feat:`, `fix:`, `docs:`, `refactor:`).
- Separate commits for separate concerns.
- Run `cargo fmt --check` and `cargo clippy` and fix issues before committing.

## Changelog

- You always update the changelog for *user-facing* feature, not internal 
  changes like CI and automation, describing the change in *user-facing* terms, 
  not implementation details

<!-- graft:start -->
## Graft — repo context graph

This repo is indexed in `graft/`: small linked markdown nodes that explain each
system and carry exact file:line spans, kept in sync with the code through git.

For ANY task here — understanding how something works, finding where code lives,
or scoping a change — get context from the graph before grepping or opening
source files. Re-ask freely (it's cheap) and reuse literal identifiers you
already have (symbol, error string, file name) as the query. New to this repo?
Run `graft map` first — a token-budgeted orientation (dir clusters, hubs,
hotspots), no LLM, no key.

- Run `graft ask "<your question>" --source` → ranked nodes with the relevant
  code spans inlined (each hit's ≤8-line crux by default; `--full` for whole
  definitions when the crux isn't enough). Match the tool to the task shape:
  for understanding or editing, the top node IS the answer — cite its
  `covers:` file:line spans and edit straight from `--source`. For
  exhaustive tasks ("every occurrence / every caller of this pattern"), ranked
  results are top-N, not complete — run `graft grep "<literal>"` instead
  (exhaustive over indexed files, grouped by enclosing symbol), falling back
  to raw `grep -rn` only for unindexed files.
- `graft skeleton <file>` → every definition's signature + span, ~10× cheaper
  than reading the file; use it to skim an API surface.
- `graft callers <symbol>` gives precomputed, exact edges — who calls this.
  Add `--direction out` for what it calls, or `--depth N` to walk
  transitively for the full blast radius. For structural questions, skip
  ranking and use this directly.
- Or browse: `graft/INDEX.md` lists every node; follow the links.
- Monorepos and folders of multiple repos rank fairly across sub-projects —
  hits carry `[scope/]` labels naming which one they're from. Narrow with
  `graft ask "<task>" --in <scope>/` once you know where you're working.

If a returned span is truncated ("+N more lines"), open the file at that exact
range before finalizing. Only open source files when a node genuinely lacks a
needed detail, and then at the exact file:line the node points to — never
re-read whole files.

After big code changes, refresh the graph with `graft build` (deterministic,
no API key, $0).
<!-- graft:end -->
