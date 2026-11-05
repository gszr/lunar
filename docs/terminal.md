# Working in the terminal

## Replies

- **Streaming answers.** Markdown, wrapped tables, formatted JSON code blocks, and links with visible destinations. Tool results and reasoning previews are separate, and each response shows the model name reported by the provider.
- **Plain-text replies.** Use `/markdown` or `/md` between turns to toggle formatting off or on. See the original Markdown without changing saved conversations. The setting lasts until you exit.
- **Scroll without losing your place.** Use PageUp/PageDown, the mouse wheel, or Ctrl+Home/End. New output does not pull you back to the bottom while you read earlier messages.
- **Multiline editing.** Familiar word and line editing shortcuts, multiline paste, and persistent prompt history. Shift+Enter or Ctrl+J inserts a newline. Ctrl+D quits.
- **Command shortcuts.** Use Command+Shift on macOS or Alt+Shift elsewhere: M selects a model, T selects thinking, O edits config, and C shows context, preserving your draft. Your terminal must forward these modifiers; remove conflicting terminal bindings if needed.
- **Command and path completion.** `/` suggests commands. `@` inserts a file path without attaching its contents. Find files in the current directory or an explicit path, grouped by file type with recent files first. Existing paths are highlighted.
- **Help on demand.** `/help` lists commands; `lunar --help` explains launch options.

## Editing and scrolling

Enter sends. Shift+Enter or Ctrl+J inserts a newline. Ctrl+D quits from any
screen, even with a draft or active turn. Esc aborts a turn or clears the editor.

Word and line editing supports Ctrl+W / Alt+Backspace, Ctrl+U/K, Ctrl+A/E,
Alt+B/F, arrows, and Delete. The editor grows to eight lines and wraps by character.

The mouse wheel always scrolls the transcript, regardless of pointer position.
Shift-drag selects text for copying. PageUp/PageDown moves by almost one pane;
the wheel moves three painted lines per notch. Submission and mission changes
jump to the tail; incoming output does not move a scrolled-up view.

## Completion

`/` opens command completion. Tab or Up/Down cycles; Enter accepts.

`@` opens file-path completion at the start of input or after whitespace. It
inserts a path, not file contents. A plain query searches the working directory;
`~/`, absolute paths, and `../` search elsewhere and keep their prefix.
Matching is a case-insensitive path substring. Files are grouped by extension,
newest first. `.git` and `target` are skipped; hidden files appear only when the
query starts with `.`. Tab or arrows cycle, Enter inserts, and Esc closes.
