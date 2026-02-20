# Virgil Strategic Playbooks

Six step-by-step exploration strategies for common codebase tasks. Each playbook is a numbered command sequence — execute in order, adapting paths and queries to the target codebase.

In all examples, replace `<DATA>` with your `--data-dir` path.

## 1. Understand Codebase Architecture

**Goal:** Build a mental model of the project structure, key modules, and architectural patterns.

1. **Get the big picture:**
   ```bash
   virgil overview --data-dir <DATA> --format json
   ```
   Note the language breakdown, total files/symbols, and directory structure.

2. **Identify hub files** (most imported):
   ```bash
   virgil files --sort dependents --limit 15 --data-dir <DATA> --format json
   ```
   Hub files are central modules that many others depend on.

3. **Outline each hub file:**
   ```bash
   virgil outline <HUB_FILE> --data-dir <DATA> --format json
   ```
   Understand what each hub exports.

4. **Examine the module tree:**
   ```bash
   virgil overview --depth 5 --data-dir <DATA> --format json
   ```
   Deeper tree reveals internal module organization.

5. **Find architectural layers via SQL:**
   ```bash
   virgil query "SELECT
     CASE
       WHEN file_path LIKE '%controller%' OR file_path LIKE '%handler%' THEN 'controller'
       WHEN file_path LIKE '%service%' OR file_path LIKE '%usecase%' THEN 'service'
       WHEN file_path LIKE '%model%' OR file_path LIKE '%entity%' THEN 'model'
       WHEN file_path LIKE '%util%' OR file_path LIKE '%helper%' THEN 'utility'
       ELSE 'other'
     END AS layer,
     COUNT(*) AS files
   FROM files GROUP BY layer ORDER BY files DESC" --data-dir <DATA> --format json
   ```

## 2. Find Where Something Is Defined and Used

**Goal:** Locate a symbol's definition and trace all its usage across the codebase.

1. **Search for the symbol:**
   ```bash
   virgil search <SYMBOL_NAME> --data-dir <DATA> --format json
   ```
   Note the file_path and line number of the definition.

2. **Read the definition:**
   ```bash
   virgil read <FILE_PATH> --start-line <N> --end-line <M> --data-dir <DATA> --root <PROJECT>
   ```

3. **Find all callers** (who imports this symbol):
   ```bash
   virgil callers <SYMBOL_NAME> --data-dir <DATA> --format json
   ```

4. **Find all dependents** of the file containing the symbol:
   ```bash
   virgil dependents <FILE_PATH> --data-dir <DATA> --format json
   ```

5. **Read key callers** to understand usage patterns:
   ```bash
   virgil read <CALLER_FILE> --start-line <N> --end-line <M> --data-dir <DATA> --root <PROJECT>
   ```

## 3. Onboard to a New Codebase

**Goal:** Go from zero knowledge to productive understanding in a systematic way.

1. **Parse the codebase:**
   ```bash
   virgil parse <PROJECT_DIR> --output <DATA>
   ```

2. **Check for parse errors:**
   ```bash
   virgil errors --data-dir <DATA> --format json
   ```

3. **Get the overview:**
   ```bash
   virgil overview --data-dir <DATA> --format json
   ```
   Note: languages used, total files, top-level directory structure.

4. **Find entry points** (most depended-on files):
   ```bash
   virgil files --sort dependents --limit 10 --data-dir <DATA> --format json
   ```

5. **Outline entry points:**
   ```bash
   virgil outline <ENTRY_FILE> --data-dir <DATA> --format json
   ```

6. **Read key files:**
   ```bash
   virgil read <ENTRY_FILE> --data-dir <DATA> --root <PROJECT_DIR>
   ```

7. **Understand the dependency graph:**
   ```bash
   virgil deps <ENTRY_FILE> --data-dir <DATA> --format json
   ```

8. **Check external dependencies:**
   ```bash
   virgil imports --external --limit 50 --data-dir <DATA> --format json
   ```

9. **Read doc comments for orientation:**
   ```bash
   virgil comments --kind doc --limit 30 --data-dir <DATA> --format json
   ```

## 4. Investigate a Bug

**Goal:** Trace a bug from symptom to root cause by following code paths.

1. **Search for related symbols:**
   ```bash
   virgil search <BUG_RELATED_TERM> --data-dir <DATA> --format json
   ```

2. **Read the suspect code:**
   ```bash
   virgil read <FILE> --start-line <N> --end-line <M> --data-dir <DATA> --root <PROJECT>
   ```

3. **Check what the suspect file depends on:**
   ```bash
   virgil deps <FILE> --data-dir <DATA> --format json
   ```

4. **Find who calls the suspect symbol:**
   ```bash
   virgil callers <SYMBOL> --data-dir <DATA> --format json
   ```

5. **Read the callers** to understand how the buggy code is invoked:
   ```bash
   virgil read <CALLER_FILE> --data-dir <DATA> --root <PROJECT>
   ```

6. **Check comments near the bug** for context or known issues:
   ```bash
   virgil comments --file <FILE_PREFIX> --data-dir <DATA> --format json
   ```

7. **Search for TODO/FIXME near the area:**
   ```bash
   virgil query "SELECT file_path, text, start_line FROM comments
   WHERE (text LIKE '%TODO%' OR text LIKE '%FIXME%' OR text LIKE '%HACK%')
   AND file_path = '<FILE>'" --data-dir <DATA> --format json
   ```

## 5. Understand Dependencies

**Goal:** Map the dependency structure — what depends on what, external vs internal, cycles.

1. **Check a specific file's dependencies:**
   ```bash
   virgil deps <FILE> --data-dir <DATA> --format json
   ```

2. **Check who depends on that file:**
   ```bash
   virgil dependents <FILE> --data-dir <DATA> --format json
   ```

3. **List all external dependencies:**
   ```bash
   virgil imports --external --data-dir <DATA> --format json
   ```

4. **List all internal imports:**
   ```bash
   virgil imports --internal --data-dir <DATA> --format json
   ```

5. **Find most-imported external packages:**
   ```bash
   virgil query "SELECT module_specifier, COUNT(*) AS cnt
   FROM imports WHERE is_external = true
   GROUP BY module_specifier ORDER BY cnt DESC LIMIT 20" --data-dir <DATA> --format json
   ```

6. **Find orphan files** (imported by nothing):
   ```bash
   virgil query "SELECT f.path FROM files f
   LEFT JOIN imports i ON i.module_specifier LIKE '%' || f.name || '%'
   WHERE i.source_file IS NULL
   ORDER BY f.path" --data-dir <DATA> --format json
   ```

7. **Detect potential circular dependencies:**
   ```bash
   virgil query "SELECT a.source_file AS file_a, b.source_file AS file_b
   FROM imports a JOIN imports b
   ON a.module_specifier LIKE '%' || b.source_file || '%'
   AND b.module_specifier LIKE '%' || a.source_file || '%'
   WHERE a.source_file < b.source_file" --data-dir <DATA> --format json
   ```

## 6. Map the API Surface

**Goal:** Discover all public-facing types, functions, and interfaces.

1. **Get overview of exported symbols:**
   ```bash
   virgil overview --data-dir <DATA> --format json
   ```

2. **Search for exported functions:**
   ```bash
   virgil search "" --kind function --exported --data-dir <DATA> --format json
   ```

3. **Search for exported classes/interfaces:**
   ```bash
   virgil search "" --kind class --exported --data-dir <DATA> --format json
   virgil search "" --kind interface --exported --data-dir <DATA> --format json
   ```

4. **Find all exported types via SQL:**
   ```bash
   virgil query "SELECT name, kind, file_path, start_line
   FROM symbols WHERE is_exported = true
   AND kind IN ('class', 'interface', 'type_alias', 'enum', 'struct', 'trait')
   ORDER BY kind, name" --data-dir <DATA> --format json
   ```

5. **Find re-exports** (barrel files):
   ```bash
   virgil imports --kind re_export --data-dir <DATA> --format json
   ```

6. **Count exported symbols per file** to find API-dense modules:
   ```bash
   virgil query "SELECT file_path, COUNT(*) AS exports
   FROM symbols WHERE is_exported = true
   GROUP BY file_path ORDER BY exports DESC LIMIT 20" --data-dir <DATA> --format json
   ```

7. **Check documentation coverage** of the API surface:
   ```bash
   virgil query "SELECT s.name, s.kind, s.file_path,
     CASE WHEN c.associated_symbol IS NOT NULL THEN true ELSE false END AS has_docs
   FROM symbols s
   LEFT JOIN comments c ON c.associated_symbol = s.name AND c.file_path = s.file_path AND c.kind = 'doc'
   WHERE s.is_exported = true
   ORDER BY has_docs, s.file_path" --data-dir <DATA> --format json
   ```
