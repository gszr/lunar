# Request export

Use `/request full`, then submit a prompt to see a runnable curl command with
context, mission history, and tool definitions. Nothing is sent or saved.
Use `/request` for just the prompt plus tool definitions, or `/request slim`
for just the prompt without tools. All three preserve live model and API settings.

**Exports include your actual API key or OAuth token. Do not share them.**
Repeat the armed command to cancel. After generating an export, `/request copy`
copies the latest command to the macOS clipboard, including the credential.

## Scope and lifecycle

| Command | Context and history | Tool definitions |
| --- | --- | --- |
| `/request` | Next prompt only | Included |
| `/request full` | Preamble, mission context, and next prompt | Included |
| `/request slim` | Next prompt only | Omitted |

Using another form switches the armed scope. A red warning remains above the
editor until cancellation or successful export. Errors keep the mode armed and
preserve the draft.

The preview is transient: it is not saved to a mission or sent in future model
context, and it disappears on mission changes or exit. Export does not refresh
OAuth tokens; they may have expired. Existing mission affinity is included when
available. Prompts containing `!` shell commands are refused rather than executed.

## Clipboard

`/request copy`, `/request full copy`, and `/request slim copy` all copy the latest
generated command, regardless of its scope. They do not regenerate it or change
the armed mode. Copying is supported only on macOS through `pbcopy`.

The clipboard contains the credential and is not cleared automatically.
