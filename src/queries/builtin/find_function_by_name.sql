-- find_function_by_name — locate function/method symbols by exact name
-- (matches symbol.name or symbol.qualified_name).
--
-- Params:
--   $name — short name (e.g. login) or qualified name (e.g. auth::login).

SELECT s.id,
       s.kind,
       s.name,
       s.qualified_name,
       s.file_path,
       sp.start_line,
       s.visibility,
       s.exported
FROM symbol s
JOIN span sp
  ON sp.entity_id = s.id AND sp.file_path = s.file_path
WHERE s.kind IN ('function', 'method')
  AND (s.name = $name OR s.qualified_name = $name)
ORDER BY s.file_path, sp.start_line;
