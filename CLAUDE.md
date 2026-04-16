## Session Logging

After every prompt, keep a log of the conversation in a markdown file. This will help you keep track of the tasks you've completed and the interactions you've had with the user.

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
When a user asked to undo, redo, or revert a change, log the action and the result in the same format as above.

If the file doesn't exist, create it. Do not summarize or edit either block.
The file you create should be staged for commit, but do not commit it yourself.

