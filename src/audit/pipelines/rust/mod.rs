pub mod primitives;

pub mod async_blocking;
pub mod clone_detection;
pub mod god_object_detection;
pub mod magic_numbers;
pub mod missing_trait_abstraction;
pub mod must_use_ignored;
pub mod mutex_overuse;
pub mod panic_detection;
pub mod pub_field_leakage;
pub mod stringly_typed;

pub mod cognitive;
pub mod comment_ratio;
pub mod cyclomatic;
pub mod function_length;

pub mod coupling;
pub mod dead_code;
pub mod duplicate_code;

pub mod integer_overflow;
pub mod panic_dos;
pub mod path_traversal;
pub mod race_conditions;
pub mod resource_exhaustion;
pub mod toctou;
pub mod type_confusion;
pub mod unsafe_memory;

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
        Box::new(panic_detection::PanicDetectionPipeline::new()?),
        Box::new(clone_detection::CloneDetectionPipeline::new()?),
        Box::new(god_object_detection::GodObjectDetectionPipeline::new()?),
        Box::new(stringly_typed::StringlyTypedPipeline::new()?),
        Box::new(must_use_ignored::MustUseIgnoredPipeline::new()?),
        Box::new(mutex_overuse::MutexOverusePipeline::new()?),
        Box::new(pub_field_leakage::PubFieldLeakagePipeline::new()?),
        Box::new(missing_trait_abstraction::MissingTraitAbstractionPipeline::new()?),
        Box::new(async_blocking::AsyncBlockingPipeline::new()?),
        Box::new(magic_numbers::MagicNumbersPipeline::new()?),
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
        Box::new(integer_overflow::IntegerOverflowPipeline::new()?),
        Box::new(unsafe_memory::UnsafeMemoryPipeline::new()?),
        Box::new(race_conditions::RaceConditionsPipeline::new()?),
        Box::new(path_traversal::PathTraversalPipeline::new()?),
        Box::new(resource_exhaustion::ResourceExhaustionPipeline::new()?),
        Box::new(panic_dos::PanicDosPipeline::new()?),
        Box::new(type_confusion::TypeConfusionPipeline::new()?),
        Box::new(toctou::ToctouPipeline::new()?),
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
