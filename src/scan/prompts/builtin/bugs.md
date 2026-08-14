Review this codebase for logic bugs. Focus on:

- Null/None/undefined misuse: values used without the check their type demands.
- Error handling gaps: ignored return values, empty catch blocks, errors that are
  swallowed and leave the program in a half-done state.
- Off-by-one and boundary mistakes in loops, slices, and index arithmetic.
- Mismatched assumptions: a caller passing arguments a callee doesn't expect
  (use the call_edge table to cross-check call sites against definitions).
- Copy-paste slips: near-identical branches where one forgot an edit.

Read the actual source of every function you suspect before reporting. Severity:
high = wrong result or crash on a common path, medium = wrong result on an edge
path, low = latent hazard.
