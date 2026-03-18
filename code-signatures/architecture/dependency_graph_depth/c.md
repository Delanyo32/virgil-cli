# Dependency Graph Depth -- C

## Overview
Dependency graph depth measures how many layers of `#include` directives a translation unit must traverse before all declarations are resolved. In C, deep include chains lengthen compile times, increase the blast radius of header changes, and make it difficult to reason about which declarations are actually needed by a given source file.

## Why It's an Architecture Concern
Deep `#include` chains mean that a change to a single low-level header can trigger recompilation of a large fraction of the project. Each additional layer of indirection makes it harder for developers to trace where a type or function is actually declared. "Umbrella" headers that pull in many other headers create implicit coupling: consumers unknowingly depend on transitive includes, and removing any single sub-header can break downstream code in non-obvious ways. Keeping the include graph shallow and explicit reduces build times, limits change propagation, and improves code comprehensibility.

## Applicability
- **Relevance**: low
- **Languages covered**: `.c, .h`
- **Frameworks/libraries**: general

---

## Pattern 1: Barrel File Re-export

### Description
In C, the barrel file pattern manifests as "umbrella headers" -- a single `.h` file that `#include`s many other headers so that consumers only need one `#include`. While convenient, these headers create hidden coupling and pull in far more declarations than any single consumer needs.

### Bad Code (Anti-pattern)
```c
/* project.h -- umbrella header */
#ifndef PROJECT_H
#define PROJECT_H

#include "project/config.h"
#include "project/logging.h"
#include "project/memory.h"
#include "project/networking.h"
#include "project/parsing.h"
#include "project/rendering.h"
#include "project/storage.h"
#include "project/threading.h"
#include "project/utils.h"

#endif /* PROJECT_H */
```

### Good Code (Fix)
```c
/* consumer.c -- imports only what it needs */
#include "project/networking.h"
#include "project/logging.h"

void send_report(const char *endpoint) {
    log_info("Sending report to %s", endpoint);
    net_post(endpoint, build_payload());
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `preproc_include`
- **Detection approach**: Count `#include` directives in a single header file. Flag if count >= 5 and the file contains no function definitions or variable declarations (pure include aggregator). Note: this is a per-file proxy signal; full analysis requires cross-file dependency graph construction from imports.parquet.
- **S-expression query sketch**:
```scheme
(preproc_include
  path: [
    (string_literal) @include_path
    (system_lib_string) @include_path
  ]) @include_directive
```

### Pipeline Mapping
- **Pipeline name**: `dependency_graph_depth`
- **Pattern name**: `barrel_file_reexport`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: Deep Import Chain

### Description
Files importing from deeply nested module paths (3+ levels of nesting), indicating excessive architectural layering. In C this appears as `#include` directives with long relative or nested directory paths.

### Bad Code (Anti-pattern)
```c
#include "platform/drivers/gpu/vulkan/pipeline.h"
#include "platform/drivers/gpu/vulkan/shader.h"
#include "core/subsystem/rendering/backend/context.h"
#include "core/subsystem/rendering/backend/framebuffer.h"

void init_renderer(void) {
    vk_pipeline_t *pipeline = vk_pipeline_create();
    vk_shader_t *shader = vk_shader_load("main.spv");
    fb_context_t *ctx = fb_context_init();
}
```

### Good Code (Fix)
```c
#include "gpu/pipeline.h"
#include "gpu/shader.h"
#include "rendering/context.h"
#include "rendering/framebuffer.h"

void init_renderer(void) {
    vk_pipeline_t *pipeline = vk_pipeline_create();
    vk_shader_t *shader = vk_shader_load("main.spv");
    fb_context_t *ctx = fb_context_init();
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `preproc_include`, `string_literal`
- **Detection approach**: Parse the include path string and count directory separators (`/`). Flag if depth >= 4. Note: per-file signal only; transitive chain depth requires building the full dependency graph from imports.parquet.
- **S-expression query sketch**:
```scheme
(preproc_include
  path: (string_literal) @include_path)
```

### Pipeline Mapping
- **Pipeline name**: `dependency_graph_depth`
- **Pattern name**: `deep_import_chain`
- **Severity**: info
- **Confidence**: low

---
