---
name: standup-update
description: >
  Generate a concise standup update from the user's recent work GitHub
  activity, including the previous work period and today. Use when the user
  asks what they worked on, requests a standup update, or invokes /standup.
---

# Standup Update

Generate a concise summary of the user's work activity using the GitHub CLI
(`gh`) or GitHub API.

## Date ranges

Always inspect:

1. The previous work period.
2. Today through the current time.

For the previous work period:

- Tuesday through Friday: inspect yesterday.
- Monday: inspect Friday through Sunday so weekend work is not missed.
- Use the user's local timezone when determining date boundaries.
- Do not duplicate activity between sections.

GitHub activity can establish work already performed today, but not future
plans. Do not invent or infer planned work. If the user provides plans, include
them in the Today section.

## Scope

- Include work repositories only.
- Infer the work GitHub organization from the current repository's remote.
- Exclude personal repositories and unrelated organizations.
- Search across the whole work organization, not only the current repository.

## Activity to inspect

Find the authenticated GitHub user's:

- Authored, updated, or merged pull requests.
- Commits and meaningful updates to existing pull requests.
- Submitted pull request reviews.
- Created or meaningfully updated issues.

Confirm that the user performed the activity during the target period. Do not
include an item solely because another person or automation updated it.

Use PR descriptions, commits, and changed files to understand what was
accomplished rather than repeating titles without context.

## Writing rules

- Produce plain text.
- Keep the update concise.
- Prioritize shipped features, fixes, designs, and other substantial work.
- Group closely related PRs into one bullet.
- Mention reviews and planning work when meaningful.
- Give minor work little detail or omit it when it adds noise.
- Do not mention personal projects.
- Do not describe the collection process.
- Put raw PR or issue links in parentheses whenever possible.
- Do not use Markdown link syntax.
- Omit empty sections unless the user asks for them.
- Do not add a blockers section unless requested.

## Output format

```text
Yesterday

- Completed important work (https://github.com/org/repo/pull/123).
- Improved a related workflow and its cleanup behavior
  (https://github.com/org/repo/pull/124,
  https://github.com/org/repo/pull/125).

Today

- Continued implementation of an important feature
  (https://github.com/org/repo/pull/126).
- Reviewed and approved a significant design
  (https://github.com/org/repo/pull/127).
```

For Monday reports, replace `Yesterday` with `Friday–Sunday`.

Return only the standup update unless the user asks for supporting details.
