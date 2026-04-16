## Session Logging

After every prompt, keep a log of the conversation in a markdown file. The main motivation is to aid in code review and to provide a clear record of the changes made by the AI. BTW, feel free to read this log to keep track of the work done in the current branch.

After completing every task, append an entry to `.claude-sessions/<git-branch-name>.md`:

\```
### <user name>
**Date:** YYYY-MM-DD HH:MM

### Prompt
<the user's full input, verbatim>

### Response
<your full final response to the user, verbatim>

### Stats
- **Tokens Used:** X
- **Time Taken:** Y seconds
- **Lines of Code Generated:** +XX/-XX

---
\```

When making a plan, write the full plan in the log.
When a user asked to undo a change, you can just remove the relevant entry from the log, but make sure to keep the rest of the log intact.

If the file doesn't exist, create it. Do not summarize or edit either block.
The file you create should be staged for commit, but do not commit it yourself.

