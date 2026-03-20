# Circular Dependencies -- C

## Overview
Circular dependencies in C occur when two or more header or source files mutually `#include` each other, creating a cycle in the dependency graph. This leads to compilation failures (incomplete types, redefinition errors) and tightly couples modules that should be independent. Include guards prevent infinite recursion but do not resolve the underlying ordering and coupling problems.

## Why It's an Architecture Concern
Circular `#include` chains make modules inseparable — changing one header forces recompilation of every file in the cycle. They prevent independent testing because neither module can compile without the other. Initialization ordering becomes fragile when global variables or static initializers in coupled translation units depend on each other's definitions. Most critically, cycles indicate tangled responsibilities: if module A needs types from B and B needs types from A, neither has a clear, self-contained purpose, making the codebase harder to reason about, refactor, and maintain.

## Applicability
- **Relevance**: high
- **Languages covered**: `.c, .h`
- **Frameworks/libraries**: general

---

## Pattern 1: Mutual Import

### Description
Two modules directly importing each other, creating a tight bidirectional coupling that prevents either from being understood or modified independently.

### Bad Code (Anti-pattern)
```c
// --- engine.h ---
#ifndef ENGINE_H
#define ENGINE_H

#include "renderer.h"  // engine.h includes renderer.h

typedef struct {
    Renderer *renderer;
    int tick_count;
} Engine;

void engine_update(Engine *engine);
#endif

// --- renderer.h ---
#ifndef RENDERER_H
#define RENDERER_H

#include "engine.h"  // renderer.h includes engine.h -- CIRCULAR

typedef struct {
    Engine *engine;  // needs Engine definition
    int frame_count;
} Renderer;

void renderer_draw(Renderer *renderer);
#endif
```

### Good Code (Fix)
```c
// --- types.h --- (shared types extracted to break the cycle)
#ifndef TYPES_H
#define TYPES_H

typedef struct Engine Engine;
typedef struct Renderer Renderer;

#endif

// --- engine.h ---
#ifndef ENGINE_H
#define ENGINE_H

#include "types.h"

struct Engine {
    Renderer *renderer;
    int tick_count;
};

void engine_update(Engine *engine);
#endif

// --- renderer.h ---
#ifndef RENDERER_H
#define RENDERER_H

#include "types.h"

struct Renderer {
    Engine *engine;
    int frame_count;
};

void renderer_draw(Renderer *renderer);
#endif
```

### Tree-sitter Detection Strategy
- **Target node types**: `preproc_include`
- **Detection approach**: Per-file: extract all `#include` paths from each translation unit. Full cycle detection requires cross-file analysis — build an adjacency list from imports.parquet mapping each file to its included headers, then detect cycles using DFS or Tarjan's algorithm. Per-file proxy: flag files that both include a header and are included by that same header.
- **S-expression query sketch**:
```scheme
(preproc_include
  path: [
    (string_literal) @import_source
    (system_lib_string) @import_source
  ])
```

### Pipeline Mapping
- **Pipeline name**: `circular_dependencies`
- **Pattern name**: `mutual_import`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Hub Module (Bidirectional)

### Description
A module with high fan-in (many dependents) AND high fan-out (many dependencies), acting as a nexus that participates in or enables dependency cycles.

### Bad Code (Anti-pattern)
```c
// --- common.h ---
#ifndef COMMON_H
#define COMMON_H

// High fan-out: includes many other modules
#include "database.h"
#include "network.h"
#include "logging.h"
#include "config.h"
#include "auth.h"
#include "cache.h"
#include "metrics.h"

// Provides utility functions used by all of the above (high fan-in)
int common_init(void);
void common_shutdown(void);
const char *common_get_version(void);
void common_log_event(int code, const char *msg);
int common_validate_input(const char *data, size_t len);
void common_register_handler(void (*handler)(int));
#endif
```

### Good Code (Fix)
```c
// --- version.h --- (focused: no external includes needed)
#ifndef VERSION_H
#define VERSION_H
const char *get_version(void);
#endif

// --- lifecycle.h --- (focused: only needs config)
#ifndef LIFECYCLE_H
#define LIFECYCLE_H
#include "config.h"
int app_init(void);
void app_shutdown(void);
#endif

// --- validation.h --- (focused: standalone)
#ifndef VALIDATION_H
#define VALIDATION_H
#include <stddef.h>
int validate_input(const char *data, size_t len);
#endif
```

### Tree-sitter Detection Strategy
- **Target node types**: `preproc_include`
- **Detection approach**: Per-file: count `#include` directives to estimate fan-out. Cross-file: query imports.parquet to count how many other files include this header (fan-in). Flag files where both fan-in >= 5 and fan-out >= 5.
- **S-expression query sketch**:
```scheme
(preproc_include
  path: [
    (string_literal) @import_source
    (system_lib_string) @import_source
  ])
```

### Pipeline Mapping
- **Pipeline name**: `circular_dependencies`
- **Pattern name**: `hub_module_bidirectional`
- **Severity**: info
- **Confidence**: medium

---
