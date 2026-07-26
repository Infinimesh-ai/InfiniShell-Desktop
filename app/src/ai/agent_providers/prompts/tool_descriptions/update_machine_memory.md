Replace the complete persistent AI memory document for the current remote SSH machine.

Use this tool only for durable facts that will help in future sessions on this same machine, such as:
- operating system and service layout;
- deployment conventions and non-standard paths;
- stable operational practices;
- machine-specific gotchas that are likely to recur.

Calling rules:
- Always pass the full revised memory document. Each call replaces the previously stored document; it does not append to it.
- Keep the document at or below 16,000 Unicode characters. Longer content will be truncated.
- Never store passwords, tokens, private keys, credentials, or other secrets.
- Do not record one-time commands, transient command output, session logs, or a chronological account of work performed.
- Preserve still-valid prior facts, merge duplicates, and replace stale facts when new evidence disproves them.
