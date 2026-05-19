# 08 — Incremental refresh: re-parse only changed files

**Type:** AFK
**Label:** ready-for-agent

## Parent

`.planning/proposals/cozodb-migration.md` — Phase 8.

## What to build

After `git pull` (or any external change to the workspace),
`projects query` should re-parse only the files whose content changed and
cascade-delete facts for files that were removed.

Scope:

- On query, walk the workspace and compare each file's
  `(size, mtime, hash)` against `build_meta_files`. Re-parse the
  diff (added, modified) and skip the unchanged majority.
- For removed files: cascade-delete every fact transitively owned by that
  file. Cross-file edges (`edge_calls`, `edge_imports`) that referenced
  the removed file's symbols must also be cleaned up.
- For modified files: delete the file's existing facts before re-emitting,
  to avoid duplicates.
- `build_meta_files` updated atomically with the data writes.

## Acceptance criteria

- [ ] Touching one file in a large workspace re-parses only that file
- [ ] Deleting a file removes its facts and every dependent cross-file edge
- [ ] Modifying a file leaves no stale rows
- [ ] Incremental refresh wall-time scales with the size of the diff, not
      the workspace
- [ ] `cargo test` green; new tests cover add / modify / delete cycles

## Blocked by

- 07-rocksdb-persistence
