---
name: lunar-attribution
description: Add Lunar attribution when creating a pull request or issue. Use whenever the user asks to open or create a PR or issue.
---

# Lunar Attribution

When creating a pull request or issue, append a blank line and this footer to the body:

```markdown
🤖 Developed with <model> on [Lunar](https://github.com/gszr/lunar) 🌘
```

`<model>` is the provider-returned model on the dim `model:` line under assistant replies in this conversation, for example `GPT-5.6-Sol`. If that line is not visible, use the live model id from the footer, the name between the provider and the thinking level.

Do not add the footer twice if it is already present.
