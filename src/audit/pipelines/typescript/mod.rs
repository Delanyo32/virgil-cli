pub mod primitives;

pub mod any_escape_hatch;
pub mod enum_usage;
pub mod implicit_any;
pub mod leaking_impl_types;
pub mod mutable_types;
pub mod optional_everything;
pub mod record_string_any;
pub mod type_assertions;
pub mod type_duplication;
pub mod unchecked_index_access;
pub mod unconstrained_generics;

pub mod cognitive;
pub mod comment_ratio;
pub mod cyclomatic;
pub mod function_length;

pub mod coupling;
pub mod dead_code;
pub mod duplicate_code;

pub mod type_system_bypass;
pub mod unsafe_type_assertions_security;

pub mod n_plus_one_queries;
pub mod sync_blocking_in_async;
pub mod memory_leak_indicators;

pub mod module_size_distribution;
pub mod circular_dependencies;
pub mod dependency_graph_depth;
pub mod api_surface_area;

use anyhow::Result;
use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines;
use crate::language::Language;

pub fn tech_debt_pipelines(language: Language) -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(any_escape_hatch::AnyEscapeHatchPipeline::new(language)?),
        Box::new(type_assertions::TypeAssertionsPipeline::new(language)?),
        Box::new(optional_everything::OptionalEverythingPipeline::new(language)?),
        Box::new(type_duplication::TypeDuplicationPipeline::new(language)?),
        Box::new(record_string_any::RecordStringAnyPipeline::new(language)?),
        Box::new(enum_usage::EnumUsagePipeline::new(language)?),
        Box::new(implicit_any::ImplicitAnyPipeline::new(language)?),
        Box::new(unchecked_index_access::UncheckedIndexAccessPipeline::new(language)?),
        Box::new(mutable_types::MutableTypesPipeline::new(language)?),
        Box::new(unconstrained_generics::UnconstrainedGenericsPipeline::new(language)?),
        Box::new(leaking_impl_types::LeakingImplTypesPipeline::new(language)?),
    ])
}

pub fn complexity_pipelines(language: Language) -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(cyclomatic::CyclomaticComplexityPipeline::new(language)?),
        Box::new(function_length::FunctionLengthPipeline::new(language)?),
        Box::new(cognitive::CognitiveComplexityPipeline::new(language)?),
        Box::new(comment_ratio::CommentToCodeRatioPipeline::new(language)?),
    ])
}

pub fn code_style_pipelines(language: Language) -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(dead_code::DeadCodePipeline::new(language)?),
        Box::new(duplicate_code::DuplicateCodePipeline::new(language)?),
        Box::new(coupling::CouplingPipeline::new(language)?),
    ])
}

pub fn security_pipelines(language: Language) -> Result<Vec<Box<dyn Pipeline>>> {
    // Start with all 9 shared JS/TS security pipelines
    let mut pipes = pipelines::javascript::security_pipelines(language)?;
    // Add 2 TypeScript-specific security pipelines
    pipes.push(Box::new(type_system_bypass::TypeSystemBypassPipeline::new(language)?));
    pipes.push(Box::new(unsafe_type_assertions_security::UnsafeTypeAssertionsSecurityPipeline::new(language)?));
    Ok(pipes)
}

pub fn scalability_pipelines(language: Language) -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(n_plus_one_queries::NPlusOneQueriesPipeline::new(language)?),
        Box::new(sync_blocking_in_async::SyncBlockingInAsyncPipeline::new(language)?),
        Box::new(memory_leak_indicators::MemoryLeakIndicatorsPipeline::new(language)?),
    ])
}

pub fn architecture_pipelines(language: Language) -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(module_size_distribution::ModuleSizeDistributionPipeline::new(language)?),
        Box::new(circular_dependencies::CircularDependenciesPipeline::new(language)?),
        Box::new(dependency_graph_depth::DependencyGraphDepthPipeline::new(language)?),
        Box::new(api_surface_area::ApiSurfaceAreaPipeline::new(language)?),
    ])
}
