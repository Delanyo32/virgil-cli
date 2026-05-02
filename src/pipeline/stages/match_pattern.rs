use crate::language::Language;
use crate::pipeline::dsl::{PipelineNode, WhereClause};
use crate::storage::workspace::Workspace;

/// Returns the parameter identifier names declared in a function-like node.
fn collect_function_params<'a>(
    func_node: &tree_sitter::Node,
    source: &'a [u8],
    _lang: Language,
) -> Vec<&'a str> {
    let mut params = Vec::new();
    let Some(params_node) = func_node.child_by_field_name("parameters") else {
        return params;
    };
    let mut cursor = params_node.walk();
    for child in params_node.named_children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                if let Ok(name) = child.utf8_text(source) {
                    params.push(name);
                }
            }
            "required_parameter" | "optional_parameter" => {
                if let Some(pattern) = child.child_by_field_name("pattern")
                    && pattern.kind() == "identifier"
                    && let Ok(name) = pattern.utf8_text(source)
                {
                    params.push(name);
                }
            }
            _ => {}
        }
    }
    params
}

/// Build a child→parent map for an entire tree in one DFS pass.
fn build_parent_map(
    root: tree_sitter::Node,
) -> std::collections::HashMap<usize, tree_sitter::Node> {
    let mut map = std::collections::HashMap::new();
    let mut stack = vec![root];
    while let Some(current) = stack.pop() {
        let mut cursor = current.walk();
        for child in current.children(&mut cursor) {
            map.insert(child.id(), current);
            stack.push(child);
        }
    }
    map
}

/// For an assignment_expression node, returns true if the LHS member-expression
/// object is a named parameter of the nearest enclosing function.
fn node_lhs_is_parameter(
    node: &tree_sitter::Node,
    parent_map: &std::collections::HashMap<usize, tree_sitter::Node>,
    source: &[u8],
    lang: Language,
) -> bool {
    let kind = node.kind();
    if kind != "assignment_expression" && kind != "augmented_assignment_expression" {
        return false;
    }
    let Some(lhs) = node.child_by_field_name("left") else {
        return false;
    };
    if lhs.kind() != "member_expression" {
        return false;
    }
    let mut root_obj = lhs;
    loop {
        let Some(obj) = root_obj.child_by_field_name("object") else {
            return false;
        };
        if obj.kind() == "identifier" {
            root_obj = obj;
            break;
        } else if obj.kind() == "member_expression" {
            root_obj = obj;
        } else {
            return false;
        }
    }
    let Ok(obj_name) = root_obj.utf8_text(source) else {
        return false;
    };

    let func_kinds = crate::graph::metrics::function_node_kinds_for_language(lang);
    let mut current_id = node.id();
    while let Some(parent) = parent_map.get(&current_id) {
        if func_kinds.contains(&parent.kind()) {
            let params = collect_function_params(parent, source, lang);
            return params.contains(&obj_name);
        }
        current_id = parent.id();
    }
    false
}

pub(crate) fn execute_match_pattern(
    query_str: &str,
    when: Option<&WhereClause>,
    workspace: &Workspace,
    pipeline_languages: Option<&[String]>,
) -> anyhow::Result<Vec<PipelineNode>> {
    use streaming_iterator::StreamingIterator;

    let mut result = Vec::new();

    for rel_path in workspace.files() {
        let Some(lang) = workspace.file_language(rel_path) else {
            continue;
        };

        if let Some(langs) = pipeline_languages {
            let lang_str = lang.as_str();
            if !langs.iter().any(|l| l.eq_ignore_ascii_case(lang_str)) {
                continue;
            }
        }

        let Some(source) = workspace.read_file(rel_path) else {
            continue;
        };

        let ts_lang = lang.tree_sitter_language();

        let query = match tree_sitter::Query::new(&ts_lang, query_str) {
            Ok(q) => q,
            Err(_e) => continue,
        };

        let mut parser = crate::parser::create_parser(lang)?;
        let tree = match parser.parse(source.as_bytes(), None) {
            Some(t) => t,
            None => {
                eprintln!("Warning: match_pattern: failed to parse {rel_path}");
                continue;
            }
        };

        let parent_map = when
            .and_then(|wc| wc.lhs_is_parameter)
            .map(|_| build_parent_map(tree.root_node()));

        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
        while let Some(m) = matches.next() {
            for cap in m.captures {
                let node = cap.node;
                if let Some(wc) = when
                    && wc.lhs_is_parameter == Some(true)
                    && let Some(ref pm) = parent_map
                    && !node_lhs_is_parameter(&node, pm, source.as_bytes(), lang)
                {
                    continue;
                }
                let line = node.start_position().row as u32 + 1;
                result.push(PipelineNode {
                    node_idx: petgraph::graph::NodeIndex::new(0),
                    file_path: rel_path.clone(),
                    name: node.utf8_text(source.as_bytes()).unwrap_or("").to_string(),
                    kind: node.kind().to_string(),
                    line,
                    exported: false,
                    language: lang.as_str().to_string(),
                    metrics: std::collections::HashMap::new(),
                });
            }
        }
    }

    Ok(result)
}
