Review this codebase for maintainability problems. Focus on:

- Oversized functions: use the span table to find functions spanning hundreds of
  lines, then read them to judge whether they genuinely do too much.
- Deep nesting and complex conditionals that resist understanding.
- Duplication: near-identical functions or blocks that should share one home
  (compare symbols with similar names across files).
- Naming that misleads: names that say one thing while the body does another.
- Dead weight: exported symbols nothing imports (join symbol against imports).

Do not report style preferences. Report only things that would slow down or
mislead the next person editing the file. Severity: high = actively misleading,
medium = costly to work around, low = friction.
