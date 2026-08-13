Review this codebase for security problems. Focus on:

- Injection: SQL, shell, or path strings built by concatenating untrusted input.
- Secrets committed to code: API keys, tokens, passwords in source or config.
- Unsafe input handling: missing validation at boundaries (HTTP handlers, CLI args,
  file parsing), unchecked deserialization.
- Dangerous patterns: `eval`-style execution, disabled TLS verification, weak
  hashing for credentials, world-readable file permissions.

Use the call graph: for each risky function you find (exec, query, open, spawn),
query its callers to see whether untrusted data can reach it.

Report only what you can point to in the code. Every finding needs a file, a line,
and one sentence on the attack it enables. Severity: high = exploitable now,
medium = exploitable with preconditions, low = hardening gap.
