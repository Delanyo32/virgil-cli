# Circular Dependencies -- C++

## Overview
Circular dependencies in C++ occur when two or more headers or translation units mutually `#include` each other, forming a cycle in the include graph. This is especially common with tightly coupled class hierarchies where each class holds pointers or references to the other. While include guards prevent infinite inclusion, they do not resolve incomplete-type errors, link-order issues, or the fundamental coupling problem.

## Why It's an Architecture Concern
Circular `#include` chains make modules inseparable — modifying one class header triggers recompilation of every file in the cycle, dramatically increasing build times in large projects. They prevent independent testing because neither class can be compiled or unit-tested in isolation. Template instantiation and static initialization ordering across coupled translation units become fragile and error-prone. Cycles indicate tangled responsibilities: if class A needs class B and B needs A, their abstractions are not properly separated, making the codebase harder to reason about, refactor, and deploy independently.

## Applicability
- **Relevance**: high
- **Languages covered**: `.cpp, .cc, .cxx, .hpp, .hxx, .hh`
- **Frameworks/libraries**: general

---

## Pattern 1: Mutual Import

### Description
Two modules directly importing each other, creating a tight bidirectional coupling that prevents either from being understood or modified independently.

### Bad Code (Anti-pattern)
```cpp
// --- player.hpp ---
#pragma once
#include "world.hpp"  // player.hpp includes world.hpp

class Player {
public:
    void move(World& world);
    void interact(World& world);
private:
    int x_, y_;
    int health_;
};

// --- world.hpp ---
#pragma once
#include "player.hpp"  // world.hpp includes player.hpp -- CIRCULAR

class World {
public:
    void update();
    void spawn_player(Player& player);
    Player& get_active_player();
private:
    std::vector<Player> players_;
    int width_, height_;
};
```

### Good Code (Fix)
```cpp
// --- player_fwd.hpp --- (forward declaration header)
#pragma once

class Player;

// --- player.hpp ---
#pragma once

class World;  // forward declaration, no include needed

class Player {
public:
    void move(World& world);
    void interact(World& world);
private:
    int x_, y_;
    int health_;
};

// --- world.hpp ---
#pragma once
#include "player_fwd.hpp"  // only forward declaration
#include <vector>

class World {
public:
    void update();
    void spawn_player(Player& player);
private:
    std::vector<Player*> players_;  // pointer avoids needing full definition
    int width_, height_;
};
```

### Tree-sitter Detection Strategy
- **Target node types**: `preproc_include`
- **Detection approach**: Per-file: extract all `#include` paths from each translation unit. Full cycle detection requires cross-file analysis — build an adjacency list from imports.parquet mapping each file to its included headers, then detect cycles using DFS or Tarjan's algorithm. Per-file proxy: flag files that both include a header and are included by that same header. Forward declarations (`class Foo;`) can serve as a signal that a cycle was already broken.
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
```cpp
// --- core.hpp ---
#pragma once

// High fan-out: includes many subsystem headers
#include "physics.hpp"
#include "audio.hpp"
#include "rendering.hpp"
#include "input.hpp"
#include "networking.hpp"
#include "scripting.hpp"
#include "ui.hpp"

// High fan-in: every subsystem above includes core.hpp for these
class Core {
public:
    static Core& instance();
    PhysicsEngine& physics();
    AudioManager& audio();
    Renderer& renderer();
    InputSystem& input();
    NetworkManager& network();
private:
    Core() = default;
};
```

### Good Code (Fix)
```cpp
// --- core_fwd.hpp --- (lightweight forward declarations only)
#pragma once
class PhysicsEngine;
class AudioManager;
class Renderer;

// --- physics_service.hpp --- (focused interface)
#pragma once

class PhysicsEngine {
public:
    virtual ~PhysicsEngine() = default;
    virtual void step(float dt) = 0;
};

// --- audio_service.hpp --- (focused interface)
#pragma once

class AudioManager {
public:
    virtual ~AudioManager() = default;
    virtual void play(int sound_id) = 0;
};

// Subsystems depend on abstract interfaces, not on Core
// Core depends on subsystems — unidirectional
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
