use std::collections::HashMap;

use crate::cli::QueryOutputFormat;
use crate::query_engine::{QueryOutput, QueryResult};

pub fn format_results(
    output: &QueryOutput,
    format: &QueryOutputFormat,
    pretty: bool,
    project_name: &str,
    query_ms: u64,
) -> String {
    // Read mode: return file content directly (not wrapped in JSON)
    if let Some(ref read) = output.read {
        return format_read(read, pretty, project_name, query_ms);
    }

    let wrapper = build_wrapper(output, project_name, query_ms, format);

    if pretty {
        serde_json::to_string_pretty(&wrapper).unwrap_or_else(|_| "{}".to_string())
    } else {
        serde_json::to_string(&wrapper).unwrap_or_else(|_| "{}".to_string())
    }
}

fn format_read(
    read: &crate::query_engine::ReadResult,
    pretty: bool,
    project_name: &str,
    query_ms: u64,
) -> String {
    let wrapper = serde_json::json!({
        "project": project_name,
        "query_ms": query_ms,
        "file": read.file,
        "start_line": read.start_line,
        "end_line": read.end_line,
        "total_lines": read.total_lines,
        "content": read.content,
    });

    if pretty {
        serde_json::to_string_pretty(&wrapper).unwrap_or_else(|_| "{}".to_string())
    } else {
        serde_json::to_string(&wrapper).unwrap_or_else(|_| "{}".to_string())
    }
}

fn build_wrapper(
    output: &QueryOutput,
    project_name: &str,
    query_ms: u64,
    format: &QueryOutputFormat,
) -> serde_json::Value {
    let results_json = match format {
        QueryOutputFormat::Outline => format_outline(&output.results),
        QueryOutputFormat::Snippet => format_snippet(&output.results),
        QueryOutputFormat::Full => format_full(&output.results),
        QueryOutputFormat::Tree => format_tree(&output.results),
        QueryOutputFormat::Locations => format_locations(&output.results),
        QueryOutputFormat::Summary => format_summary(&output.results, output.files_parsed),
    };

    serde_json::json!({
        "project": project_name,
        "query_ms": query_ms,
        "files_parsed": output.files_parsed,
        "total": output.total,
        "results": results_json,
    })
}

fn format_outline(results: &[QueryResult]) -> serde_json::Value {
    let items: Vec<serde_json::Value> = results
        .iter()
        .map(|r| {
            let mut obj = serde_json::json!({
                "name": r.name,
                "kind": r.kind,
                "file": r.file,
                "line": r.line,
                "exported": r.exported,
            });
            if let Some(ref sig) = r.signature {
                obj["signature"] = serde_json::Value::String(sig.clone());
            }
            if let Some(ref parent) = r.parent {
                obj["parent"] = serde_json::Value::String(parent.clone());
            }
            obj
        })
        .collect();
    serde_json::Value::Array(items)
}

fn format_snippet(results: &[QueryResult]) -> serde_json::Value {
    let items: Vec<serde_json::Value> = results
        .iter()
        .map(|r| {
            let mut obj = serde_json::json!({
                "name": r.name,
                "kind": r.kind,
                "file": r.file,
                "line": r.line,
                "end_line": r.end_line,
                "exported": r.exported,
            });
            if let Some(ref sig) = r.signature {
                obj["signature"] = serde_json::Value::String(sig.clone());
            }
            if let Some(ref preview) = r.preview {
                obj["preview"] = serde_json::Value::String(preview.clone());
            }
            if let Some(ref docstring) = r.docstring {
                obj["docstring"] = serde_json::Value::String(docstring.clone());
            }
            if let Some(ref parent) = r.parent {
                obj["parent"] = serde_json::Value::String(parent.clone());
            }
            obj
        })
        .collect();
    serde_json::Value::Array(items)
}

fn format_full(results: &[QueryResult]) -> serde_json::Value {
    let items: Vec<serde_json::Value> = results
        .iter()
        .map(|r| {
            let mut obj = serde_json::json!({
                "name": r.name,
                "kind": r.kind,
                "file": r.file,
                "line": r.line,
                "end_line": r.end_line,
                "column": r.column,
                "exported": r.exported,
            });
            if let Some(ref sig) = r.signature {
                obj["signature"] = serde_json::Value::String(sig.clone());
            }
            if let Some(ref docstring) = r.docstring {
                obj["docstring"] = serde_json::Value::String(docstring.clone());
            }
            if let Some(ref body) = r.body {
                obj["body"] = serde_json::Value::String(body.clone());
            }
            if let Some(ref parent) = r.parent {
                obj["parent"] = serde_json::Value::String(parent.clone());
            }
            obj
        })
        .collect();
    serde_json::Value::Array(items)
}

fn format_tree(results: &[QueryResult]) -> serde_json::Value {
    // Group by file, then by parent
    let mut by_file: HashMap<&str, Vec<&QueryResult>> = HashMap::new();
    for r in results {
        by_file.entry(r.file.as_str()).or_default().push(r);
    }

    let mut file_entries: Vec<serde_json::Value> = Vec::new();
    let mut file_keys: Vec<&&str> = by_file.keys().collect();
    file_keys.sort();

    for file in file_keys {
        let file_results = &by_file[file];

        // Separate top-level and nested
        let mut top_level: Vec<serde_json::Value> = Vec::new();
        let mut children_map: HashMap<&str, Vec<&QueryResult>> = HashMap::new();

        for r in file_results {
            if let Some(ref parent) = r.parent {
                children_map.entry(parent.as_str()).or_default().push(r);
            } else {
                top_level.push(result_to_tree_node(r, &children_map));
            }
        }

        // For top-level items, attach their children
        let items: Vec<serde_json::Value> = top_level
            .into_iter()
            .map(|mut node| {
                if let Some(name) = node.get("name").and_then(|n| n.as_str())
                    && let Some(children) = children_map.get(name)
                {
                    let child_nodes: Vec<serde_json::Value> = children
                        .iter()
                        .map(|c| {
                            serde_json::json!({
                                "name": c.name,
                                "kind": c.kind,
                                "line": c.line,
                            })
                        })
                        .collect();
                    node["children"] = serde_json::Value::Array(child_nodes);
                }
                node
            })
            .collect();

        file_entries.push(serde_json::json!({
            "file": file,
            "symbols": items,
        }));
    }

    serde_json::Value::Array(file_entries)
}

fn result_to_tree_node(
    r: &QueryResult,
    _children: &HashMap<&str, Vec<&QueryResult>>,
) -> serde_json::Value {
    serde_json::json!({
        "name": r.name,
        "kind": r.kind,
        "line": r.line,
        "exported": r.exported,
    })
}

fn format_locations(results: &[QueryResult]) -> serde_json::Value {
    let items: Vec<serde_json::Value> = results
        .iter()
        .map(|r| serde_json::Value::String(format!("{}:{}", r.file, r.line)))
        .collect();
    serde_json::Value::Array(items)
}

fn format_summary(results: &[QueryResult], files_parsed: usize) -> serde_json::Value {
    let mut by_kind: HashMap<&str, usize> = HashMap::new();
    let mut by_file: HashMap<&str, usize> = HashMap::new();

    for r in results {
        *by_kind.entry(r.kind.as_str()).or_default() += 1;
        *by_file.entry(r.file.as_str()).or_default() += 1;
    }

    let mut kind_entries: Vec<serde_json::Value> = by_kind
        .iter()
        .map(|(k, v)| serde_json::json!({"kind": k, "count": v}))
        .collect();
    kind_entries.sort_by(|a, b| b["count"].as_u64().cmp(&a["count"].as_u64()));

    let mut file_entries: Vec<serde_json::Value> = by_file
        .iter()
        .map(|(f, c)| serde_json::json!({"file": f, "count": c}))
        .collect();
    file_entries.sort_by(|a, b| b["count"].as_u64().cmp(&a["count"].as_u64()));

    serde_json::json!({
        "total_symbols": results.len(),
        "files_with_matches": by_file.len(),
        "files_parsed": files_parsed,
        "by_kind": kind_entries,
        "by_file": file_entries,
    })
}
