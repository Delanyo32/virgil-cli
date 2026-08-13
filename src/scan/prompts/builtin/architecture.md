Review this codebase's structure using the dependency graph. Focus on:

- Import cycles: files or modules that import each other, directly or through a
  chain (walk the imports table).
- Layering violations: low-level modules importing high-level ones (infer layers
  from directory structure and import direction).
- God files: files that a large share of the codebase imports AND that import a
  large share of the codebase — both hub and authority.
- Dead exports: exported symbols with no importers anywhere.
- Inheritance tangles: deep or wide extends/implements chains (extends and
  implements tables).

This review lives in the graph tables — query first, read source only to confirm.
Severity: high = cycle or violation that blocks safe refactoring, medium =
structure that will rot, low = tidiness.
