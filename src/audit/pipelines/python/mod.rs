pub mod primitives;

pub mod bare_except;
pub mod deep_nesting;
pub mod duplicate_logic;
pub mod god_functions;
pub mod magic_numbers;
pub mod missing_type_hints;
pub mod mutable_default_args;
pub mod stringly_typed;

pub mod cognitive;
pub mod comment_ratio;
pub mod cyclomatic;
pub mod function_length;

pub mod coupling;
pub mod dead_code;
pub mod duplicate_code;

pub mod code_injection;
pub mod command_injection;
pub mod insecure_deserialization;
pub mod path_traversal;
pub mod resource_exhaustion;
pub mod sql_injection;
pub mod ssrf;
pub mod xxe_format_string;

pub mod memory_leak_indicators;
pub mod n_plus_one_queries;
pub mod sync_blocking_in_async;

pub mod api_surface_area;
pub mod module_size_distribution;

use crate::audit::pipeline::AnyPipeline;
use anyhow::Result;

pub fn tech_debt_pipelines() -> Result<Vec<AnyPipeline>> {
    Ok(vec![
        AnyPipeline::Graph(Box::new(bare_except::BareExceptPipeline::new()?)),
        AnyPipeline::Graph(Box::new(mutable_default_args::MutableDefaultArgsPipeline::new()?)),
        AnyPipeline::Graph(Box::new(magic_numbers::PythonMagicNumbersPipeline::new()?)),
        AnyPipeline::Graph(Box::new(god_functions::GodFunctionsPipeline::new()?)),
        AnyPipeline::Graph(Box::new(missing_type_hints::MissingTypeHintsPipeline::new()?)),
        AnyPipeline::Graph(Box::new(stringly_typed::StringlyTypedPipeline::new()?)),
        AnyPipeline::Graph(Box::new(deep_nesting::DeepNestingPipeline::new()?)),
        AnyPipeline::Graph(Box::new(duplicate_logic::DuplicateLogicPipeline::new()?)),
    ])
}

pub fn complexity_pipelines() -> Result<Vec<AnyPipeline>> {
    Ok(vec![
        AnyPipeline::Node(Box::new(cyclomatic::CyclomaticComplexityPipeline::new()?)),
        AnyPipeline::Node(Box::new(function_length::FunctionLengthPipeline::new()?)),
        AnyPipeline::Node(Box::new(cognitive::CognitiveComplexityPipeline::new()?)),
        AnyPipeline::Node(Box::new(comment_ratio::CommentToCodeRatioPipeline::new()?)),
    ])
}

pub fn code_style_pipelines() -> Result<Vec<AnyPipeline>> {
    Ok(vec![
        AnyPipeline::Graph(Box::new(dead_code::DeadCodePipeline::new()?)),
        AnyPipeline::Legacy(Box::new(duplicate_code::DuplicateCodePipeline::new()?)),
        AnyPipeline::Graph(Box::new(coupling::CouplingPipeline::new()?)),
    ])
}

pub fn security_pipelines() -> Result<Vec<AnyPipeline>> {
    Ok(vec![
        AnyPipeline::Legacy(Box::new(command_injection::CommandInjectionPipeline::new()?)),
        AnyPipeline::Legacy(Box::new(code_injection::CodeInjectionPipeline::new()?)),
        AnyPipeline::Graph(Box::new(sql_injection::SqlInjectionPipeline::new()?)),
        AnyPipeline::Graph(Box::new(path_traversal::PathTraversalPipeline::new()?)),
        AnyPipeline::Legacy(Box::new(insecure_deserialization::InsecureDeserializationPipeline::new()?)),
        AnyPipeline::Legacy(Box::new(ssrf::SsrfPipeline::new()?)),
        AnyPipeline::Legacy(Box::new(resource_exhaustion::ResourceExhaustionPipeline::new()?)),
        AnyPipeline::Legacy(Box::new(xxe_format_string::XxeFormatStringPipeline::new()?)),
    ])
}

pub fn scalability_pipelines() -> Result<Vec<AnyPipeline>> {
    Ok(vec![
        AnyPipeline::Graph(Box::new(n_plus_one_queries::NPlusOneQueriesPipeline::new()?)),
        AnyPipeline::Legacy(Box::new(sync_blocking_in_async::SyncBlockingInAsyncPipeline::new()?)),
        AnyPipeline::Graph(Box::new(memory_leak_indicators::MemoryLeakIndicatorsPipeline::new()?)),
    ])
}

pub fn architecture_pipelines() -> Result<Vec<AnyPipeline>> {
    Ok(vec![
        AnyPipeline::Graph(Box::new(module_size_distribution::ModuleSizeDistributionPipeline::new()?)),
        AnyPipeline::Graph(Box::new(api_surface_area::ApiSurfaceAreaPipeline::new()?)),
    ])
}
