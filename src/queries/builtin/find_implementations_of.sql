-- find_implementations_of — list every type that implements (or extends)
-- the given interface/class.
--
-- Params:
--   $name — short name (matches symbol.name) of the parent type.

WITH base AS (
    SELECT id AS bid
    FROM symbol
    WHERE name = $name
      AND kind IN ('interface', 'class', 'trait', 'struct')
), derived AS (
    SELECT i.impl_id AS d_id, 'implements' AS rel
    FROM implements i
    JOIN base ON i.interface_id = base.bid
  UNION ALL
    SELECT e.child_id AS d_id, 'extends' AS rel
    FROM extends e
    JOIN base ON e.parent_id = base.bid
)
SELECT s.name,
       s.kind,
       s.file_path,
       sp.start_line,
       d.rel AS relation
FROM derived d
JOIN symbol s ON s.id = d.d_id
JOIN span sp ON sp.entity_id = s.id AND sp.file_path = s.file_path
ORDER BY s.file_path, sp.start_line;
