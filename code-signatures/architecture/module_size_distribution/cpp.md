# Module Size Distribution -- C++

## Overview
Module size distribution measures how symbol definitions are spread across source files in a C++ codebase. Well-balanced files keep compilation times manageable, reduce merge conflicts, and make it straightforward to locate and reason about specific functionality. Imbalanced distributions -- files that are too large or too small -- signal structural problems that compound as the project grows.

## Why It's an Architecture Concern
Oversized C++ files concentrate too many classes, functions, and templates into a single translation unit, leading to long compile times, difficult code reviews, and frequent merge conflicts. Because C++ headers propagate dependencies transitively, a bloated header can trigger cascading recompilation across the entire project. Anemic modules that wrap a single trivial symbol add file system clutter and navigation overhead without providing meaningful organizational benefit.

## Applicability
- **Relevance**: high
- **Languages covered**: `.cpp, .cc, .cxx, .hpp, .hxx, .hh`
- **Frameworks/libraries**: general

---

## Pattern 1: Oversized Module

### Description
File containing 30 or more top-level symbol definitions or exceeding 1000 lines of code, indicating excessive responsibility concentration.

### Bad Code (Anti-pattern)
```cpp
// engine.cpp -- monolithic file mixing rendering, physics, audio, and input
#include "engine.hpp"

class Renderer {
    void init() { /* ... */ }
    void draw() { /* ... */ }
    void shutdown() { /* ... */ }
};

class PhysicsWorld {
    void step(float dt) { /* ... */ }
    void addBody(Body* b) { /* ... */ }
};

void handleKeyPress(int key) { /* ... */ }
void handleMouseMove(int x, int y) { /* ... */ }
void playSound(const std::string& name) { /* ... */ }
void stopSound(const std::string& name) { /* ... */ }

template <typename T>
T clamp(T val, T lo, T hi) { /* ... */ }

// ... 20 more functions and classes covering unrelated subsystems
namespace utils {
    std::string trim(const std::string& s) { /* ... */ }
    std::vector<std::string> split(const std::string& s, char d) { /* ... */ }
}
```

### Good Code (Fix)
```cpp
// renderer.cpp -- focused on rendering
#include "renderer.hpp"

class Renderer {
public:
    void init() { /* ... */ }
    void draw() { /* ... */ }
    void shutdown() { /* ... */ }
};
```

```cpp
// physics.cpp -- focused on physics simulation
#include "physics.hpp"

class PhysicsWorld {
public:
    void step(float dt) { /* ... */ }
    void addBody(Body* b) { /* ... */ }
};
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`, `class_specifier`, `struct_specifier`, `enum_specifier`, `namespace_definition`, `template_declaration`, `type_definition`, `declaration`
- **Detection approach**: Count all top-level symbol definitions (direct children of `translation_unit`). Flag if count >= 30. Also check if total line count >= 1000.
- **S-expression query sketch**:
```scheme
(translation_unit
  [
    (function_definition) @def
    (class_specifier name: (_)) @def
    (struct_specifier name: (_)) @def
    (enum_specifier name: (_)) @def
    (namespace_definition) @def
    (template_declaration) @def
    (type_definition) @def
    (declaration) @def
  ])
```

### Pipeline Mapping
- **Pipeline name**: `module_size_distribution`
- **Pattern name**: `oversized_module`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Monolithic Export Surface

### Description
Module exporting 20 or more symbols, making it a coupling magnet that many other modules depend on, increasing the blast radius of any change.

### Bad Code (Anti-pattern)
```cpp
// core.hpp -- enormous public header
#pragma once

class Logger { /* ... */ };
class Config { /* ... */ };
class EventBus { /* ... */ };
class ThreadPool { /* ... */ };

void initLogging();
void shutdownLogging();
std::string formatMessage(const std::string& msg);
int parseArguments(int argc, char** argv);
bool validatePath(const std::string& path);
void registerCallback(std::function<void()> cb);
template <typename T> T fromString(const std::string& s);
// ... 12 more public declarations spanning unrelated concerns
```

### Good Code (Fix)
```cpp
// logger.hpp -- focused public interface
#pragma once

class Logger {
public:
    void init();
    void shutdown();
    void log(const std::string& msg);
};
```

```cpp
// config.hpp -- focused public interface
#pragma once

class Config {
public:
    void load(const std::string& path);
    std::string get(const std::string& key) const;
};
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`, `class_specifier`, `struct_specifier`, `template_declaration`, `declaration`
- **Detection approach**: Count public, non-static top-level declarations. In headers, count all non-static symbols. In implementation files, count non-static free functions and class definitions with public members. Flag if count >= 20.
- **S-expression query sketch**:
```scheme
(translation_unit
  (function_definition
    declarator: (function_declarator
      declarator: (identifier) @name)) @def)

(class_specifier
  body: (field_declaration_list
    (access_specifier) @access
    (function_definition) @method))
```

### Pipeline Mapping
- **Pipeline name**: `module_size_distribution`
- **Pattern name**: `monolithic_export_surface`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 3: Anemic Module

### Description
File containing only a single symbol definition, creating unnecessary indirection and file system fragmentation without adding organizational value.

### Bad Code (Anti-pattern)
```cpp
// version.cpp
#include "version.hpp"

std::string getVersion() {
    return "1.2.3";
}
```

### Good Code (Fix)
```cpp
// app_info.cpp -- merge the trivial function into a related module
#include "app_info.hpp"

std::string getVersion() {
    return "1.2.3";
}

std::string getBuildDate() {
    return __DATE__;
}

void printUsage(const std::string& program) {
    std::cout << "Usage: " << program << " [options]\n";
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition`, `class_specifier`, `struct_specifier`, `enum_specifier`, `namespace_definition`, `template_declaration`, `type_definition`, `declaration`
- **Detection approach**: Count top-level symbol definitions (direct children of `translation_unit`). Flag if count == 1, excluding entry points (files containing `main`) and test files.
- **S-expression query sketch**:
```scheme
(translation_unit
  [
    (function_definition) @def
    (class_specifier name: (_)) @def
    (struct_specifier name: (_)) @def
    (enum_specifier name: (_)) @def
    (namespace_definition) @def
    (template_declaration) @def
    (type_definition) @def
    (declaration) @def
  ])
```

### Pipeline Mapping
- **Pipeline name**: `module_size_distribution`
- **Pattern name**: `anemic_module`
- **Severity**: info
- **Confidence**: low

---
