-- import_depth — compute the longest path of file imports leading into
-- each file. A file with depth 0 is imported by nobody.
--
-- Returns rows of (file, depth). Uses a recursive CTE rather than PGQ
-- because PGQ MATCH returns shortest paths by default, while this
-- template needs the LONGEST path length to each file — max-aggregated
-- over all paths reaching that file.

WITH RECURSIVE depth(f, d) AS (
    -- Roots: files with no incoming imports.
    SELECT f.path, 0
    FROM file f
    WHERE NOT EXISTS (
        SELECT 1 FROM imports i WHERE i.imported_id = f.path
    )
  UNION ALL
    -- Extend by following an import edge.
    SELECT i.imported_id, depth.d + 1
    FROM imports i
    JOIN depth ON depth.f = i.importer_file_id
)
SELECT f AS file,
       MAX(d) AS depth
FROM depth
GROUP BY f
ORDER BY depth DESC, file ASC;
