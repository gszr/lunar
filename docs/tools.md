# Coding tools

- **Read, write, edit, and run commands.** Four tools, with parallel execution of calls from the same response. Tools run without approval prompts; use Lunar only in environments you trust.
- **Stop work with Esc.** Cancel a turn or running command. Shell commands time out after 60 seconds, and cancellation stops their process group. Interactive programs cannot take over terminal input.
- **Manage long output.** Results are limited to 50 KB or 2,000 lines. File reads include the next offset. Long shell output shows its tail and links to the saved full output, with files older than seven days cleaned up at startup.
- **Bounded tool loops.** After 100 rounds, submit `continue` to proceed. Tool calls from a response cut short by its length limit are not executed.
