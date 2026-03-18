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

pub mod n_plus_one_queries;
pub mod sync_blocking_in_async;
pub mod memory_leak_indicators;

pub mod module_size_distribution;
pub mod circular_dependencies;
pub mod dependency_graph_depth;
pub mod api_surface_area;

use anyhow::Result;
use crate::audit::pipeline::Pipeline;

pub fn tech_debt_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(bare_except::BareExceptPipeline::new()?),
        Box::new(mutable_default_args::MutableDefaultArgsPipeline::new()?),
        Box::new(magic_numbers::PythonMagicNumbersPipeline::new()?),
        Box::new(god_functions::GodFunctionsPipeline::new()?),
        Box::new(missing_type_hints::MissingTypeHintsPipeline::new()?),
        Box::new(stringly_typed::StringlyTypedPipeline::new()?),
        Box::new(deep_nesting::DeepNestingPipeline::new()?),
        Box::new(duplicate_logic::DuplicateLogicPipeline::new()?),
    ])
}

pub fn complexity_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(cyclomatic::CyclomaticComplexityPipeline::new()?),
        Box::new(function_length::FunctionLengthPipeline::new()?),
        Box::new(cognitive::CognitiveComplexityPipeline::new()?),
        Box::new(comment_ratio::CommentToCodeRatioPipeline::new()?),
    ])
}

pub fn code_style_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(dead_code::DeadCodePipeline::new()?),
        Box::new(duplicate_code::DuplicateCodePipeline::new()?),
        Box::new(coupling::CouplingPipeline::new()?),
    ])
}

pub fn security_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(command_injection::CommandInjectionPipeline::new()?),
        Box::new(code_injection::CodeInjectionPipeline::new()?),
        Box::new(sql_injection::SqlInjectionPipeline::new()?),
        Box::new(path_traversal::PathTraversalPipeline::new()?),
        Box::new(insecure_deserialization::InsecureDeserializationPipeline::new()?),
        Box::new(ssrf::SsrfPipeline::new()?),
        Box::new(resource_exhaustion::ResourceExhaustionPipeline::new()?),
        Box::new(xxe_format_string::XxeFormatStringPipeline::new()?),
    ])
}

pub fn scalability_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(n_plus_one_queries::NPlusOneQueriesPipeline::new()?),
        Box::new(sync_blocking_in_async::SyncBlockingInAsyncPipeline::new()?),
        Box::new(memory_leak_indicators::MemoryLeakIndicatorsPipeline::new()?),
    ])
}

pub fn architecture_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(module_size_distribution::ModuleSizeDistributionPipeline::new()?),
        Box::new(circular_dependencies::CircularDependenciesPipeline::new()?),
        Box::new(dependency_graph_depth::DependencyGraphDepthPipeline::new()?),
        Box::new(api_surface_area::ApiSurfaceAreaPipeline::new()?),
    ])
}
