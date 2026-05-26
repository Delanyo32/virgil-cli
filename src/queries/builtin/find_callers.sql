-- find_callers — list every direct caller of a function/method.
--
-- Params:
--   $name — short name of the callee (matches symbol.name).
--
-- Reads the materialised `call_edge` table (populated at build time —
-- see `from_code_graph::resolve_and_emit_call_edges`). Uses PGQ MATCH
-- against the `codegraph` property graph for the direction-of-edge
-- traversal; the join shape is identical to a plain SQL join but
-- exercises the duckpgq engine path.

SELECT caller,
       caller_file,
       sp.start_line AS caller_line,
       callee,
       call_site_file
FROM GRAPH_TABLE (codegraph
    MATCH (a:symbol)-[e:calls]->(c:symbol)
    WHERE c.name = $name
      AND c.kind IN ('function', 'method')
    COLUMNS (
      a.id          AS caller_id,
      a.name        AS caller,
      a.file_path   AS caller_file,
      c.name        AS callee,
      e.file_path   AS call_site_file
    )
) gt
JOIN span sp
  ON sp.entity_id = gt.caller_id AND sp.file_path = gt.caller_file
ORDER BY caller_file, caller_line;
