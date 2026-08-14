Review this codebase for security problems. Focus on:

- Injection: SQL, shell, or path strings built by concatenating untrusted input.
- Secrets hard-coded in source: API keys, tokens, passwords written as literals.
  Only source files are in the database, so say nothing about `.env`, CI, or any
  other config file.
- Unsafe input handling: missing validation at boundaries (HTTP handlers, CLI args,
  file parsing), unchecked deserialization.
- Dangerous patterns: `eval`-style execution, disabled TLS verification, weak
  hashing for credentials, world-readable file permissions.

Use the call graph: for each risky function you find (exec, query, open, spawn),
query its callers to see whether untrusted data can reach it.

Report only what you can point to in the code. Every finding needs a file, a line,
and one sentence on the attack it enables. Severity: high = exploitable now,
medium = exploitable with preconditions, low = hardening gap.
