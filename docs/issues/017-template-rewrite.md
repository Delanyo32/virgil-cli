# Rewrite built-in `*.cozoql` templates + Rust handlers against new schema

**Label:** enhancement
**Type:** AFK

## What to build

Per ADR-0004, built-in templates were deleted at issue #1 and stayed dark through the migration. Now that the new schema is fully populated, rebuild the template surface clean-slate against the new relations.

The starting set is the same 7 templates plus 3 Rust handlers that existed pre-migration — but they should be re-authored, not ported. New schema fields (`extends`, `implements`, `references`, `*_attrs`, `field_type`) unlock template ideas the old surface couldn't express; this slice may add new templates that exploit them where the value is clear.

Suggested starting set:
- `find_callers`, `find_callees` (`calls` relation, new shape)
- `find_cycles` (call-graph recursion)
- `find_function_by_name` (uses `symbol.qualified_name`)
- `export_surface` (uses `visibility = "public"` + `imports`)
- `import_depth` (recursive `imports`)
- `unused_symbols` (symbols with no `references` rows)
- New: `find_implementations_of` (uses `implements`)
- New: `find_writers_of` (uses `references` with `ref_kind = "write"`)
- Rust handlers: `complexity_hotspots`, `taint_paths`, `unreleased_resources` rebuilt against new shapes

## Acceptance criteria

- [ ] All built-in template names in `src/queries/builtin/` work against the new schema
- [ ] Rust handlers in `src/queries/rust_templates.rs` rebuilt and passing tests
- [ ] Template integration tests pass against `../virgil-skills/benchmarks/<lang>/` for at least 3 languages per template
- [ ] `cargo run -- projects query <benchmark> --template find_callers --param name=<X>` returns expected results
- [ ] Existing CLI flag surface (`--template`, `--cozoscript`, `--file`) unchanged
- [ ] `cargo test` passes

## Blocked by

- #12, #13, #15, #16
