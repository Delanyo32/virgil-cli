# Comment Ratio -- Rust

## Overview
Comment ratio measures the proportion of documentation/comments relative to code in a function or module. Functions with complex logic but no comments are harder to maintain; conversely, over-commented code with trivial comments is noise.

## Why It's a Complexity Concern
Under-documented complex code forces future developers to reverse-engineer intent. Critical algorithms, business rules, and non-obvious logic need comments. The sweet spot is documenting "why" not "what", with complex functions having a higher comment ratio than simple ones.

## Applicability
- **Relevance**: medium
- **Languages covered**: .rs
- **Threshold**: Minimum ~10% comment-to-code ratio for functions with CC > 5

---

## Pattern 1: Complex Function Without Comments

### Description
A function with high cyclomatic/cognitive complexity but zero or near-zero comments, leaving future maintainers to decipher the logic.

### Bad Code (Anti-pattern)
```rust
fn resolve_dependencies(manifest: &Manifest, registry: &Registry) -> Result<Vec<Package>, Error> {
    let mut resolved = Vec::new();
    let mut queue: VecDeque<&str> = manifest.dependencies.keys().map(|s| s.as_str()).collect();
    let mut visited = HashSet::new();

    while let Some(name) = queue.pop_front() {
        if visited.contains(name) {
            continue;
        }
        visited.insert(name.to_string());

        let versions = registry.lookup(name)?;
        let constraint = manifest.dependencies.get(name)
            .or_else(|| manifest.dev_dependencies.get(name));

        let pkg = match constraint {
            Some(c) => versions.iter()
                .filter(|v| c.matches(&v.version))
                .max_by(|a, b| a.version.cmp(&b.version))
                .ok_or_else(|| Error::NoMatch(name.to_string(), c.clone()))?,
            None => versions.last()
                .ok_or_else(|| Error::NotFound(name.to_string()))?,
        };

        for dep in &pkg.dependencies {
            if !visited.contains(dep.as_str()) {
                queue.push_back(dep);
            }
        }

        if resolved.iter().any(|p: &Package| p.name == pkg.name && p.version != pkg.version) {
            return Err(Error::Conflict(pkg.name.clone()));
        }

        resolved.push(pkg.clone());
    }

    Ok(resolved)
}
```

### Good Code (Fix)
```rust
/// Resolves a dependency graph using breadth-first traversal of the registry.
///
/// Returns packages in topological-ish order (dependencies before dependents).
fn resolve_dependencies(manifest: &Manifest, registry: &Registry) -> Result<Vec<Package>, Error> {
    let mut resolved = Vec::new();
    let mut queue: VecDeque<&str> = manifest.dependencies.keys().map(|s| s.as_str()).collect();
    let mut visited = HashSet::new();

    while let Some(name) = queue.pop_front() {
        if visited.contains(name) {
            continue;
        }
        visited.insert(name.to_string());

        let versions = registry.lookup(name)?;

        // Check both regular and dev dependencies for version constraints;
        // unconstrained deps fall through to "latest version" below
        let constraint = manifest.dependencies.get(name)
            .or_else(|| manifest.dev_dependencies.get(name));

        let pkg = match constraint {
            Some(c) => versions.iter()
                .filter(|v| c.matches(&v.version))
                .max_by(|a, b| a.version.cmp(&b.version))
                .ok_or_else(|| Error::NoMatch(name.to_string(), c.clone()))?,
            None => versions.last()
                .ok_or_else(|| Error::NotFound(name.to_string()))?,
        };

        for dep in &pkg.dependencies {
            if !visited.contains(dep.as_str()) {
                queue.push_back(dep);
            }
        }

        // Detect conflicting versions early -- a simple resolver cannot handle
        // two versions of the same crate (unlike Cargo's feature-gated duplicates)
        if resolved.iter().any(|p: &Package| p.name == pkg.name && p.version != pkg.version) {
            return Err(Error::Conflict(pkg.name.clone()));
        }

        resolved.push(pkg.clone());
    }

    Ok(resolved)
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_item` for function bodies; `line_comment` for `//` and `///` and `//!` comments; `block_comment` for `/* */`
- **Detection approach**: Count comment lines and code lines within a function body. Calculate ratio. Flag functions with CC > 5 and comment ratio below threshold. Treat `///` doc comments above the function signature as part of the function's documentation.
- **S-expression query sketch**:
  ```scheme
  ;; Capture function body and any comments within it
  (function_item
    body: (block) @function.body)

  (line_comment) @comment
  (block_comment) @comment
  ```

### Pipeline Mapping
- **Pipeline name**: `comment_ratio`
- **Pattern name**: `undocumented_complex_function`
- **Severity**: info
- **Confidence**: medium

---

## Pattern 2: Trivial Over-Commenting

### Description
Functions with comments that merely restate the code rather than explaining intent, adding noise without value.

### Bad Code (Anti-pattern)
```rust
fn process_buffer(buf: &mut Vec<u8>, offset: usize) -> usize {
    // Get the length
    let len = buf.len();

    // Check if offset is greater than length
    if offset > len {
        // Return 0
        return 0;
    }

    // Drain the buffer from 0 to offset
    buf.drain(0..offset);

    // Calculate remaining
    let remaining = buf.len();

    // Return remaining
    remaining
}
```

### Good Code (Fix)
```rust
fn process_buffer(buf: &mut Vec<u8>, offset: usize) -> usize {
    let len = buf.len();
    if offset > len {
        return 0;
    }

    // Discard already-processed bytes so the next read starts at the right position
    buf.drain(0..offset);
    buf.len()
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `line_comment` adjacent to `let_declaration`, `expression_statement`, `return_expression`, `if_expression`
- **Detection approach**: Compare comment text with the adjacent code statement. Flag comments that are paraphrases of the code (heuristic: comment contains same identifiers as the next statement).
- **S-expression query sketch**:
  ```scheme
  ;; Capture comment immediately followed by a statement
  (block
    (line_comment) @comment
    .
    (_) @next_statement)
  ```

### Pipeline Mapping
- **Pipeline name**: `comment_ratio`
- **Pattern name**: `trivial_over_commenting`
- **Severity**: info
- **Confidence**: low
