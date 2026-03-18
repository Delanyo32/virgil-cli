# Dependency Graph Depth -- C++

## Overview
Dependency graph depth measures how many layers of `#include` directives and `using` declarations a translation unit must traverse before all declarations are resolved. In C++, deep include chains compound with template instantiation costs, dramatically increasing build times and making it difficult to reason about the true set of dependencies a file carries.

## Why It's an Architecture Concern
Deep dependency chains in C++ amplify the already-expensive compilation model. A change to a header buried four levels deep can trigger cascading recompilation across hundreds of translation units. `using namespace` re-exports make it even harder to trace where a symbol originates, and umbrella headers pull in heavy template-rich code that inflates compile times. Keeping the include graph shallow, using forward declarations aggressively, and avoiding transitive `using namespace` directives reduces build overhead and limits the blast radius of changes.

## Applicability
- **Relevance**: low
- **Languages covered**: `.cpp, .cc, .cxx, .hpp, .hxx, .hh`
- **Frameworks/libraries**: general

---

## Pattern 1: Barrel File Re-export

### Description
In C++, the barrel file pattern appears as umbrella headers that `#include` many sub-headers and optionally re-export entire namespaces with `using namespace`. This creates a single entry point that pulls in a large surface area of declarations, often far more than any individual consumer needs. The combination of transitive includes and namespace re-exports makes dependency boundaries invisible.

### Bad Code (Anti-pattern)
```cpp
// engine.hpp -- umbrella header with namespace re-exports
#ifndef ENGINE_HPP
#define ENGINE_HPP

#include "engine/audio.hpp"
#include "engine/input.hpp"
#include "engine/physics.hpp"
#include "engine/rendering.hpp"
#include "engine/networking.hpp"
#include "engine/scripting.hpp"
#include "engine/ui.hpp"

using namespace engine::audio;
using namespace engine::rendering;
using namespace engine::physics;

#endif // ENGINE_HPP
```

### Good Code (Fix)
```cpp
// game_renderer.cpp -- imports only needed headers
#include "engine/rendering.hpp"
#include "engine/ui.hpp"

void GameRenderer::draw_frame() {
    auto& ctx = engine::rendering::get_context();
    engine::ui::Widget panel("stats");
    ctx.draw(panel.render());
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `preproc_include`, `using_declaration`
- **Detection approach**: Count `#include` directives and `using namespace` declarations in a single header file. Flag as barrel if include count >= 5 or if the file contains 3+ `using namespace` directives with no class/function definitions. Note: this is a per-file proxy signal; full analysis requires cross-file dependency graph construction from imports.parquet.
- **S-expression query sketch**:
```scheme
;; Capture all #include directives
(preproc_include
  path: [
    (string_literal) @include_path
    (system_lib_string) @include_path
  ]) @include_directive

;; Capture using namespace declarations
(using_declaration
  (scoped_identifier) @namespace_path) @using_decl
```

### Pipeline Mapping
- **Pipeline name**: `dependency_graph_depth`
- **Pattern name**: `barrel_file_reexport`
- **Severity**: warning
- **Confidence**: medium

---

## Pattern 2: Deep Import Chain

### Description
Files importing from deeply nested module paths (3+ levels of nesting), indicating excessive architectural layering. In C++ this appears as `#include` paths with many directory levels or deeply qualified `using` declarations.

### Bad Code (Anti-pattern)
```cpp
#include "core/platform/graphics/vulkan/pipeline_state.hpp"
#include "core/platform/graphics/vulkan/descriptor_set.hpp"
#include "core/subsystem/ecs/components/physics/rigid_body.hpp"

using core::platform::graphics::vulkan::PipelineState;
using core::subsystem::ecs::components::physics::RigidBody;

class Renderer {
    PipelineState pipeline_;
    void attach(const RigidBody& body);
};
```

### Good Code (Fix)
```cpp
#include "graphics/pipeline_state.hpp"
#include "graphics/descriptor_set.hpp"
#include "ecs/rigid_body.hpp"

using graphics::PipelineState;
using ecs::RigidBody;

class Renderer {
    PipelineState pipeline_;
    void attach(const RigidBody& body);
};
```

### Tree-sitter Detection Strategy
- **Target node types**: `preproc_include`, `string_literal`, `using_declaration`, `scoped_identifier`
- **Detection approach**: Parse include path strings and count directory separators (`/`). For `using` declarations, count `::` separators in the qualified name. Flag if depth >= 4. Note: per-file signal only; transitive chain depth requires building the full dependency graph from imports.parquet.
- **S-expression query sketch**:
```scheme
;; Capture include paths for depth analysis
(preproc_include
  path: (string_literal) @include_path)

;; Capture using declarations for namespace depth analysis
(using_declaration
  (scoped_identifier) @qualified_name)
```

### Pipeline Mapping
- **Pipeline name**: `dependency_graph_depth`
- **Pattern name**: `deep_import_chain`
- **Severity**: info
- **Confidence**: low

---
