---
phase: 01-engine-fixes-architecture-json-expansion
plan: 03
subsystem: audit
tags: [json, architecture, audit-pipeline, c, cpp, csharp, php, static-analysis]

# Dependency graph
requires:
  - phase: 01-engine-fixes-architecture-json-expansion
    plan: 01
    provides: "JSON audit engine wire-up, base architecture pipeline templates (module_size_distribution, circular_dependencies, dependency_depth, api_surface_area)"
provides:
  - 16 per-language JSON architecture pipeline files for C, C++, C#, PHP (4 pipelines x 4 language groups)
  - Language-calibrated thresholds: C/C++ depth gte:4, C#/PHP depth gte:6, PHP api_surface_area gte:15
affects:
  - audit-architecture-coverage
  - plan-04
  - plan-05

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Per-language JSON pipeline files with 'languages' field for language-scoped execution"
    - "Language-calibrated thresholds: C/C++ stricter depth (gte:4), PHP raised export threshold (gte:15)"

key-files:
  created:
    - src/audit/builtin/module_size_distribution_c.json
    - src/audit/builtin/module_size_distribution_cpp.json
    - src/audit/builtin/module_size_distribution_csharp.json
    - src/audit/builtin/module_size_distribution_php.json
    - src/audit/builtin/circular_dependencies_c.json
    - src/audit/builtin/circular_dependencies_cpp.json
    - src/audit/builtin/circular_dependencies_csharp.json
    - src/audit/builtin/circular_dependencies_php.json
    - src/audit/builtin/dependency_graph_depth_c.json
    - src/audit/builtin/dependency_graph_depth_cpp.json
    - src/audit/builtin/dependency_graph_depth_csharp.json
    - src/audit/builtin/dependency_graph_depth_php.json
    - src/audit/builtin/api_surface_area_c.json
    - src/audit/builtin/api_surface_area_cpp.json
    - src/audit/builtin/api_surface_area_csharp.json
    - src/audit/builtin/api_surface_area_php.json
  modified: []

key-decisions:
  - "C/C++ dependency_graph_depth threshold set to gte:4 (header inclusion chains should be shallow per D-04)"
  - "C#/PHP dependency_graph_depth threshold set to gte:6 (assembly nesting and Composer depth are common)"
  - "PHP api_surface_area count threshold raised to gte:15 (PHP exports all top-level declarations by default per D-04)"
  - "C/C++ header files (.h/.hpp) may be flagged as oversized_module — DSL has no extension-based exclusion; accepted as known limitation for Phase 1"

patterns-established:
  - "Language-scoped JSON pipeline: derive from base template, add 'languages' field, suffix pipeline name with language code"
  - "Per-language threshold calibration: languages with looser conventions (PHP default-export) use raised count thresholds"

requirements-completed: [ARCH-06, ARCH-07, ARCH-08, ARCH-09]

# Metrics
duration: 8min
completed: 2026-04-16
---

# Phase 01 Plan 03: C/C++/C#/PHP Architecture JSON Pipelines Summary

**16 per-language JSON architecture pipeline files for C, C++, C#, and PHP with language-calibrated thresholds — completing architecture audit coverage for all 9 language groups**

## Performance

- **Duration:** ~8 min
- **Started:** 2026-04-16T09:00:00Z
- **Completed:** 2026-04-16T09:05:52Z
- **Tasks:** 1
- **Files modified:** 16 created

## Accomplishments
- Created 4 module_size_distribution pipeline files (c/cpp/csharp/php) using gte:30 symbol count threshold with same severity map as base template
- Created 4 circular_dependencies pipeline files (c/cpp/csharp/php) using Tarjan SCC algorithm via imports edge traversal
- Created 4 dependency_graph_depth pipeline files with language-calibrated thresholds: C and C++ use gte:4 (header chains should stay shallow), C# and PHP use gte:6 (assembly nesting and Composer depth are common)
- Created 4 api_surface_area pipeline files: PHP uses raised gte:15 count threshold (because PHP exports all top-level declarations by default), all others use gte:10
- All 2559 lib tests pass with zero failures

## Task Commits

Each task was committed atomically:

1. **Task 1: Create 16 per-language JSON architecture pipeline files for C, C++, C#, PHP** - `9538888` (feat)

**Plan metadata:** (docs commit — see below)

## Files Created/Modified
- `src/audit/builtin/module_size_distribution_c.json` - C module size pipeline (gte:30, includes is_barrel_file exclusion)
- `src/audit/builtin/module_size_distribution_cpp.json` - C++ module size pipeline
- `src/audit/builtin/module_size_distribution_csharp.json` - C# module size pipeline
- `src/audit/builtin/module_size_distribution_php.json` - PHP module size pipeline
- `src/audit/builtin/circular_dependencies_c.json` - C circular include detection
- `src/audit/builtin/circular_dependencies_cpp.json` - C++ circular include detection
- `src/audit/builtin/circular_dependencies_csharp.json` - C# circular using detection
- `src/audit/builtin/circular_dependencies_php.json` - PHP circular use/require detection
- `src/audit/builtin/dependency_graph_depth_c.json` - C include chain depth (gte:4)
- `src/audit/builtin/dependency_graph_depth_cpp.json` - C++ include chain depth (gte:4)
- `src/audit/builtin/dependency_graph_depth_csharp.json` - C# using chain depth (gte:6)
- `src/audit/builtin/dependency_graph_depth_php.json` - PHP use/require chain depth (gte:6)
- `src/audit/builtin/api_surface_area_c.json` - C exported symbol ratio (count gte:10)
- `src/audit/builtin/api_surface_area_cpp.json` - C++ exported symbol ratio (count gte:10)
- `src/audit/builtin/api_surface_area_csharp.json` - C# exported symbol ratio (count gte:10)
- `src/audit/builtin/api_surface_area_php.json` - PHP exported symbol ratio (count gte:15, raised threshold)

## Decisions Made
- C/C++ dependency_graph_depth uses gte:4 threshold — header inclusion chains should be shallow; deeper chains indicate design problems
- C#/PHP dependency_graph_depth uses gte:6 — assembly nesting and Composer dependency structures can legitimately run deeper
- PHP api_surface_area raised to gte:15 — PHP's semantics export all top-level declarations by default, so lower thresholds would produce excessive false positives
- C/C++ header files (.h/.hpp) may trigger oversized_module — the JSON DSL has no extension-based exclusion step; documented as known Phase 1 limitation

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
None - all 16 files created from templates without complications. All tests passed immediately.

## User Setup Required
None - no external service configuration required.

## Known Stubs
None - all pipelines are complete declarative JSON with no placeholder data.

## Next Phase Readiness
- Architecture audit coverage is now complete for all 9 language groups (TypeScript/JS covered in Plan 02, C/C++/C#/PHP covered in this plan, remaining groups from wave 1)
- All 16 new JSON files are embedded via include_dir! at compile time (no runtime loading changes needed)
- Plan 04 and Plan 05 can proceed independently

---
*Phase: 01-engine-fixes-architecture-json-expansion*
*Completed: 2026-04-16*
