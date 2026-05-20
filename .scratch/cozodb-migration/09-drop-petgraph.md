# 09 — Drop petgraph (done)

**Type:** AFK
**Label:** done

## Status

**Shipped** in commit `bbf822d` (feat!: drop petgraph; delete unused
taint/resource/cfg code).

The earlier deprecation in `1120649` was based on a wrong reading of
scope. After we paused and audited what actually used petgraph, it
turned out ~80% of the consumers (taint, resource, cfg, the 9
language CFG builders) were unreachable code: the `taint_paths` and
`unreleased_resources` template handlers were stubs that returned
empty findings. Once those got deleted, only `CodeGraph` was left, and
rewriting it without petgraph was straightforward.

## Final outcome

- 11,838 lines deleted (12 files removed, 22 edited).
- `cargo tree | grep petgraph` returns nothing.
- 230 tests pass.

## What replaced petgraph

`CodeGraph` is now backed by three Vecs:

```rust
pub struct CodeGraph {
    pub nodes: Vec<NodeWeight>,
    pub out_edges: Vec<Vec<(NodeIndex, EdgeWeight)>>,
    pub in_edges: Vec<Vec<(NodeIndex, EdgeWeight)>>,
    // ...interner + lookup maps unchanged
}
pub type NodeIndex = usize;
```

Same operations the old `DiGraph` offered (add_node/add_edge, node
iteration, edge traversal by direction), no transitive dep.

## What we lost

- `taint_paths` and `unreleased_resources` `--template` handlers. They
  were stubs anyway. If/when someone wants intra-function taint or
  lifecycle analysis back, they file a new issue scoped to "design
  Cozo-backed taint analysis from scratch."
- The `--no-cfg` / `--no-resource-graph` / `--symbols-only` skip flags
  on `serve`. They were knobs for analyses that no longer exist.

## Blocked by

N/A — closed.
