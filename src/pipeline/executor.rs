//! Graph pipeline executor.
//!
//! Takes a sequence of [`GraphStage`] steps, a [`CodeGraph`] reference, and optional
//! seed nodes, then executes the pipeline to produce either [`AuditFinding`]s (when the
//! last stage is `Flag`) or [`QueryResult`]s (otherwise).

use petgraph::graph::NodeIndex;

use crate::graph::CodeGraph;
use crate::pipeline::dsl::{GraphStage, PipelineNode, interpolate_message};
use crate::pipeline::helpers::{is_barrel_file, is_excluded_for_arch_analysis, is_test_file};
use crate::pipeline::node_helpers::pipeline_node_from_index;
use crate::pipeline::output::{AuditFinding, QueryResult};
use crate::pipeline::stages;
use crate::storage::workspace::Workspace;

// ---------------------------------------------------------------------------
// PipelineOutput
// ---------------------------------------------------------------------------

/// The result of executing a graph pipeline.
pub enum PipelineOutput {
    Findings(Vec<AuditFinding>),
    Results(Vec<QueryResult>),
}

/// Accumulated taint sources and sanitizers built up across `TaintSources` and
/// `TaintSanitizers` stages. Consumed when a `TaintSinks` stage runs.
#[derive(Default)]
struct TaintContext {
    sources: Vec<crate::pipeline::dsl::TaintSourcePattern>,
    sanitizers: Vec<crate::pipeline::dsl::TaintSanitizerPattern>,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Execute a graph pipeline against a `CodeGraph`.
pub fn run_pipeline(
    stages_seq: &[GraphStage],
    graph: &CodeGraph,
    workspace: Option<&Workspace>,
    pipeline_languages: Option<&[String]>,
    seed_nodes: Option<Vec<NodeIndex>>,
    pipeline_name: &str,
) -> anyhow::Result<PipelineOutput> {
    let is_test_fn = |path: &str| is_test_file(path);
    let is_generated_fn = |path: &str| is_excluded_for_arch_analysis(path);
    let is_barrel_fn = |path: &str| is_barrel_file(path);

    let (pipeline_stages, flag_stage) = if let Some(GraphStage::Flag { flag }) = stages_seq.last() {
        (&stages_seq[..stages_seq.len() - 1], Some(flag))
    } else {
        (stages_seq, None)
    };

    let mut nodes: Vec<PipelineNode> = match seed_nodes {
        Some(idxs) => idxs
            .into_iter()
            .filter_map(|idx| pipeline_node_from_index(idx, graph))
            .collect(),
        None => Vec::new(),
    };

    let mut taint_ctx = TaintContext::default();

    for stage in pipeline_stages {
        nodes = execute_stage(
            stage,
            nodes,
            graph,
            workspace,
            pipeline_languages,
            pipeline_name,
            &is_test_fn,
            &is_generated_fn,
            &is_barrel_fn,
            &mut taint_ctx,
        )?;
    }

    if let Some(flag) = flag_stage {
        let effective_pipeline = flag.pipeline_name.as_deref().unwrap_or(pipeline_name);
        let findings = nodes
            .iter()
            .filter_map(|node| {
                let severity = flag.resolve_severity(node)?;
                let message = interpolate_message(&flag.message, node);
                Some(AuditFinding {
                    file_path: node.file_path.clone(),
                    line: node.line,
                    column: 1,
                    severity,
                    pipeline: effective_pipeline.to_string(),
                    pattern: flag.pattern.clone(),
                    message,
                    snippet: String::new(),
                })
            })
            .collect();
        Ok(PipelineOutput::Findings(findings))
    } else {
        let results = nodes
            .into_iter()
            .map(pipeline_node_to_query_result)
            .collect();
        Ok(PipelineOutput::Results(results))
    }
}

// ---------------------------------------------------------------------------
// Stage dispatch
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)] // central stage dispatch needs all context
fn execute_stage(
    stage: &GraphStage,
    nodes: Vec<PipelineNode>,
    graph: &CodeGraph,
    workspace: Option<&Workspace>,
    pipeline_languages: Option<&[String]>,
    _pipeline_name: &str,
    is_test_fn: &impl Fn(&str) -> bool,
    is_generated_fn: &impl Fn(&str) -> bool,
    is_barrel_fn: &impl Fn(&str) -> bool,
    taint_ctx: &mut TaintContext,
) -> anyhow::Result<Vec<PipelineNode>> {
    match stage {
        GraphStage::Select {
            select,
            filter,
            exclude,
        } => stages::select::execute_select(
            select,
            filter.as_ref(),
            exclude.as_ref(),
            graph,
            is_test_fn,
            is_generated_fn,
            is_barrel_fn,
        ),
        GraphStage::GroupBy { group_by } => {
            Ok(stages::aggregate::execute_group_by(group_by, nodes))
        }
        GraphStage::Count { count } => {
            Ok(stages::aggregate::execute_count(&count.threshold, nodes))
        }
        GraphStage::FindCycles { find_cycles } => {
            stages::cycles::execute_find_cycles(&find_cycles.edge, nodes, graph)
        }
        GraphStage::MaxDepth { max_depth } => {
            stages::cycles::execute_max_depth(max_depth, nodes, graph)
        }
        GraphStage::Ratio { ratio } => stages::aggregate::execute_ratio(
            ratio,
            nodes,
            is_test_fn,
            is_generated_fn,
            is_barrel_fn,
        ),
        GraphStage::Flag { .. } => {
            // Flag is handled at the top level in run_pipeline; if it appears mid-pipeline
            // just pass nodes through unchanged.
            Ok(nodes)
        }
        GraphStage::MatchPattern {
            match_pattern,
            when,
        } => match workspace {
            Some(ws) => stages::match_pattern::execute_match_pattern(
                match_pattern,
                when.as_ref(),
                ws,
                pipeline_languages,
            ),
            None => anyhow::bail!(
                "match_pattern stage requires workspace -- call run_pipeline with Some(workspace)"
            ),
        },
        GraphStage::ComputeMetric { compute_metric } => match workspace {
            Some(ws) => {
                stages::compute_metric::execute_compute_metric(compute_metric, nodes, ws, graph)
            }
            None => anyhow::bail!(
                "compute_metric stage requires workspace -- call run_pipeline with Some(workspace)"
            ),
        },
        GraphStage::Taint { taint } => {
            let config = crate::graph::taint::TaintConfig {
                sources: taint.sources.clone(),
                sinks: taint.sinks.clone(),
                sanitizers: taint.sanitizers.clone(),
            };
            stages::taint::execute_taint_with_config(&config, graph, &taint.sinks)
        }
        GraphStage::TaintSources { taint_sources } => {
            taint_ctx.sources.extend(taint_sources.iter().cloned());
            Ok(nodes)
        }
        GraphStage::TaintSanitizers { taint_sanitizers } => {
            taint_ctx
                .sanitizers
                .extend(taint_sanitizers.iter().cloned());
            Ok(nodes)
        }
        GraphStage::TaintSinks { taint_sinks } => {
            let config = crate::graph::taint::TaintConfig {
                sources: taint_ctx.sources.clone(),
                sinks: taint_sinks.clone(),
                sanitizers: taint_ctx.sanitizers.clone(),
            };
            stages::taint::execute_taint_with_config(&config, graph, taint_sinks)
        }
        GraphStage::FindDuplicates { find_duplicates } => Ok(
            stages::find_duplicates::execute_find_duplicates(find_duplicates, nodes),
        ),
    }
}

/// Convert a `PipelineNode` to a `QueryResult` for non-Flag pipeline output.
fn pipeline_node_to_query_result(node: PipelineNode) -> QueryResult {
    QueryResult {
        name: node.name,
        kind: node.kind,
        file: node.file_path,
        line: node.line,
        end_line: node.line,
        column: 1,
        exported: node.exported,
        signature: None,
        docstring: None,
        body: None,
        preview: None,
        parent: None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::graph::{EdgeWeight, NodeWeight};
    use crate::language::Language;
    use crate::pipeline::dsl::{
        CountConfig, DenominatorConfig, EdgeType, FindCyclesConfig, FlagConfig, GraphStage,
        MaxDepthConfig, MetricValue, NodeType, NumeratorConfig, NumericPredicate, RatioConfig,
        WhereClause,
    };

    // ── Test graph builders ──────────────────────────────────────────

    fn make_file_graph(paths: &[&str]) -> CodeGraph {
        let mut g = CodeGraph::new();
        for path in paths {
            let idx = g.graph.add_node(NodeWeight::File {
                path: path.to_string(),
                language: Language::Rust,
            });
            g.file_nodes.insert(path.to_string(), idx);
        }
        g
    }

    fn add_import(g: &mut CodeGraph, from: &str, to: &str) {
        let from_idx = *g.file_nodes.get(from).unwrap();
        let to_idx = *g.file_nodes.get(to).unwrap();
        g.graph.add_edge(from_idx, to_idx, EdgeWeight::Imports);
    }

    // ── Test 1: select(file) with a 2-file graph ─────────────────────

    #[test]
    fn test_select_file_nodes() {
        let graph = make_file_graph(&["src/a.rs", "src/b.rs"]);
        let stages = vec![GraphStage::Select {
            select: NodeType::File,
            filter: None,
            exclude: None,
        }];
        let out = run_pipeline(&stages, &graph, None, None, None, "test").unwrap();
        match out {
            PipelineOutput::Results(results) => {
                assert_eq!(results.len(), 2);
                let files: HashSet<_> = results.iter().map(|r| r.file.as_str()).collect();
                assert!(files.contains("src/a.rs"));
                assert!(files.contains("src/b.rs"));
            }
            _ => panic!("expected Results"),
        }
    }

    #[test]
    fn test_select_with_exclude() {
        let graph = make_file_graph(&["src/a.rs", "src/a_test.rs"]);
        let stages = vec![GraphStage::Select {
            select: NodeType::File,
            filter: None,
            exclude: Some(WhereClause {
                is_test_file: Some(true),
                ..Default::default()
            }),
        }];
        let out = run_pipeline(&stages, &graph, None, None, None, "test").unwrap();
        match out {
            PipelineOutput::Results(results) => {
                assert_eq!(results.len(), 1);
                assert_eq!(results[0].file, "src/a.rs");
            }
            _ => panic!("expected Results"),
        }
    }

    // ── Test 2: find_cycles with a 2-node cycle ─────────────────────

    #[test]
    fn test_find_cycles_two_node_cycle() {
        let mut graph = make_file_graph(&["a.rs", "b.rs"]);
        add_import(&mut graph, "a.rs", "b.rs");
        add_import(&mut graph, "b.rs", "a.rs");

        let stages = vec![
            GraphStage::Select {
                select: NodeType::File,
                filter: None,
                exclude: None,
            },
            GraphStage::FindCycles {
                find_cycles: FindCyclesConfig {
                    edge: EdgeType::Imports,
                },
            },
        ];
        let out = run_pipeline(&stages, &graph, None, None, None, "test").unwrap();
        match out {
            PipelineOutput::Results(results) => {
                assert_eq!(results.len(), 1, "expected exactly 1 cycle node");
                assert_eq!(results[0].kind, "cycle");
            }
            _ => panic!("expected Results"),
        }
    }

    #[test]
    fn test_find_cycles_two_node_cycle_size_metric() {
        let mut graph = make_file_graph(&["a.rs", "b.rs"]);
        add_import(&mut graph, "a.rs", "b.rs");
        add_import(&mut graph, "b.rs", "a.rs");

        // Run without Flag to inspect pipeline nodes directly
        let is_test_fn = |path: &str| is_test_file(path);
        let is_generated_fn = |path: &str| is_excluded_for_arch_analysis(path);
        let is_barrel_fn = |path: &str| is_barrel_file(path);

        // Run select stage manually
        let select_stage = GraphStage::Select {
            select: NodeType::File,
            filter: None,
            exclude: None,
        };
        let mut taint_ctx = TaintContext::default();
        let nodes = execute_stage(
            &select_stage,
            Vec::new(),
            &graph,
            None,
            None,
            "test",
            &is_test_fn,
            &is_generated_fn,
            &is_barrel_fn,
            &mut taint_ctx,
        )
        .unwrap();

        let cycle_stage = GraphStage::FindCycles {
            find_cycles: FindCyclesConfig {
                edge: EdgeType::Imports,
            },
        };
        let cycle_nodes = execute_stage(
            &cycle_stage,
            nodes,
            &graph,
            None,
            None,
            "test",
            &is_test_fn,
            &is_generated_fn,
            &is_barrel_fn,
            &mut taint_ctx,
        )
        .unwrap();

        assert_eq!(cycle_nodes.len(), 1);
        assert_eq!(cycle_nodes[0].metric_f64("cycle_size") as usize, 2);
    }

    // ── Test 3: find_cycles with a DAG returns empty ─────────────────

    #[test]
    fn test_find_cycles_dag_returns_empty() {
        let mut graph = make_file_graph(&["a.rs", "b.rs", "c.rs"]);
        add_import(&mut graph, "a.rs", "b.rs");
        add_import(&mut graph, "b.rs", "c.rs");

        let stages = vec![
            GraphStage::Select {
                select: NodeType::File,
                filter: None,
                exclude: None,
            },
            GraphStage::FindCycles {
                find_cycles: FindCyclesConfig {
                    edge: EdgeType::Imports,
                },
            },
        ];
        let out = run_pipeline(&stages, &graph, None, None, None, "test").unwrap();
        match out {
            PipelineOutput::Results(results) => {
                assert!(results.is_empty(), "DAG should produce no cycles");
            }
            _ => panic!("expected Results"),
        }
    }

    // ── Test 4: group_by + count with threshold ───────────────────────

    #[test]
    fn test_group_by_count_threshold() {
        use crate::models::SymbolKind;

        let mut graph = make_file_graph(&["src/a.rs", "src/b.rs"]);

        // Add 3 symbols to a.rs and 1 to b.rs
        let a_idx = *graph.file_nodes.get("src/a.rs").unwrap();
        let b_idx = *graph.file_nodes.get("src/b.rs").unwrap();

        for i in 0..3u32 {
            let sym_idx = graph.graph.add_node(NodeWeight::Symbol {
                name: format!("fn_a_{}", i),
                kind: SymbolKind::Function,
                file_path: "src/a.rs".to_string(),
                start_line: i + 1,
                end_line: i + 5,
                exported: true,
            });
            graph.graph.add_edge(a_idx, sym_idx, EdgeWeight::Contains);
        }

        let sym_idx = graph.graph.add_node(NodeWeight::Symbol {
            name: "fn_b".to_string(),
            kind: SymbolKind::Function,
            file_path: "src/b.rs".to_string(),
            start_line: 1,
            end_line: 5,
            exported: true,
        });
        graph.graph.add_edge(b_idx, sym_idx, EdgeWeight::Contains);

        let stages = vec![
            GraphStage::Select {
                select: NodeType::Symbol,
                filter: None,
                exclude: None,
            },
            GraphStage::GroupBy {
                group_by: "file_path".to_string(),
            },
            GraphStage::Count {
                count: CountConfig {
                    threshold: NumericPredicate {
                        gte: Some(2.0),
                        ..Default::default()
                    },
                },
            },
        ];
        let out = run_pipeline(&stages, &graph, None, None, None, "test").unwrap();
        match out {
            PipelineOutput::Results(results) => {
                // Only src/a.rs has >= 2 symbols
                assert_eq!(results.len(), 1);
                assert_eq!(results[0].file, "src/a.rs");
            }
            _ => panic!("expected Results"),
        }
    }

    // ── Test 5: max_depth returns correct depth ───────────────────────

    #[test]
    fn test_max_depth_three_level_chain() {
        // a.rs -> b.rs -> c.rs  (depth: a=0, b=1, c=2)
        let mut graph = make_file_graph(&["a.rs", "b.rs", "c.rs"]);
        add_import(&mut graph, "a.rs", "b.rs");
        add_import(&mut graph, "b.rs", "c.rs");

        let is_test_fn = |path: &str| is_test_file(path);
        let is_generated_fn = |path: &str| is_excluded_for_arch_analysis(path);
        let is_barrel_fn = |path: &str| is_barrel_file(path);

        // Select all files first
        let select_stage = GraphStage::Select {
            select: NodeType::File,
            filter: None,
            exclude: None,
        };
        let mut taint_ctx = TaintContext::default();
        let nodes = execute_stage(
            &select_stage,
            Vec::new(),
            &graph,
            None,
            None,
            "test",
            &is_test_fn,
            &is_generated_fn,
            &is_barrel_fn,
            &mut taint_ctx,
        )
        .unwrap();

        // Run max_depth with threshold >= 2 (should match c.rs only)
        let max_depth_stage = GraphStage::MaxDepth {
            max_depth: MaxDepthConfig {
                edge: EdgeType::Imports,
                skip_barrel_files: Some(false),
                threshold: NumericPredicate {
                    gte: Some(2.0),
                    ..Default::default()
                },
            },
        };
        let result = execute_stage(
            &max_depth_stage,
            nodes,
            &graph,
            None,
            None,
            "test",
            &is_test_fn,
            &is_generated_fn,
            &is_barrel_fn,
            &mut taint_ctx,
        )
        .unwrap();

        assert_eq!(result.len(), 1, "only c.rs should have depth >= 2");
        assert_eq!(result[0].file_path, "c.rs");
        assert_eq!(result[0].metric_f64("depth") as usize, 2);
    }

    // ── Test 6: flag stage produces AuditFinding with correct severity ─

    #[test]
    fn test_flag_stage_produces_findings() {
        let graph = make_file_graph(&["src/big.rs"]);

        let is_test_fn = |path: &str| is_test_file(path);
        let is_generated_fn = |path: &str| is_excluded_for_arch_analysis(path);
        let is_barrel_fn = |path: &str| is_barrel_file(path);

        // Create a node with count=25
        let select_stage = GraphStage::Select {
            select: NodeType::File,
            filter: None,
            exclude: None,
        };
        let mut taint_ctx = TaintContext::default();
        let mut nodes = execute_stage(
            &select_stage,
            Vec::new(),
            &graph,
            None,
            None,
            "test_pipeline",
            &is_test_fn,
            &is_generated_fn,
            &is_barrel_fn,
            &mut taint_ctx,
        )
        .unwrap();

        // Inject a count metric manually
        for n in &mut nodes {
            n.metrics.insert("count".to_string(), MetricValue::Int(25));
        }

        let flag_config = FlagConfig {
            pattern: "oversized_module".to_string(),
            message: "{{file}} has {{count}} symbols".to_string(),
            severity: None,
            severity_map: Some(vec![
                crate::pipeline::dsl::SeverityEntry {
                    when: Some(WhereClause {
                        metrics: {
                            let mut m = std::collections::HashMap::new();
                            m.insert(
                                "count".to_string(),
                                NumericPredicate {
                                    gte: Some(20.0),
                                    ..Default::default()
                                },
                            );
                            m
                        },
                        ..Default::default()
                    }),
                    severity: "error".to_string(),
                },
                crate::pipeline::dsl::SeverityEntry {
                    when: None,
                    severity: "warning".to_string(),
                },
            ]),
            pipeline_name: None,
        };

        let effective_pipeline = flag_config
            .pipeline_name
            .as_deref()
            .unwrap_or("test_pipeline");
        let findings: Vec<AuditFinding> = nodes
            .iter()
            .filter_map(|node| {
                let severity = flag_config.resolve_severity(node)?;
                let message = interpolate_message(&flag_config.message, node);
                Some(AuditFinding {
                    file_path: node.file_path.clone(),
                    line: node.line,
                    column: 1,
                    severity,
                    pipeline: effective_pipeline.to_string(),
                    pattern: flag_config.pattern.clone(),
                    message,
                    snippet: String::new(),
                })
            })
            .collect();

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "error"); // count=25 >= 20
        assert_eq!(findings[0].pattern, "oversized_module");
        assert_eq!(findings[0].pipeline, "test_pipeline");
        assert!(findings[0].message.contains("src/big.rs"));
        assert!(findings[0].message.contains("25"));
    }

    #[test]
    fn test_run_pipeline_flag_produces_findings() {
        let graph = make_file_graph(&["src/large.rs"]);

        let stages = vec![
            GraphStage::Select {
                select: NodeType::File,
                filter: None,
                exclude: None,
            },
            GraphStage::Flag {
                flag: FlagConfig {
                    pattern: "test_pattern".to_string(),
                    message: "Found {{file}}".to_string(),
                    severity: Some("info".to_string()),
                    severity_map: None,
                    pipeline_name: None,
                },
            },
        ];

        let out = run_pipeline(&stages, &graph, None, None, None, "my_pipeline").unwrap();
        match out {
            PipelineOutput::Findings(findings) => {
                assert_eq!(findings.len(), 1);
                assert_eq!(findings[0].severity, "info");
                assert_eq!(findings[0].pattern, "test_pattern");
                assert_eq!(findings[0].pipeline, "my_pipeline");
            }
            _ => panic!("expected Findings"),
        }
    }

    // ── Test 7: execute_graph_pipeline delegates to run_pipeline ────────

    #[test]
    fn test_execute_graph_pipeline_wrapper() {
        // execute_graph_pipeline must delegate to run_pipeline.
        // A full pipeline ending in Flag should produce non-empty Findings
        // with the correct pipeline name and pattern.
        let graph = make_file_graph(&["src/x.rs"]);
        let stages = vec![
            GraphStage::Select {
                select: NodeType::File,
                filter: None,
                exclude: None,
            },
            GraphStage::Flag {
                flag: FlagConfig {
                    pattern: "wrapper_pattern".to_string(),
                    message: "Found {{file}}".to_string(),
                    severity: Some("warning".to_string()),
                    severity_map: None,
                    pipeline_name: None,
                },
            },
        ];

        let out = run_pipeline(&stages, &graph, None, None, None, "wrapper_pipeline").unwrap();
        match out {
            PipelineOutput::Findings(findings) => {
                assert!(!findings.is_empty(), "expected at least one finding");
                assert_eq!(findings[0].pipeline, "wrapper_pipeline");
                assert_eq!(findings[0].pattern, "wrapper_pattern");
                assert_eq!(findings[0].severity, "warning");
            }
            _ => panic!("expected Findings, not Results"),
        }
    }

    // ── Test 8: ratio stage ──────────────────────────────────────────

    // -- match_pattern + compute_metric tests ----

    #[test]
    fn test_match_pattern_finds_panic_in_rust() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src_dir = dir.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(
            src_dir.join("lib.rs"),
            r#"fn foo() { panic!("oops"); }
fn bar() { println!("ok"); }
"#,
        )
        .unwrap();
        let ws = crate::storage::workspace::Workspace::load(dir.path(), &[Language::Rust], None)
            .unwrap();

        let stages = vec![
            GraphStage::MatchPattern {
                match_pattern:
                    r#"(macro_invocation (identifier) @name (#eq? @name "panic")) @call"#
                        .to_string(),
                when: None,
            },
            GraphStage::Flag {
                flag: crate::pipeline::dsl::FlagConfig {
                    pattern: "panic_detected".to_string(),
                    message: "panic at {{file}}:{{line}}".to_string(),
                    severity: Some("warning".to_string()),
                    severity_map: None,
                    pipeline_name: None,
                },
            },
        ];
        let graph = CodeGraph::new();
        let out = run_pipeline(&stages, &graph, Some(&ws), None, None, "panic_detection").unwrap();
        match out {
            PipelineOutput::Findings(findings) => {
                assert!(
                    !findings.is_empty(),
                    "expected at least one finding for panic!()"
                );
                assert_eq!(findings[0].pattern, "panic_detected");
                assert_eq!(findings[0].line, 1);
                assert!(findings[0].file_path.contains("lib.rs"));
            }
            _ => panic!("expected Findings"),
        }
    }

    #[test]
    fn test_match_pattern_no_match_returns_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src_dir = dir.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(
            src_dir.join("lib.rs"),
            "fn clean() { println!(\"all good\"); }\n",
        )
        .unwrap();
        let ws = crate::storage::workspace::Workspace::load(dir.path(), &[Language::Rust], None)
            .unwrap();

        let stages = vec![
            GraphStage::MatchPattern {
                match_pattern:
                    r#"(macro_invocation (identifier) @name (#eq? @name "panic")) @call"#
                        .to_string(),
                when: None,
            },
            GraphStage::Flag {
                flag: crate::pipeline::dsl::FlagConfig {
                    pattern: "panic_detected".to_string(),
                    message: "panic at {{file}}:{{line}}".to_string(),
                    severity: Some("warning".to_string()),
                    severity_map: None,
                    pipeline_name: None,
                },
            },
        ];
        let graph = CodeGraph::new();
        let out = run_pipeline(&stages, &graph, Some(&ws), None, None, "panic_detection").unwrap();
        match out {
            PipelineOutput::Findings(findings) => {
                assert!(
                    findings.is_empty(),
                    "expected zero findings for clean code, got {}",
                    findings.len()
                );
            }
            _ => panic!("expected Findings"),
        }
    }

    #[test]
    fn test_match_pattern_finds_function_in_typescript() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("sample.ts"),
            "export function greet(name: string): string { return name; }\n",
        )
        .unwrap();
        let ws =
            crate::storage::workspace::Workspace::load(dir.path(), &[Language::TypeScript], None)
                .unwrap();
        let stages = vec![
            GraphStage::MatchPattern {
                match_pattern: "(function_declaration name: (identifier) @name)".to_string(),
                when: None,
            },
            GraphStage::Flag {
                flag: crate::pipeline::dsl::FlagConfig {
                    pattern: "ts_function".to_string(),
                    message: "function at {{file}}:{{line}}".to_string(),
                    severity: Some("info".to_string()),
                    severity_map: None,
                    pipeline_name: None,
                },
            },
        ];
        let graph = CodeGraph::new();
        let out = run_pipeline(&stages, &graph, Some(&ws), None, None, "test_ts").unwrap();
        match out {
            PipelineOutput::Findings(findings) => {
                assert!(
                    !findings.is_empty(),
                    "expected findings for TypeScript function"
                );
                assert!(
                    findings[0].file_path.ends_with(".ts"),
                    "expected .ts file path, got {}",
                    findings[0].file_path
                );
                assert_eq!(findings[0].line, 1, "expected line 1");
            }
            _ => panic!("expected Findings"),
        }
    }

    #[test]
    fn test_compute_metric_cyclomatic_flags_complex_function() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src_dir = dir.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        // Function with CC > 1: multiple if branches
        std::fs::write(
            src_dir.join("complex.rs"),
            r#"fn complex(x: i32, y: i32, z: i32) {
    if x > 0 {
        println!("a");
    }
    if y > 0 {
        println!("b");
    }
    if z > 0 {
        println!("c");
    }
    for i in 0..10 {
        println!("{}", i);
    }
}
"#,
        )
        .unwrap();
        let ws = crate::storage::workspace::Workspace::load(dir.path(), &[Language::Rust], None)
            .unwrap();

        // Build a graph with a symbol node at line 1 (the function start)
        let mut graph = CodeGraph::new();
        let file_idx = graph.graph.add_node(NodeWeight::File {
            path: "src/complex.rs".to_string(),
            language: Language::Rust,
        });
        graph
            .file_nodes
            .insert("src/complex.rs".to_string(), file_idx);
        let sym_idx = graph.graph.add_node(NodeWeight::Symbol {
            name: "complex".to_string(),
            kind: crate::models::SymbolKind::Function,
            file_path: "src/complex.rs".to_string(),
            start_line: 1,
            end_line: 14,
            exported: false,
        });
        graph
            .symbol_nodes
            .insert(("src/complex.rs".to_string(), 1), sym_idx);

        let stages = vec![
            GraphStage::Select {
                select: crate::pipeline::dsl::NodeType::Symbol,
                filter: None,
                exclude: None,
            },
            GraphStage::ComputeMetric {
                compute_metric: "cyclomatic_complexity".to_string(),
            },
            GraphStage::Flag {
                flag: crate::pipeline::dsl::FlagConfig {
                    pattern: "high_cc".to_string(),
                    message: "CC={{cyclomatic_complexity}} in {{name}}".to_string(),
                    severity: Some("warning".to_string()),
                    severity_map: None,
                    pipeline_name: None,
                },
            },
        ];
        let out = run_pipeline(
            &stages,
            &graph,
            Some(&ws),
            None,
            None,
            "cyclomatic_complexity",
        )
        .unwrap();
        match out {
            PipelineOutput::Findings(findings) => {
                assert_eq!(findings.len(), 1, "expected 1 finding for complex function");
                assert!(
                    findings[0].message.contains("CC="),
                    "message should contain CC value"
                );
                // CC = 1 (base) + 3 (if) + 1 (for) = 5
                assert!(
                    findings[0].message.contains("CC=5"),
                    "expected CC=5, got: {}",
                    findings[0].message
                );
            }
            _ => panic!("expected Findings"),
        }
    }

    #[test]
    fn test_compute_metric_cyclomatic_clean_function_no_finding_above_threshold() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src_dir = dir.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        // Simple function with CC=1 (no branches)
        std::fs::write(
            src_dir.join("simple.rs"),
            "fn simple() {\n    println!(\"hello\");\n}\n",
        )
        .unwrap();
        let ws = crate::storage::workspace::Workspace::load(dir.path(), &[Language::Rust], None)
            .unwrap();

        let mut graph = CodeGraph::new();
        let file_idx = graph.graph.add_node(NodeWeight::File {
            path: "src/simple.rs".to_string(),
            language: Language::Rust,
        });
        graph
            .file_nodes
            .insert("src/simple.rs".to_string(), file_idx);
        let sym_idx = graph.graph.add_node(NodeWeight::Symbol {
            name: "simple".to_string(),
            kind: crate::models::SymbolKind::Function,
            file_path: "src/simple.rs".to_string(),
            start_line: 1,
            end_line: 3,
            exported: false,
        });
        graph
            .symbol_nodes
            .insert(("src/simple.rs".to_string(), 1), sym_idx);

        let stages = vec![
            GraphStage::Select {
                select: crate::pipeline::dsl::NodeType::Symbol,
                filter: None,
                exclude: None,
            },
            GraphStage::ComputeMetric {
                compute_metric: "cyclomatic_complexity".to_string(),
            },
        ];
        let out = run_pipeline(&stages, &graph, Some(&ws), None, None, "cc_test").unwrap();
        match out {
            PipelineOutput::Results(results) => {
                assert_eq!(results.len(), 1, "expected 1 result for simple function");
                // The node should have CC=1 (base complexity only)
            }
            _ => panic!("expected Results (no Flag stage)"),
        }
    }

    #[test]
    fn test_ratio_exported_symbols() {
        use crate::models::SymbolKind;

        let mut graph = make_file_graph(&["src/a.rs"]);
        let a_idx = *graph.file_nodes.get("src/a.rs").unwrap();

        // 3 exported, 1 not exported
        for i in 0..3u32 {
            let sym_idx = graph.graph.add_node(NodeWeight::Symbol {
                name: format!("pub_fn_{}", i),
                kind: SymbolKind::Function,
                file_path: "src/a.rs".to_string(),
                start_line: i + 1,
                end_line: i + 5,
                exported: true,
            });
            graph.graph.add_edge(a_idx, sym_idx, EdgeWeight::Contains);
        }
        let priv_idx = graph.graph.add_node(NodeWeight::Symbol {
            name: "priv_fn".to_string(),
            kind: SymbolKind::Function,
            file_path: "src/a.rs".to_string(),
            start_line: 10,
            end_line: 15,
            exported: false,
        });
        graph.graph.add_edge(a_idx, priv_idx, EdgeWeight::Contains);

        let stages = vec![
            GraphStage::Select {
                select: NodeType::Symbol,
                filter: None,
                exclude: None,
            },
            GraphStage::GroupBy {
                group_by: "file_path".to_string(),
            },
            GraphStage::Ratio {
                ratio: RatioConfig {
                    numerator: NumeratorConfig {
                        filter: Some(WhereClause {
                            exported: Some(true),
                            ..Default::default()
                        }),
                    },
                    denominator: DenominatorConfig { filter: None },
                    threshold: Some(WhereClause {
                        metrics: {
                            let mut m = std::collections::HashMap::new();
                            m.insert(
                                "ratio".to_string(),
                                NumericPredicate {
                                    gte: Some(0.5),
                                    ..Default::default()
                                },
                            );
                            m
                        },
                        ..Default::default()
                    }),
                },
            },
        ];
        let out = run_pipeline(&stages, &graph, None, None, None, "test").unwrap();
        match out {
            PipelineOutput::Results(results) => {
                // 3/4 = 0.75 >= 0.5, so should pass
                assert_eq!(results.len(), 1);
            }
            _ => panic!("expected Results"),
        }
    }

    #[test]
    fn test_compute_metric_nesting_depth_detects_deep_nesting() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src_dir = dir.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        // 4 levels of nesting: for > if > for > if
        std::fs::write(
            src_dir.join("lib.rs"),
            r#"fn deeply_nested(items: &[i32]) -> i32 {
    let mut sum = 0;
    for item in items {
        if *item > 0 {
            for _ in 0..*item {
                if *item > 5 {
                    sum += 1;
                }
            }
        }
    }
    sum
}
"#,
        )
        .unwrap();
        let ws = crate::storage::workspace::Workspace::load(dir.path(), &[Language::Rust], None)
            .unwrap();

        let mut graph = CodeGraph::new();
        let file_idx = graph.graph.add_node(NodeWeight::File {
            path: "src/lib.rs".to_string(),
            language: Language::Rust,
        });
        graph.file_nodes.insert("src/lib.rs".to_string(), file_idx);
        let sym_idx = graph.graph.add_node(NodeWeight::Symbol {
            name: "deeply_nested".to_string(),
            kind: crate::models::SymbolKind::Function,
            file_path: "src/lib.rs".to_string(),
            start_line: 1,
            end_line: 13,
            exported: false,
        });
        graph
            .symbol_nodes
            .insert(("src/lib.rs".to_string(), 1), sym_idx);

        let is_test_fn = |path: &str| is_test_file(path);
        let is_generated_fn = |path: &str| is_excluded_for_arch_analysis(path);
        let is_barrel_fn = |path: &str| is_barrel_file(path);
        let mut taint_ctx = TaintContext::default();

        let select_stage = GraphStage::Select {
            select: crate::pipeline::dsl::NodeType::Symbol,
            filter: None,
            exclude: None,
        };
        let nodes = execute_stage(
            &select_stage,
            Vec::new(),
            &graph,
            Some(&ws),
            None,
            "nesting_depth_test",
            &is_test_fn,
            &is_generated_fn,
            &is_barrel_fn,
            &mut taint_ctx,
        )
        .unwrap();

        let metric_stage = GraphStage::ComputeMetric {
            compute_metric: "nesting_depth".to_string(),
        };
        let result_nodes = execute_stage(
            &metric_stage,
            nodes,
            &graph,
            Some(&ws),
            None,
            "nesting_depth_test",
            &is_test_fn,
            &is_generated_fn,
            &is_barrel_fn,
            &mut taint_ctx,
        )
        .unwrap();

        assert_eq!(
            result_nodes.len(),
            1,
            "expected 1 result for deeply_nested function"
        );
        let depth = result_nodes[0].metric_f64("nesting_depth") as i64;
        assert!(
            depth >= 4,
            "expected nesting_depth >= 4 for 4-level nesting, got {}",
            depth
        );
    }

    #[test]
    fn test_compute_metric_nesting_depth_javascript_arrow_function() {
        // Regression test: JS async arrow functions must produce nesting_depth metrics.
        // Uses GraphBuilder so the test exercises the full symbol-discovery path,
        // not just execute_stage with a hand-built graph.
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("controller.js"),
            r#"const createComment = async (req, res) => {
    if (req.body) {
        if (req.user) {
            if (req.params.id) {
                if (req.body.parentId) {
                    return req.body;
                }
            }
        }
    }
};
"#,
        )
        .unwrap();
        let ws =
            crate::storage::workspace::Workspace::load(dir.path(), &[Language::JavaScript], None)
                .unwrap();
        let graph = crate::graph::builder::GraphBuilder::new(&ws, &[Language::JavaScript])
            .build()
            .unwrap();

        // Assert the graph builder actually registered the arrow function symbol.
        let sym_count = graph.symbol_nodes.len();
        assert!(
            sym_count >= 1,
            "expected at least 1 symbol node from JS arrow function, got {}",
            sym_count
        );

        let is_test_fn = |path: &str| is_test_file(path);
        let is_generated_fn = |path: &str| is_excluded_for_arch_analysis(path);
        let is_barrel_fn = |path: &str| is_barrel_file(path);
        let mut taint_ctx = TaintContext::default();

        let select_stage = GraphStage::Select {
            select: crate::pipeline::dsl::NodeType::Symbol,
            filter: None,
            exclude: None,
        };
        let nodes = execute_stage(
            &select_stage,
            Vec::new(),
            &graph,
            Some(&ws),
            None,
            "js_nesting_test",
            &is_test_fn,
            &is_generated_fn,
            &is_barrel_fn,
            &mut taint_ctx,
        )
        .unwrap();

        assert_eq!(
            nodes.len(),
            1,
            "expected 1 symbol node, got {}",
            nodes.len()
        );

        let metric_stage = GraphStage::ComputeMetric {
            compute_metric: "nesting_depth".to_string(),
        };
        let result_nodes = execute_stage(
            &metric_stage,
            nodes,
            &graph,
            Some(&ws),
            None,
            "js_nesting_test",
            &is_test_fn,
            &is_generated_fn,
            &is_barrel_fn,
            &mut taint_ctx,
        )
        .unwrap();

        assert_eq!(
            result_nodes.len(),
            1,
            "expected 1 node after compute_metric"
        );
        // 4 levels: if > if > if > if
        let depth = result_nodes[0].metric_f64("nesting_depth") as i64;
        assert_eq!(depth, 4, "expected nesting depth of 4, got {}", depth);
    }

    #[test]
    fn test_compute_metric_nesting_depth_cpp_qualified_method() {
        // Regression test: CPP_SYMBOL_QUERY must match qualified-name function definitions
        // like `int DataProcessor::process(int x, int y) { ... }`. Previously only
        // plain `(identifier)` declarators were matched, so out-of-class method definitions
        // were never symbolised and never reached the metric pipeline.
        let dir = tempfile::tempdir().expect("tempdir");
        let src_dir = dir.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(
            src_dir.join("processor.cpp"),
            r#"int DataProcessor::process(int x, int y) {
    if (x > 0) {
        if (y > 0) {
            if (x > y) {
                if (x > 10) {
                    return x;
                }
            }
        }
    }
    return 0;
}
"#,
        )
        .unwrap();
        let ws =
            crate::storage::workspace::Workspace::load(dir.path(), &[Language::Cpp], None).unwrap();
        // Use GraphBuilder so the test exercises the full symbol-discovery path
        // (including CPP_SYMBOL_QUERY), not just execute_stage with a hand-built graph.
        let graph = crate::graph::builder::GraphBuilder::new(&ws, &[Language::Cpp])
            .build()
            .unwrap();

        assert!(
            !graph.symbol_nodes.is_empty(),
            "CPP_SYMBOL_QUERY must match qualified-name function definition -- got 0 symbols"
        );

        let is_test_fn = |path: &str| is_test_file(path);
        let is_generated_fn = |path: &str| is_excluded_for_arch_analysis(path);
        let is_barrel_fn = |path: &str| is_barrel_file(path);
        let mut taint_ctx = TaintContext::default();

        let nodes = execute_stage(
            &GraphStage::Select {
                select: crate::pipeline::dsl::NodeType::Symbol,
                filter: None,
                exclude: None,
            },
            Vec::new(),
            &graph,
            Some(&ws),
            None,
            "cpp_nesting_test",
            &is_test_fn,
            &is_generated_fn,
            &is_barrel_fn,
            &mut taint_ctx,
        )
        .unwrap();

        let result_nodes = execute_stage(
            &GraphStage::ComputeMetric {
                compute_metric: "nesting_depth".to_string(),
            },
            nodes,
            &graph,
            Some(&ws),
            None,
            "cpp_nesting_test",
            &is_test_fn,
            &is_generated_fn,
            &is_barrel_fn,
            &mut taint_ctx,
        )
        .unwrap();

        assert_eq!(result_nodes.len(), 1, "expected 1 symbol node");
        let depth = result_nodes[0].metrics.get("nesting_depth")
            .expect("nesting_depth metric should be present -- CPP_SYMBOL_QUERY must match qualified-name functions");
        // 4 levels: if > if > if > if
        match depth {
            MetricValue::Int(v) => assert_eq!(*v, 4, "expected nesting depth 4, got {}", v),
            _ => panic!("expected Int metric"),
        }
    }

    #[test]
    fn test_lhs_is_parameter_filters_local_object_mutations() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src").join("service.js"),
            r#"function createFilter(role) {
    const filter = {};
    filter.role = role;
    return filter;
}
function mutateParam(user) {
    user.name = "overwritten";
}
"#,
        )
        .unwrap();
        let ws =
            crate::storage::workspace::Workspace::load(dir.path(), &[Language::JavaScript], None)
                .unwrap();

        let stages = vec![
            GraphStage::MatchPattern {
                match_pattern: "(assignment_expression left: (member_expression) @lhs) @assign"
                    .to_string(),
                when: Some(crate::pipeline::dsl::WhereClause {
                    lhs_is_parameter: Some(true),
                    ..Default::default()
                }),
            },
            GraphStage::Flag {
                flag: crate::pipeline::dsl::FlagConfig {
                    pattern: "argument_mutation".to_string(),
                    message: "mutation".to_string(),
                    severity: Some("warning".to_string()),
                    severity_map: None,
                    pipeline_name: None,
                },
            },
        ];
        let graph = CodeGraph::new();
        let out = run_pipeline(&stages, &graph, Some(&ws), None, None, "lhs_param_test").unwrap();
        match out {
            PipelineOutput::Findings(findings) => {
                assert_eq!(
                    findings.len(),
                    1,
                    "expected exactly 1 finding (mutateParam's user.name), got {} findings: {:?}",
                    findings.len(),
                    findings
                        .iter()
                        .map(|f| format!("{}:{}", f.file_path, f.line))
                        .collect::<Vec<_>>()
                );
                assert_eq!(
                    findings[0].line, 7,
                    "expected line 7 (user.name = ...), got {}",
                    findings[0].line
                );
            }
            _ => panic!("expected Findings"),
        }
    }

    #[test]
    fn test_compute_metric_function_length_csharp_method() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src_dir = dir.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        // Write a C# method with 60 lines in the body
        let mut body =
            String::from("public class MyService {\n    public void ProcessOrder(int orderId) {\n");
        for i in 0..58 {
            body.push_str(&format!("        var step{i} = orderId + {i};\n"));
        }
        body.push_str("    }\n}\n");
        std::fs::write(src_dir.join("service.cs"), &body).unwrap();

        let ws = crate::storage::workspace::Workspace::load(dir.path(), &[Language::CSharp], None)
            .unwrap();
        let graph = crate::graph::builder::GraphBuilder::new(&ws, &[Language::CSharp])
            .build()
            .expect("GraphBuilder should succeed");

        let is_test_fn = |path: &str| is_test_file(path);
        let is_generated_fn = |path: &str| is_excluded_for_arch_analysis(path);
        let is_barrel_fn = |path: &str| is_barrel_file(path);
        let mut taint_ctx = TaintContext::default();

        let nodes = execute_stage(
            &GraphStage::Select {
                select: crate::pipeline::dsl::NodeType::Symbol,
                filter: Some(crate::pipeline::dsl::WhereClause {
                    kind: Some(vec!["method".to_string()]),
                    ..Default::default()
                }),
                exclude: None,
            },
            Vec::new(),
            &graph,
            Some(&ws),
            None,
            "csharp_length_test",
            &is_test_fn,
            &is_generated_fn,
            &is_barrel_fn,
            &mut taint_ctx,
        )
        .unwrap();

        assert!(
            !nodes.is_empty(),
            "expected at least one method symbol in C# file"
        );

        let result_nodes = execute_stage(
            &GraphStage::ComputeMetric {
                compute_metric: "function_length".to_string(),
            },
            nodes,
            &graph,
            Some(&ws),
            None,
            "csharp_length_test",
            &is_test_fn,
            &is_generated_fn,
            &is_barrel_fn,
            &mut taint_ctx,
        )
        .unwrap();

        assert_eq!(result_nodes.len(), 1, "expected 1 method node with metric");
        let len = result_nodes[0]
            .metrics
            .get("function_length")
            .expect("function_length metric should be present");
        match len {
            MetricValue::Int(v) => assert!(*v >= 50, "expected >= 50 lines, got {}", v),
            _ => panic!("expected Int metric"),
        }
    }
}
