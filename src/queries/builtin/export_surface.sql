-- export_surface — list every publicly visible symbol that's actually
-- imported from another file in the workspace. Filters for the names a
-- library presents to its consumers.

SELECT DISTINCT
       s.name,
       s.qualified_name,
       s.kind,
       s.file_path,
       sp.start_line
FROM symbol s
JOIN imports i ON i.imported_id = s.file_path
JOIN span sp ON sp.entity_id = s.id AND sp.file_path = s.file_path
WHERE s.visibility = 'public'
  AND s.exported = true
ORDER BY s.file_path, sp.start_line;
