# Cyclomatic Complexity -- C++

## Overview
Cyclomatic complexity measures the number of independent execution paths through a function by counting decision points such as `if`, `else if`, `switch` cases, loops (`for`, `while`, `do-while`), logical operators (`&&`, `||`), ternary expressions (`?:`), and `catch` clauses. High cyclomatic complexity indicates code that is difficult to test exhaustively and prone to latent defects.

## Why It's a Complexity Concern
Each decision point requires its own test case for adequate branch coverage, so high-CC functions impose a heavy testing burden. C++ compounds this with features like exception handling, template instantiation, and operator overloading that can introduce hidden control flow. Elevated cyclomatic complexity is consistently correlated with higher defect density and increased maintenance costs.

## Applicability
- **Relevance**: high
- **Languages covered**: `.cpp`, `.cc`, `.cxx`, `.hpp`, `.hxx`, `.hh`
- **Threshold**: 10

---

## Pattern 1: High Decision Density

### Description
Functions with many if/else branches, switch cases, or compound boolean expressions that create numerous execution paths.

### Bad Code (Anti-pattern)
```cpp
std::string classifyShape(const Shape& shape, const RenderConfig& config)
{
    if (shape.type() == ShapeType::Polygon) {
        if (shape.sides() == 3) {
            if (shape.isEquilateral() || config.forceRegular) {
                return "equilateral_triangle";
            } else if (shape.isIsosceles() && shape.angle(0) < 90) {
                return "acute_isosceles";
            } else if (shape.isIsosceles()) {
                return "obtuse_isosceles";
            } else {
                return "scalene_triangle";
            }
        } else if (shape.sides() == 4) {
            if (shape.isSquare()) {
                return "square";
            } else if (shape.isRectangle() || shape.isParallelogram()) {
                return "rectangle_like";
            } else {
                return "quadrilateral";
            }
        } else if (shape.sides() > 4 && shape.sides() <= 12) {
            return shape.isRegular() ? "regular_polygon" : "irregular_polygon";
        } else {
            return "complex_polygon";
        }
    } else if (shape.type() == ShapeType::Curve) {
        if (shape.isClosed()) {
            if (shape.isCircle()) {
                return "circle";
            } else if (shape.isEllipse()) {
                return "ellipse";
            } else {
                return "closed_curve";
            }
        } else {
            return "open_curve";
        }
    } else if (shape.type() == ShapeType::Composite) {
        try {
            auto parts = shape.decompose();
            return parts.size() > 10 ? "complex_composite" : "simple_composite";
        } catch (const DecomposeError& e) {
            return "undecomposable";
        }
    } else {
        return "unknown";
    }
}
```

### Good Code (Fix)
```cpp
std::string classifyTriangle(const Shape& shape, const RenderConfig& config)
{
    if (shape.isEquilateral() || config.forceRegular)
        return "equilateral_triangle";
    if (shape.isIsosceles())
        return shape.angle(0) < 90 ? "acute_isosceles" : "obtuse_isosceles";
    return "scalene_triangle";
}

std::string classifyPolygon(const Shape& shape, const RenderConfig& config)
{
    switch (shape.sides()) {
    case 3:
        return classifyTriangle(shape, config);
    case 4:
        if (shape.isSquare()) return "square";
        if (shape.isRectangle() || shape.isParallelogram()) return "rectangle_like";
        return "quadrilateral";
    default:
        if (shape.sides() <= 12)
            return shape.isRegular() ? "regular_polygon" : "irregular_polygon";
        return "complex_polygon";
    }
}

std::string classifyCurve(const Shape& shape)
{
    if (!shape.isClosed()) return "open_curve";
    if (shape.isCircle()) return "circle";
    if (shape.isEllipse()) return "ellipse";
    return "closed_curve";
}

std::string classifyComposite(const Shape& shape)
{
    try {
        auto parts = shape.decompose();
        return parts.size() > 10 ? "complex_composite" : "simple_composite";
    } catch (const DecomposeError&) {
        return "undecomposable";
    }
}

std::string classifyShape(const Shape& shape, const RenderConfig& config)
{
    switch (shape.type()) {
    case ShapeType::Polygon:  return classifyPolygon(shape, config);
    case ShapeType::Curve:    return classifyCurve(shape);
    case ShapeType::Composite: return classifyComposite(shape);
    default:                  return "unknown";
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `if_statement`, `else_clause`, `case_statement`, `for_statement`, `for_range_loop`, `while_statement`, `do_statement`, `binary_expression` (with `&&`, `||`), `conditional_expression` (`?:`), `catch_clause`
- **Detection approach**: Count decision points within a function body. Each `if`, `else if`, `case`, `for`, range-based `for`, `while`, `do-while`, `&&`, `||`, `?:`, and `catch` adds 1 to CC. Flag when total exceeds threshold.
- **S-expression query sketch**:
```scheme
;; Find function bodies
(function_definition body: (compound_statement) @fn_body) @fn

;; Count decision points within function bodies
(if_statement) @decision
(case_statement) @decision
(for_statement) @decision
(for_range_loop) @decision
(while_statement) @decision
(do_statement) @decision
(conditional_expression) @decision
(catch_clause) @decision
(binary_expression operator: ["&&" "||"]) @decision
```

### Pipeline Mapping
- **Pipeline name**: `cyclomatic`
- **Pattern name**: `high_cyclomatic_complexity`
- **Severity**: warning
- **Confidence**: high

---

## Pattern 2: Nested Conditional Chains

### Description
Deeply nested if/else or switch statements that compound complexity. C++ templates and RAII can help reduce nesting, but legacy code often exhibits deep conditional hierarchies.

### Bad Code (Anti-pattern)
```cpp
bool loadResource(const std::string& path, ResourceManager& mgr, const Options& opts)
{
    if (!path.empty()) {
        auto file = std::ifstream(path);
        if (file.is_open()) {
            auto header = readHeader(file);
            if (header.isValid()) {
                if (header.version >= MIN_VERSION) {
                    auto data = readData(file, header);
                    if (data) {
                        if (opts.validate && !validateData(*data, opts)) {
                            logError("validation failed");
                            return false;
                        }
                        try {
                            mgr.store(path, std::move(*data));
                            return true;
                        } catch (const StorageException& e) {
                            logError(e.what());
                            return false;
                        }
                    } else {
                        logError("failed to read data");
                        return false;
                    }
                } else {
                    logError("version too old");
                    return false;
                }
            } else {
                logError("invalid header");
                return false;
            }
        } else {
            logError("cannot open file");
            return false;
        }
    } else {
        logError("empty path");
        return false;
    }
}
```

### Good Code (Fix)
```cpp
bool loadResource(const std::string& path, ResourceManager& mgr, const Options& opts)
{
    if (path.empty()) {
        logError("empty path");
        return false;
    }

    auto file = std::ifstream(path);
    if (!file.is_open()) {
        logError("cannot open file");
        return false;
    }

    auto header = readHeader(file);
    if (!header.isValid()) {
        logError("invalid header");
        return false;
    }
    if (header.version < MIN_VERSION) {
        logError("version too old");
        return false;
    }

    auto data = readData(file, header);
    if (!data) {
        logError("failed to read data");
        return false;
    }

    if (opts.validate && !validateData(*data, opts)) {
        logError("validation failed");
        return false;
    }

    return storeResource(path, std::move(*data), mgr);
}

bool storeResource(const std::string& path, ResourceData data, ResourceManager& mgr)
{
    try {
        mgr.store(path, std::move(data));
        return true;
    } catch (const StorageException& e) {
        logError(e.what());
        return false;
    }
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `if_statement` containing nested `if_statement` within its `compound_statement` body
- **Detection approach**: Track nesting depth of conditional statements within a function body. Walk the AST from each `if_statement` and count how many ancestor `if_statement` nodes exist within the same function boundary. Flag when nesting depth exceeds 3 levels.
- **S-expression query sketch**:
```scheme
;; Detect nested if statements (3+ levels)
(if_statement
  consequence: (compound_statement
    (if_statement
      consequence: (compound_statement
        (if_statement) @deeply_nested))))
```

### Pipeline Mapping
- **Pipeline name**: `cyclomatic`
- **Pattern name**: `nested_conditional_chains`
- **Severity**: warning
- **Confidence**: high
