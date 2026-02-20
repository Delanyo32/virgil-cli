# Virgil SQL Recipes

Reusable DuckDB queries for `virgil query "<SQL>" --data-dir <DATA> --format json`.

Available tables: `files`, `symbols`, `imports`, `comments`, `errors`.

## File Analysis

### Largest files by line count

```sql
SELECT path, language, line_count, size_bytes
FROM files ORDER BY line_count DESC LIMIT 20
```

### File count per language

```sql
SELECT language, COUNT(*) AS files, SUM(line_count) AS total_lines
FROM files GROUP BY language ORDER BY files DESC
```

### Files per directory (top level)

```sql
SELECT split_part(path, '/', 1) AS dir, COUNT(*) AS files
FROM files GROUP BY dir ORDER BY files DESC
```

### Files with no symbols (config, data, empty)

```sql
SELECT f.path, f.language, f.line_count
FROM files f LEFT JOIN symbols s ON f.path = s.file_path
WHERE s.name IS NULL ORDER BY f.line_count DESC
```

### Largest files without doc comments

```sql
SELECT f.path, f.line_count
FROM files f LEFT JOIN comments c ON f.path = c.file_path AND c.kind = 'doc'
WHERE c.file_path IS NULL
ORDER BY f.line_count DESC LIMIT 20
```

## Symbol Analysis

### Symbol count by kind

```sql
SELECT kind, COUNT(*) AS cnt FROM symbols GROUP BY kind ORDER BY cnt DESC
```

### Largest symbols (most lines of code)

```sql
SELECT name, kind, file_path, (end_line - start_line + 1) AS lines
FROM symbols ORDER BY lines DESC LIMIT 20
```

### Naming convention check (camelCase vs snake_case)

```sql
SELECT
  CASE
    WHEN name ~ '^[a-z]+[A-Z]' THEN 'camelCase'
    WHEN name ~ '^[a-z]+_[a-z]' THEN 'snake_case'
    WHEN name ~ '^[A-Z][a-z]' THEN 'PascalCase'
    WHEN name ~ '^[A-Z_]+$' THEN 'UPPER_SNAKE'
    ELSE 'other'
  END AS convention,
  COUNT(*) AS cnt
FROM symbols GROUP BY convention ORDER BY cnt DESC
```

### Potentially dead exports (exported but never imported)

```sql
SELECT s.name, s.kind, s.file_path
FROM symbols s
LEFT JOIN imports i ON i.imported_name = s.name
WHERE s.is_exported = true AND i.imported_name IS NULL
ORDER BY s.file_path, s.name
```

### Average symbol size by kind

```sql
SELECT kind, ROUND(AVG(end_line - start_line + 1), 1) AS avg_lines, COUNT(*) AS cnt
FROM symbols GROUP BY kind ORDER BY avg_lines DESC
```

### Functions with most lines (complexity indicator)

```sql
SELECT name, file_path, (end_line - start_line + 1) AS lines
FROM symbols WHERE kind IN ('function', 'method', 'arrow_function')
ORDER BY lines DESC LIMIT 20
```

## Dependency Analysis

### Most imported external packages

```sql
SELECT module_specifier, COUNT(*) AS cnt
FROM imports WHERE is_external = true
GROUP BY module_specifier ORDER BY cnt DESC LIMIT 20
```

### Most imported internal modules

```sql
SELECT module_specifier, COUNT(*) AS cnt
FROM imports WHERE is_external = false
GROUP BY module_specifier ORDER BY cnt DESC LIMIT 20
```

### Most imported symbol names

```sql
SELECT imported_name, COUNT(*) AS cnt
FROM imports WHERE imported_name != '*' AND imported_name != 'default'
GROUP BY imported_name ORDER BY cnt DESC LIMIT 20
```

### Orphan files (not imported by anything)

```sql
SELECT f.path FROM files f
LEFT JOIN imports i ON i.module_specifier LIKE '%' || replace(f.path, f.extension, '') || '%'
WHERE i.source_file IS NULL
ORDER BY f.path
```

### Potential circular dependencies

```sql
SELECT a.source_file AS file_a, b.source_file AS file_b
FROM imports a JOIN imports b
ON a.module_specifier LIKE '%' || b.source_file || '%'
AND b.module_specifier LIKE '%' || a.source_file || '%'
WHERE a.source_file < b.source_file
```

### Barrel files (files that mostly re-export)

```sql
SELECT source_file, COUNT(*) AS re_exports
FROM imports WHERE kind = 're_export'
GROUP BY source_file ORDER BY re_exports DESC LIMIT 10
```

### Import kind distribution

```sql
SELECT kind, COUNT(*) AS cnt, is_external
FROM imports GROUP BY kind, is_external ORDER BY cnt DESC
```

### Type-only import usage

```sql
SELECT source_file, COUNT(*) AS type_imports
FROM imports WHERE is_type_only = true
GROUP BY source_file ORDER BY type_imports DESC LIMIT 20
```

## Comment Analysis

### Documentation coverage (files with/without doc comments)

```sql
SELECT
  COUNT(DISTINCT f.path) AS total_files,
  COUNT(DISTINCT c.file_path) AS documented_files,
  ROUND(100.0 * COUNT(DISTINCT c.file_path) / COUNT(DISTINCT f.path), 1) AS pct
FROM files f LEFT JOIN comments c ON f.path = c.file_path AND c.kind = 'doc'
```

### TODO / FIXME / HACK tracker

```sql
SELECT file_path, start_line, text FROM comments
WHERE text LIKE '%TODO%' OR text LIKE '%FIXME%' OR text LIKE '%HACK%'
ORDER BY file_path, start_line
```

### Comment density per file

```sql
SELECT f.path, f.line_count,
  COUNT(c.file_path) AS comment_count,
  ROUND(100.0 * COUNT(c.file_path) / f.line_count, 1) AS comments_per_100_lines
FROM files f LEFT JOIN comments c ON f.path = c.file_path
GROUP BY f.path, f.line_count
ORDER BY comments_per_100_lines DESC LIMIT 20
```

### Undocumented exported symbols

```sql
SELECT s.name, s.kind, s.file_path
FROM symbols s
LEFT JOIN comments c ON c.associated_symbol = s.name AND c.file_path = s.file_path AND c.kind = 'doc'
WHERE s.is_exported = true AND c.associated_symbol IS NULL
ORDER BY s.file_path, s.name
```

### Comment kind distribution

```sql
SELECT kind, COUNT(*) AS cnt FROM comments GROUP BY kind ORDER BY cnt DESC
```

## Cross-Table Analysis

### Composite complexity score (lines + imports + symbols)

```sql
SELECT f.path, f.line_count,
  COUNT(DISTINCT s.name) AS symbols,
  COUNT(DISTINCT i.module_specifier) AS imports,
  f.line_count + COUNT(DISTINCT s.name) * 5 + COUNT(DISTINCT i.module_specifier) * 3 AS complexity_score
FROM files f
LEFT JOIN symbols s ON f.path = s.file_path
LEFT JOIN imports i ON f.path = i.source_file
GROUP BY f.path, f.line_count
ORDER BY complexity_score DESC LIMIT 20
```

### Language-specific patterns (exported ratio per language)

```sql
SELECT f.language,
  COUNT(*) AS total_symbols,
  SUM(CASE WHEN s.is_exported THEN 1 ELSE 0 END) AS exported,
  ROUND(100.0 * SUM(CASE WHEN s.is_exported THEN 1 ELSE 0 END) / COUNT(*), 1) AS export_pct
FROM symbols s JOIN files f ON s.file_path = f.path
GROUP BY f.language ORDER BY total_symbols DESC
```

### Files with highest symbol-to-line ratio (dense files)

```sql
SELECT f.path, f.line_count, COUNT(s.name) AS symbols,
  ROUND(1.0 * COUNT(s.name) / f.line_count, 3) AS symbols_per_line
FROM files f JOIN symbols s ON f.path = s.file_path
GROUP BY f.path, f.line_count
HAVING f.line_count > 10
ORDER BY symbols_per_line DESC LIMIT 20
```

### Parse success rate

```sql
SELECT
  (SELECT COUNT(*) FROM files) AS parsed_ok,
  (SELECT COUNT(*) FROM errors) AS failed,
  ROUND(100.0 * (SELECT COUNT(*) FROM files) /
    ((SELECT COUNT(*) FROM files) + (SELECT COUNT(*) FROM errors)), 1) AS success_pct
```
