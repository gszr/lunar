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
- Add yourself as coauthor.
- Run `cargo fmt --check` and `cargo clippy` and fix issues before committing.
