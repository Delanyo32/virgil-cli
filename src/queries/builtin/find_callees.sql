-- find_callees — list every direct function/method that `$name` calls.
--
-- Params:
--   $name — short name of the caller.
--
-- Reads the materialised `call_edge` table via PGQ MATCH on the
-- `codegraph` property graph.

SELECT caller,
       callee,
       callee_file,
       sp.start_line AS callee_line
FROM GRAPH_TABLE (codegraph
    MATCH (a:symbol)-[e:calls]->(c:symbol)
    WHERE a.name = $name
      AND c.kind IN ('function', 'method')
    COLUMNS (
      a.name        AS caller,
      c.id          AS callee_id,
      c.name        AS callee,
      c.file_path   AS callee_file
    )
) gt
JOIN span sp
  ON sp.entity_id = gt.callee_id AND sp.file_path = gt.callee_file
ORDER BY callee_file, callee_line;
