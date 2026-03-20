# Comment Ratio -- C++

## Overview
Comment ratio measures the proportion of documentation/comments relative to code in a function or module. Functions with complex logic but no comments are harder to maintain; conversely, over-commented code with trivial comments is noise.

## Why It's a Complexity Concern
Under-documented complex code forces future developers to reverse-engineer intent. Critical algorithms, business rules, and non-obvious logic need comments. The sweet spot is documenting "why" not "what", with complex functions having a higher comment ratio than simple ones.

## Applicability
- **Relevance**: medium
- **Languages covered**: .cpp, .cc, .cxx, .hpp, .hxx, .hh
- **Threshold**: Minimum ~10% comment-to-code ratio for functions with CC > 5

---

## Pattern 1: Complex Function Without Comments

### Description
A function with high cyclomatic/cognitive complexity but zero or near-zero comments, leaving future maintainers to decipher the logic.

### Bad Code (Anti-pattern)
```cpp
std::optional<Route> Router::findRoute(
    const Graph& graph, NodeId src, NodeId dst, const Constraints& constraints)
{
    std::priority_queue<State, std::vector<State>, std::greater<>> pq;
    std::unordered_map<NodeId, double> dist;
    std::unordered_map<NodeId, NodeId> prev;

    pq.push({src, 0.0});
    dist[src] = 0.0;

    while (!pq.empty()) {
        auto [node, cost] = pq.top();
        pq.pop();

        if (node == dst) {
            std::vector<NodeId> path;
            for (auto n = dst; n != src; n = prev[n])
                path.push_back(n);
            path.push_back(src);
            std::reverse(path.begin(), path.end());
            return Route{path, cost};
        }

        if (cost > dist[node])
            continue;

        for (const auto& edge : graph.neighbors(node)) {
            if (constraints.avoid.count(edge.target))
                continue;

            double weight = edge.distance;
            if (constraints.preferHighways && edge.type == EdgeType::Highway)
                weight *= 0.8;
            else if (constraints.avoidTolls && edge.toll > 0)
                weight += edge.toll * constraints.tollPenalty;

            if (constraints.maxElevation > 0 &&
                graph.elevation(edge.target) > constraints.maxElevation)
                continue;

            double newDist = cost + weight;
            if (!dist.count(edge.target) || newDist < dist[edge.target]) {
                dist[edge.target] = newDist;
                prev[edge.target] = node;
                pq.push({edge.target, newDist});
            }
        }
    }
    return std::nullopt;
}
```

### Good Code (Fix)
```cpp
/// Finds the shortest route from src to dst using Dijkstra's algorithm
/// with configurable constraints (avoidance zones, toll penalties, elevation limits).
/// Returns nullopt if no valid route exists.
std::optional<Route> Router::findRoute(
    const Graph& graph, NodeId src, NodeId dst, const Constraints& constraints)
{
    std::priority_queue<State, std::vector<State>, std::greater<>> pq;
    std::unordered_map<NodeId, double> dist;
    std::unordered_map<NodeId, NodeId> prev;

    pq.push({src, 0.0});
    dist[src] = 0.0;

    while (!pq.empty()) {
        auto [node, cost] = pq.top();
        pq.pop();

        if (node == dst) {
            // Reconstruct path by walking the predecessor chain backward
            std::vector<NodeId> path;
            for (auto n = dst; n != src; n = prev[n])
                path.push_back(n);
            path.push_back(src);
            std::reverse(path.begin(), path.end());
            return Route{path, cost};
        }

        // Stale entry -- a shorter path to this node was already processed
        if (cost > dist[node])
            continue;

        for (const auto& edge : graph.neighbors(node)) {
            if (constraints.avoid.count(edge.target))
                continue;

            // Weight adjustments: highways get a 20% bonus to prefer them;
            // tolled roads incur a configurable penalty to discourage them
            double weight = edge.distance;
            if (constraints.preferHighways && edge.type == EdgeType::Highway)
                weight *= 0.8;
            else if (constraints.avoidTolls && edge.toll > 0)
                weight += edge.toll * constraints.tollPenalty;

            // Hard elevation cutoff for vehicles with grade limitations
            if (constraints.maxElevation > 0 &&
                graph.elevation(edge.target) > constraints.maxElevation)
                continue;

            double newDist = cost + weight;
            if (!dist.count(edge.target) || newDist < dist[edge.target]) {
                dist[edge.target] = newDist;
                prev[edge.target] = node;
                pq.push({edge.target, newDist});
            }
        }
    }
    return std::nullopt;
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `function_definition` for function bodies; `comment` for `//`, `/* */`, and `///` (Doxygen) comments
- **Detection approach**: Count comment lines and code lines within a function body. Calculate ratio. Flag functions with CC > 5 and comment ratio below threshold. Consider `///` Doxygen comments above the function signature as part of the function's documentation.
- **S-expression query sketch**:
  ```scheme
  ;; Capture function body and any comments within it
  (function_definition
    body: (compound_statement) @function.body)

  (comment) @comment
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
```cpp
void ConnectionPool::returnConnection(std::shared_ptr<Connection> conn)
{
    // Lock the mutex
    std::lock_guard<std::mutex> lock(mutex_);

    // Check if connection is valid
    if (!conn->isValid()) {
        // Decrement active count
        activeCount_--;
        // Return
        return;
    }

    // Reset the connection
    conn->reset();

    // Push connection to pool
    pool_.push_back(conn);

    // Decrement active count
    activeCount_--;

    // Notify one waiting thread
    cv_.notify_one();
}
```

### Good Code (Fix)
```cpp
void ConnectionPool::returnConnection(std::shared_ptr<Connection> conn)
{
    std::lock_guard<std::mutex> lock(mutex_);

    if (!conn->isValid()) {
        activeCount_--;
        return;
    }

    // Reset clears transaction state and prepared statements so the
    // next borrower gets a clean session
    conn->reset();
    pool_.push_back(conn);
    activeCount_--;

    // Wake one thread blocked in acquire() waiting for a free connection
    cv_.notify_one();
}
```

### Tree-sitter Detection Strategy
- **Target node types**: `comment` adjacent to `expression_statement`, `declaration`, `return_statement`, `if_statement`
- **Detection approach**: Compare comment text with the adjacent code statement. Flag comments that are paraphrases of the code (heuristic: comment contains same identifiers as the next statement).
- **S-expression query sketch**:
  ```scheme
  ;; Capture comment immediately followed by a statement
  (compound_statement
    (comment) @comment
    .
    (_) @next_statement)
  ```

### Pipeline Mapping
- **Pipeline name**: `comment_ratio`
- **Pattern name**: `trivial_over_commenting`
- **Severity**: info
- **Confidence**: low
