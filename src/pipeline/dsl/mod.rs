//! JSON audit pipeline DSL.
//!
//! A pipeline is a `Vec<GraphStage>`. Stages compose left-to-right:
//! `select` → `compute_metric` / `taint_sources` / `taint_sanitizers` / `taint_sinks` → `flag`.
//! Each stage reads from and writes to a shared `Vec<PipelineNode>` carried through the run.

mod stages;
mod types;
mod where_clause;

pub use stages::{
    CountConfig, DenominatorConfig, FindCyclesConfig, FindDuplicatesStage, FlagConfig, GraphStage,
    MaxDepthConfig, NumeratorConfig, RatioConfig, TaintStage,
};
pub use types::{
    EdgeDirection, EdgeType, MetricValue, NodeType, NumericPredicate, PipelineNode, SeverityEntry,
    interpolate_message,
};
pub use where_clause::WhereClause;

// Re-export pattern types for ergonomic JSON-shape access. The structs themselves
// live in `crate::graph::taint` (the engine that consumes them).
pub use crate::graph::taint::{TaintSanitizerPattern, TaintSinkPattern, TaintSourcePattern};

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use petgraph::graph::NodeIndex;

    use super::*;

    fn make_node(metrics: Vec<(&str, MetricValue)>) -> PipelineNode {
        let mut m = HashMap::new();
        for (k, v) in metrics {
            m.insert(k.to_string(), v);
        }
        PipelineNode {
            node_idx: NodeIndex::new(0),
            file_path: "src/foo.rs".to_string(),
            name: "my_func".to_string(),
            kind: "function".to_string(),
            line: 42,
            exported: true,
            language: "rust".to_string(),
            metrics: m,
        }
    }

    // -----------------------------------------------------------------------
    // NumericPredicate::matches
    // -----------------------------------------------------------------------

    #[test]
    fn test_numeric_predicate_gte() {
        let pred = NumericPredicate {
            gte: Some(5.0),
            ..Default::default()
        };
        assert!(pred.matches(5.0));
        assert!(pred.matches(10.0));
        assert!(!pred.matches(4.9));
    }

    #[test]
    fn test_numeric_predicate_lte() {
        let pred = NumericPredicate {
            lte: Some(10.0),
            ..Default::default()
        };
        assert!(pred.matches(10.0));
        assert!(pred.matches(0.0));
        assert!(!pred.matches(10.1));
    }

    #[test]
    fn test_numeric_predicate_gt() {
        let pred = NumericPredicate {
            gt: Some(5.0),
            ..Default::default()
        };
        assert!(pred.matches(5.1));
        assert!(!pred.matches(5.0));
        assert!(!pred.matches(4.0));
    }

    #[test]
    fn test_numeric_predicate_lt() {
        let pred = NumericPredicate {
            lt: Some(5.0),
            ..Default::default()
        };
        assert!(pred.matches(4.9));
        assert!(!pred.matches(5.0));
        assert!(!pred.matches(6.0));
    }

    #[test]
    fn test_numeric_predicate_eq() {
        let pred = NumericPredicate {
            eq: Some(7.0),
            ..Default::default()
        };
        assert!(pred.matches(7.0));
        assert!(!pred.matches(7.1));
        assert!(!pred.matches(6.9));
    }

    #[test]
    fn test_numeric_predicate_compound_range() {
        // 5 <= value <= 10
        let pred = NumericPredicate {
            gte: Some(5.0),
            lte: Some(10.0),
            ..Default::default()
        };
        assert!(pred.matches(5.0));
        assert!(pred.matches(7.5));
        assert!(pred.matches(10.0));
        assert!(!pred.matches(4.9));
        assert!(!pred.matches(10.1));
    }

    #[test]
    fn test_numeric_predicate_empty_always_true() {
        let pred = NumericPredicate::default();
        assert!(pred.matches(0.0));
        assert!(pred.matches(f64::MAX));
        assert!(pred.matches(f64::MIN));
    }

    // -----------------------------------------------------------------------
    // WhereClause::is_empty
    // -----------------------------------------------------------------------

    #[test]
    fn test_where_clause_is_empty_default() {
        let wc = WhereClause::default();
        assert!(wc.is_empty());
    }

    #[test]
    fn test_where_clause_not_empty_exported() {
        let wc = WhereClause {
            exported: Some(true),
            ..Default::default()
        };
        assert!(!wc.is_empty());
    }

    #[test]
    fn test_where_clause_not_empty_count() {
        let mut metrics = HashMap::new();
        metrics.insert(
            "count".to_string(),
            NumericPredicate {
                gte: Some(1.0),
                ..Default::default()
            },
        );
        let wc = WhereClause {
            metrics,
            ..Default::default()
        };
        assert!(!wc.is_empty());
    }

    #[test]
    fn test_where_clause_not_empty_is_test_file() {
        let wc = WhereClause {
            is_test_file: Some(false),
            ..Default::default()
        };
        assert!(!wc.is_empty());
    }

    #[test]
    fn test_where_clause_not_empty_and() {
        let wc = WhereClause {
            and: Some(vec![WhereClause::default()]),
            ..Default::default()
        };
        assert!(!wc.is_empty());
    }

    // -----------------------------------------------------------------------
    // WhereClause::eval_metrics
    // -----------------------------------------------------------------------

    #[test]
    fn test_eval_metrics_count_threshold() {
        let node = make_node(vec![("count", MetricValue::Int(15))]);

        let mut m = HashMap::new();
        m.insert(
            "count".to_string(),
            NumericPredicate {
                gte: Some(10.0),
                ..Default::default()
            },
        );
        let wc_pass = WhereClause {
            metrics: m,
            ..Default::default()
        };
        assert!(wc_pass.eval_metrics(&node));

        let mut m2 = HashMap::new();
        m2.insert(
            "count".to_string(),
            NumericPredicate {
                gte: Some(20.0),
                ..Default::default()
            },
        );
        let wc_fail = WhereClause {
            metrics: m2,
            ..Default::default()
        };
        assert!(!wc_fail.eval_metrics(&node));
    }

    #[test]
    fn test_eval_metrics_cycle_size() {
        let node = make_node(vec![("cycle_size", MetricValue::Int(3))]);

        let mut m = HashMap::new();
        m.insert(
            "cycle_size".to_string(),
            NumericPredicate {
                gte: Some(3.0),
                ..Default::default()
            },
        );
        let wc = WhereClause {
            metrics: m,
            ..Default::default()
        };
        assert!(wc.eval_metrics(&node));

        let mut m2 = HashMap::new();
        m2.insert(
            "cycle_size".to_string(),
            NumericPredicate {
                gt: Some(3.0),
                ..Default::default()
            },
        );
        let wc_fail = WhereClause {
            metrics: m2,
            ..Default::default()
        };
        assert!(!wc_fail.eval_metrics(&node));
    }

    #[test]
    fn test_eval_metrics_exported() {
        let node_exported = make_node(vec![]);
        assert!(node_exported.exported);

        let wc_exported = WhereClause {
            exported: Some(true),
            ..Default::default()
        };
        assert!(wc_exported.eval_metrics(&node_exported));

        let wc_not_exported = WhereClause {
            exported: Some(false),
            ..Default::default()
        };
        assert!(!wc_not_exported.eval_metrics(&node_exported));
    }

    #[test]
    fn test_eval_metrics_and_operator() {
        let node = make_node(vec![
            ("count", MetricValue::Int(10)),
            ("depth", MetricValue::Int(5)),
        ]);

        let mut m1 = HashMap::new();
        m1.insert(
            "count".to_string(),
            NumericPredicate {
                gte: Some(5.0),
                ..Default::default()
            },
        );
        let mut m2 = HashMap::new();
        m2.insert(
            "depth".to_string(),
            NumericPredicate {
                lte: Some(10.0),
                ..Default::default()
            },
        );
        let wc = WhereClause {
            and: Some(vec![
                WhereClause {
                    metrics: m1,
                    ..Default::default()
                },
                WhereClause {
                    metrics: m2,
                    ..Default::default()
                },
            ]),
            ..Default::default()
        };
        assert!(wc.eval_metrics(&node));
    }

    #[test]
    fn test_eval_metrics_or_operator() {
        let node = make_node(vec![("count", MetricValue::Int(2))]);

        let mut m1 = HashMap::new();
        m1.insert(
            "count".to_string(),
            NumericPredicate {
                gte: Some(100.0),
                ..Default::default()
            },
        );
        let mut m2 = HashMap::new();
        m2.insert(
            "count".to_string(),
            NumericPredicate {
                lte: Some(5.0),
                ..Default::default()
            },
        );
        let wc = WhereClause {
            or: Some(vec![
                WhereClause {
                    metrics: m1,
                    ..Default::default()
                },
                WhereClause {
                    metrics: m2,
                    ..Default::default()
                },
            ]),
            ..Default::default()
        };
        assert!(wc.eval_metrics(&node));
    }

    #[test]
    fn test_eval_metrics_not_operator() {
        let node = make_node(vec![("count", MetricValue::Int(2))]);

        let mut m = HashMap::new();
        m.insert(
            "count".to_string(),
            NumericPredicate {
                gte: Some(100.0),
                ..Default::default()
            },
        );
        let wc = WhereClause {
            not: Some(Box::new(WhereClause {
                metrics: m,
                ..Default::default()
            })),
            ..Default::default()
        };
        // count=2, NOT(count >= 100) => true
        assert!(wc.eval_metrics(&node));
    }

    // -----------------------------------------------------------------------
    // FlagConfig::resolve_severity
    // -----------------------------------------------------------------------

    #[test]
    fn test_resolve_severity_fallback_default() {
        let flag = FlagConfig {
            pattern: "test".to_string(),
            message: "msg".to_string(),
            severity: None,
            severity_map: None,
            pipeline_name: None,
        };
        let node = make_node(vec![]);
        assert_eq!(flag.resolve_severity(&node), Some("warning".to_string()));
    }

    #[test]
    fn test_resolve_severity_uses_severity_field() {
        let flag = FlagConfig {
            pattern: "test".to_string(),
            message: "msg".to_string(),
            severity: Some("error".to_string()),
            severity_map: None,
            pipeline_name: None,
        };
        let node = make_node(vec![]);
        assert_eq!(flag.resolve_severity(&node), Some("error".to_string()));
    }

    #[test]
    fn test_resolve_severity_map_first_match_wins() {
        let node = make_node(vec![("count", MetricValue::Int(25))]);

        let mut m1 = HashMap::new();
        m1.insert(
            "count".to_string(),
            NumericPredicate {
                gte: Some(20.0),
                ..Default::default()
            },
        );
        let mut m2 = HashMap::new();
        m2.insert(
            "count".to_string(),
            NumericPredicate {
                gte: Some(10.0),
                ..Default::default()
            },
        );
        let flag = FlagConfig {
            pattern: "test".to_string(),
            message: "msg".to_string(),
            severity: Some("info".to_string()),
            severity_map: Some(vec![
                SeverityEntry {
                    when: Some(WhereClause {
                        metrics: m1,
                        ..Default::default()
                    }),
                    severity: "error".to_string(),
                },
                SeverityEntry {
                    when: Some(WhereClause {
                        metrics: m2,
                        ..Default::default()
                    }),
                    severity: "warning".to_string(),
                },
            ]),
            pipeline_name: None,
        };
        // count=25 >= 20 matches first entry
        assert_eq!(flag.resolve_severity(&node), Some("error".to_string()));
    }

    #[test]
    fn test_resolve_severity_map_fallthrough_to_second() {
        let node = make_node(vec![("count", MetricValue::Int(12))]);

        let mut m1 = HashMap::new();
        m1.insert(
            "count".to_string(),
            NumericPredicate {
                gte: Some(20.0),
                ..Default::default()
            },
        );
        let mut m2 = HashMap::new();
        m2.insert(
            "count".to_string(),
            NumericPredicate {
                gte: Some(10.0),
                ..Default::default()
            },
        );
        let flag = FlagConfig {
            pattern: "test".to_string(),
            message: "msg".to_string(),
            severity: Some("info".to_string()),
            severity_map: Some(vec![
                SeverityEntry {
                    when: Some(WhereClause {
                        metrics: m1,
                        ..Default::default()
                    }),
                    severity: "error".to_string(),
                },
                SeverityEntry {
                    when: Some(WhereClause {
                        metrics: m2,
                        ..Default::default()
                    }),
                    severity: "warning".to_string(),
                },
            ]),
            pipeline_name: None,
        };
        // count=12 does NOT match first (>= 20), does match second (>= 10)
        assert_eq!(flag.resolve_severity(&node), Some("warning".to_string()));
    }

    #[test]
    fn test_resolve_severity_map_none_when_is_default() {
        // An entry with when=None acts as catch-all
        let node = make_node(vec![("count", MetricValue::Int(0))]);
        let mut m = HashMap::new();
        m.insert(
            "count".to_string(),
            NumericPredicate {
                gte: Some(100.0),
                ..Default::default()
            },
        );
        let flag = FlagConfig {
            pattern: "test".to_string(),
            message: "msg".to_string(),
            severity: Some("info".to_string()),
            severity_map: Some(vec![
                SeverityEntry {
                    when: Some(WhereClause {
                        metrics: m,
                        ..Default::default()
                    }),
                    severity: "error".to_string(),
                },
                SeverityEntry {
                    when: None, // catch-all
                    severity: "hint".to_string(),
                },
            ]),
            pipeline_name: None,
        };
        // count=0 does not match first, second has no condition => "hint"
        assert_eq!(flag.resolve_severity(&node), Some("hint".to_string()));
    }

    #[test]
    fn test_resolve_severity_map_no_match_falls_back_to_severity() {
        let node = make_node(vec![("count", MetricValue::Int(0))]);
        let mut m = HashMap::new();
        m.insert(
            "count".to_string(),
            NumericPredicate {
                gte: Some(100.0),
                ..Default::default()
            },
        );
        let flag = FlagConfig {
            pattern: "test".to_string(),
            message: "msg".to_string(),
            severity: Some("info".to_string()),
            severity_map: Some(vec![SeverityEntry {
                when: Some(WhereClause {
                    metrics: m,
                    ..Default::default()
                }),
                severity: "error".to_string(),
            }]),
            pipeline_name: None,
        };
        // Nothing matches, falls back to severity field
        assert_eq!(flag.resolve_severity(&node), Some("info".to_string()));
    }

    #[test]
    fn test_resolve_severity_map_no_match_no_severity_suppresses() {
        // severity_map has only conditional entries, none match, no bare severity field
        // -> returns None (suppression)
        let node = make_node(vec![("cyclomatic_complexity", MetricValue::Int(3))]);
        let mut m1 = HashMap::new();
        m1.insert(
            "cyclomatic_complexity".to_string(),
            NumericPredicate {
                gte: Some(20.0),
                ..Default::default()
            },
        );
        let mut m2 = HashMap::new();
        m2.insert(
            "cyclomatic_complexity".to_string(),
            NumericPredicate {
                gt: Some(10.0),
                ..Default::default()
            },
        );
        let flag = FlagConfig {
            pattern: "test".to_string(),
            message: "msg".to_string(),
            severity: None, // NO bare severity field
            severity_map: Some(vec![
                SeverityEntry {
                    when: Some(WhereClause {
                        metrics: m1,
                        ..Default::default()
                    }),
                    severity: "error".to_string(),
                },
                SeverityEntry {
                    when: Some(WhereClause {
                        metrics: m2,
                        ..Default::default()
                    }),
                    severity: "warning".to_string(),
                },
            ]),
            pipeline_name: None,
        };
        // CC=3: neither >= 20 nor > 10, and no bare severity => suppressed
        assert_eq!(flag.resolve_severity(&node), None);
    }

    // -----------------------------------------------------------------------
    // interpolate_message
    // -----------------------------------------------------------------------

    #[test]
    fn test_interpolate_basic_fields() {
        let node = make_node(vec![]);
        let result = interpolate_message("{{name}} at {{file}}:{{line}}", &node);
        assert_eq!(result, "my_func at src/foo.rs:42");
    }

    #[test]
    fn test_interpolate_kind_and_language() {
        let node = make_node(vec![]);
        let result = interpolate_message("{{kind}} in {{language}}", &node);
        assert_eq!(result, "function in rust");
    }

    #[test]
    fn test_interpolate_metric_int() {
        let node = make_node(vec![("count", MetricValue::Int(7))]);
        let result = interpolate_message("count={{count}}", &node);
        assert_eq!(result, "count=7");
    }

    #[test]
    fn test_interpolate_metric_float() {
        let node = make_node(vec![("ratio", MetricValue::Float(0.75))]);
        let result = interpolate_message("ratio={{ratio}}", &node);
        assert_eq!(result, "ratio=0.75");
    }

    #[test]
    fn test_interpolate_metric_text() {
        let node = make_node(vec![(
            "cycle_path",
            MetricValue::Text("a->b->a".to_string()),
        )]);
        let result = interpolate_message("path={{cycle_path}}", &node);
        assert_eq!(result, "path=a->b->a");
    }

    #[test]
    fn test_interpolate_no_placeholders() {
        let node = make_node(vec![]);
        let result = interpolate_message("no placeholders here", &node);
        assert_eq!(result, "no placeholders here");
    }

    // -----------------------------------------------------------------------
    // GraphStage deserialization (untagged enum)
    // -----------------------------------------------------------------------

    #[test]
    fn test_deserialize_select_stage() {
        let json = r#"{"select": "symbol"}"#;
        let stage: GraphStage = serde_json::from_str(json).unwrap();
        match stage {
            GraphStage::Select {
                select,
                filter,
                exclude,
            } => {
                assert_eq!(select, NodeType::Symbol);
                assert!(filter.is_none());
                assert!(exclude.is_none());
            }
            _ => panic!("expected Select stage"),
        }
    }

    #[test]
    fn test_deserialize_select_with_where() {
        let json = r#"{"select": "file", "where": {"exported": true}}"#;
        let stage: GraphStage = serde_json::from_str(json).unwrap();
        match stage {
            GraphStage::Select { select, filter, .. } => {
                assert_eq!(select, NodeType::File);
                let wc = filter.unwrap();
                assert_eq!(wc.exported, Some(true));
            }
            _ => panic!("expected Select stage"),
        }
    }

    #[test]
    fn test_deserialize_find_cycles_stage() {
        let json = r#"{"find_cycles": {"edge": "calls"}}"#;
        let stage: GraphStage = serde_json::from_str(json).unwrap();
        match stage {
            GraphStage::FindCycles { find_cycles } => {
                assert_eq!(find_cycles.edge, EdgeType::Calls);
            }
            _ => panic!("expected FindCycles stage"),
        }
    }

    #[test]
    fn test_deserialize_flag_stage() {
        let json = r#"{
            "flag": {
                "pattern": "oversized_module",
                "message": "{{name}} has {{count}} symbols",
                "severity": "warning"
            }
        }"#;
        let stage: GraphStage = serde_json::from_str(json).unwrap();
        match stage {
            GraphStage::Flag { flag } => {
                assert_eq!(flag.pattern, "oversized_module");
                assert_eq!(flag.severity, Some("warning".to_string()));
                assert!(flag.severity_map.is_none());
            }
            _ => panic!("expected Flag stage"),
        }
    }

    #[test]
    fn test_deserialize_group_by_stage() {
        let json = r#"{"group_by": "file_path"}"#;
        let stage: GraphStage = serde_json::from_str(json).unwrap();
        match stage {
            GraphStage::GroupBy { group_by } => {
                assert_eq!(group_by, "file_path");
            }
            _ => panic!("expected GroupBy stage"),
        }
    }

    #[test]
    fn test_deserialize_match_pattern_stage() {
        let json = r#"{"match_pattern": "(identifier) @name"}"#;
        let stage: GraphStage = serde_json::from_str(json).unwrap();
        match stage {
            GraphStage::MatchPattern {
                match_pattern,
                when,
            } => {
                assert_eq!(match_pattern, "(identifier) @name");
                assert!(when.is_none());
            }
            _ => panic!("expected MatchPattern stage"),
        }
    }

    #[test]
    fn test_deserialize_match_pattern_with_when_lhs_is_parameter() {
        let json = r#"{"match_pattern": "(identifier) @name", "when": {"lhs_is_parameter": true}}"#;
        let stage: GraphStage = serde_json::from_str(json).unwrap();
        match stage {
            GraphStage::MatchPattern {
                match_pattern,
                when,
            } => {
                assert_eq!(match_pattern, "(identifier) @name");
                let wc = when.expect("when should be present");
                assert_eq!(wc.lhs_is_parameter, Some(true));
            }
            _ => panic!("expected MatchPattern stage"),
        }
    }

    #[test]
    fn test_deserialize_compute_metric_stage() {
        let json = r#"{"compute_metric": "cyclomatic_complexity"}"#;
        let stage: GraphStage = serde_json::from_str(json).unwrap();
        match stage {
            GraphStage::ComputeMetric { compute_metric } => {
                assert_eq!(compute_metric, "cyclomatic_complexity");
            }
            _ => panic!("expected ComputeMetric stage"),
        }
    }

    #[test]
    fn test_deserialize_flag_with_severity_map() {
        let json = r#"{
            "flag": {
                "pattern": "hub_module",
                "message": "Hub module {{name}}",
                "severity_map": [
                    {"when": {"metrics": {"count": {"gte": 20}}}, "severity": "error"},
                    {"when": {"metrics": {"count": {"gte": 10}}}, "severity": "warning"},
                    {"severity": "info"}
                ]
            }
        }"#;
        let stage: GraphStage = serde_json::from_str(json).unwrap();
        match stage {
            GraphStage::Flag { flag } => {
                let map = flag.severity_map.unwrap();
                assert_eq!(map.len(), 3);
                assert_eq!(map[0].severity, "error");
                assert_eq!(map[1].severity, "warning");
                assert_eq!(map[2].severity, "info");
                assert!(map[2].when.is_none());
            }
            _ => panic!("expected Flag stage"),
        }
    }

    #[test]
    fn test_node_type_roundtrip() {
        let variants = vec![NodeType::File, NodeType::Symbol, NodeType::CallSite];
        for v in variants {
            let json = serde_json::to_string(&v).unwrap();
            let back: NodeType = serde_json::from_str(&json).unwrap();
            assert_eq!(v, back);
        }
    }

    #[test]
    fn test_edge_type_roundtrip() {
        let variants = vec![
            EdgeType::Calls,
            EdgeType::Imports,
            EdgeType::FlowsTo,
            EdgeType::Acquires,
            EdgeType::ReleasedBy,
            EdgeType::Contains,
            EdgeType::Exports,
            EdgeType::DefinedIn,
        ];
        for v in variants {
            let json = serde_json::to_string(&v).unwrap();
            let back: EdgeType = serde_json::from_str(&json).unwrap();
            assert_eq!(v, back);
        }
    }

    #[test]
    fn test_edge_direction_default_is_out() {
        assert_eq!(EdgeDirection::default(), EdgeDirection::Out);
    }

    #[test]
    fn where_clause_metric_predicates_cyclomatic() {
        let json = r#"{"metrics": {"cyclomatic_complexity": {"gt": 10}}}"#;
        let wc: WhereClause = serde_json::from_str(json).unwrap();
        assert!(wc.metrics.contains_key("cyclomatic_complexity"));

        let mut node = PipelineNode {
            node_idx: petgraph::graph::NodeIndex::new(0),
            file_path: "test.rs".to_string(),
            name: "foo".to_string(),
            kind: "function".to_string(),
            line: 1,
            exported: false,
            language: "rust".to_string(),
            metrics: std::collections::HashMap::new(),
        };
        // CC = 15 > 10 -- should pass
        node.metrics
            .insert("cyclomatic_complexity".to_string(), MetricValue::Int(15));
        assert!(wc.eval_metrics(&node));

        // CC = 5 <= 10 -- should fail
        node.metrics
            .insert("cyclomatic_complexity".to_string(), MetricValue::Int(5));
        assert!(!wc.eval_metrics(&node));
    }

    #[test]
    fn test_where_clause_generic_metrics_deserialization() {
        let json =
            r#"{"metrics": {"cyclomatic_complexity": {"gte": 10}, "function_length": {"gt": 50}}}"#;
        let wc: WhereClause = serde_json::from_str(json).unwrap();
        assert!(!wc.metrics.is_empty());
        assert!(wc.metrics.contains_key("cyclomatic_complexity"));
        assert!(wc.metrics.contains_key("function_length"));
    }

    #[test]
    fn test_where_clause_generic_metrics_eval() {
        let node_pass = make_node(vec![
            ("cyclomatic_complexity", MetricValue::Int(15)),
            ("function_length", MetricValue::Int(60)),
        ]);
        let node_fail = make_node(vec![
            ("cyclomatic_complexity", MetricValue::Int(5)),
            ("function_length", MetricValue::Int(60)),
        ]);
        let json = r#"{"metrics": {"cyclomatic_complexity": {"gte": 10}}}"#;
        let wc: WhereClause = serde_json::from_str(json).unwrap();
        assert!(wc.eval_metrics(&node_pass));
        assert!(!wc.eval_metrics(&node_fail));
    }

    #[test]
    fn test_where_clause_generic_metrics_is_not_empty() {
        let json = r#"{"metrics": {"efferent_coupling": {"gte": 8}}}"#;
        let wc: WhereClause = serde_json::from_str(json).unwrap();
        assert!(wc.metrics.contains_key("efferent_coupling"));
        assert!(!wc.is_empty());
    }

    #[test]
    fn test_taint_sources_stage_deserializes() {
        let json = r#"{"taint_sources": [{"pattern": "request.form", "kind": "user_input"}]}"#;
        let stage: GraphStage = serde_json::from_str(json).unwrap();
        match stage {
            GraphStage::TaintSources { taint_sources } => {
                assert_eq!(taint_sources.len(), 1);
                assert_eq!(taint_sources[0].pattern, "request.form");
                assert_eq!(taint_sources[0].kind, "user_input");
            }
            _ => panic!("expected TaintSources stage"),
        }
    }

    #[test]
    fn test_taint_sanitizers_stage_deserializes() {
        let json = r#"{"taint_sanitizers": [{"pattern": "escape"}, {"pattern": "quote"}]}"#;
        let stage: GraphStage = serde_json::from_str(json).unwrap();
        match stage {
            GraphStage::TaintSanitizers { taint_sanitizers } => {
                assert_eq!(taint_sanitizers.len(), 2);
            }
            _ => panic!("expected TaintSanitizers stage"),
        }
    }

    #[test]
    fn test_taint_sinks_stage_deserializes() {
        let json =
            r#"{"taint_sinks": [{"pattern": "cursor.execute", "vulnerability": "sql_injection"}]}"#;
        let stage: GraphStage = serde_json::from_str(json).unwrap();
        match stage {
            GraphStage::TaintSinks { taint_sinks } => {
                assert_eq!(taint_sinks.len(), 1);
                assert_eq!(taint_sinks[0].vulnerability, "sql_injection");
            }
            _ => panic!("expected TaintSinks stage"),
        }
    }

    #[test]
    fn test_decomposed_taint_pipeline_deserializes() {
        let json = r#"[
            {"taint_sources": [{"pattern": "request.form", "kind": "user_input"}]},
            {"taint_sanitizers": [{"pattern": "escape"}]},
            {"taint_sinks": [{"pattern": "cursor.execute", "vulnerability": "sql_injection"}]},
            {"flag": {"pattern": "sql_injection", "message": "found at {{file}}:{{line}}", "severity": "error"}}
        ]"#;
        let stages: Vec<GraphStage> = serde_json::from_str(json).unwrap();
        assert_eq!(stages.len(), 4);
    }

    #[test]
    fn where_clause_kind_filter() {
        let json = r#"{"kind": ["function", "method"]}"#;
        let wc: WhereClause = serde_json::from_str(json).unwrap();
        assert!(wc.kind.is_some());

        let node_fn = PipelineNode {
            node_idx: petgraph::graph::NodeIndex::new(0),
            file_path: "test.rs".to_string(),
            name: "foo".to_string(),
            kind: "function".to_string(),
            line: 1,
            exported: false,
            language: "rust".to_string(),
            metrics: std::collections::HashMap::new(),
        };
        let is_test = |_: &str| false;
        let is_gen = |_: &str| false;
        let is_barrel = |_: &str| false;
        assert!(wc.eval(&node_fn, &is_test, &is_gen, &is_barrel));

        let node_class = PipelineNode {
            node_idx: petgraph::graph::NodeIndex::new(0),
            file_path: "test.rs".to_string(),
            name: "Foo".to_string(),
            kind: "class".to_string(),
            line: 1,
            exported: false,
            language: "rust".to_string(),
            metrics: std::collections::HashMap::new(),
        };
        assert!(!wc.eval(&node_class, &is_test, &is_gen, &is_barrel));
    }
}
