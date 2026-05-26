-- find_cycles — surface call-graph cycles.
--
-- Reports each cycle as a (a_name, b_name) pair where a calls b
-- transitively and b calls a transitively. `a_id < b_id` keeps the
-- pair canonical (only one row per cycle pair).
--
-- Uses a recursive CTE over the materialised `call_edge` table rather
-- than a PGQ MATCH because duckpgq currently crashes when GRAPH_TABLE
-- is wrapped in a WITH clause (INTERNAL Error: NULL unique_ptr).
-- Recursive CTE is the standard Datalog-style transitive closure +
-- self-intersect for cycle detection.

WITH RECURSIVE reach(a_id, b_id) AS (
    SELECT caller_id, callee_id FROM call_edge
  UNION
    SELECT r.a_id, ce.callee_id
    FROM reach r
    JOIN call_edge ce ON ce.caller_id = r.b_id
)
SELECT sa.name AS a_name,
       sb.name AS b_name,
       r1.a_id,
       r1.b_id
FROM reach r1
JOIN reach r2 ON r1.a_id = r2.b_id AND r1.b_id = r2.a_id
JOIN symbol sa ON sa.id = r1.a_id
JOIN symbol sb ON sb.id = r1.b_id
WHERE r1.a_id < r1.b_id
ORDER BY a_name, b_name;
