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

pub mod memory_leak_indicators;
pub mod sync_blocking_in_async;

use crate::audit::pipeline::{AnyPipeline, Pipeline};
use anyhow::Result;

pub fn tech_debt_pipelines() -> Result<Vec<AnyPipeline>> {
    Ok(vec![
        AnyPipeline::Graph(Box::new(panic_detection::PanicDetectionPipeline::new()?)),
        AnyPipeline::Graph(Box::new(clone_detection::CloneDetectionPipeline::new()?)),
        AnyPipeline::Graph(Box::new(god_object_detection::GodObjectDetectionPipeline::new()?)),
        AnyPipeline::Graph(Box::new(stringly_typed::StringlyTypedPipeline::new()?)),
        AnyPipeline::Graph(Box::new(must_use_ignored::MustUseIgnoredPipeline::new()?)),
        AnyPipeline::Graph(Box::new(mutex_overuse::MutexOverusePipeline::new()?)),
        AnyPipeline::Graph(Box::new(pub_field_leakage::PubFieldLeakagePipeline::new()?)),
        AnyPipeline::Graph(Box::new(missing_trait_abstraction::MissingTraitAbstractionPipeline::new()?)),
        AnyPipeline::Graph(Box::new(async_blocking::AsyncBlockingPipeline::new()?)),
        AnyPipeline::Graph(Box::new(magic_numbers::MagicNumbersPipeline::new()?)),
    ])
}

pub fn complexity_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![])
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
        Box::new(sync_blocking_in_async::SyncBlockingInAsyncPipeline::new()?),
        Box::new(memory_leak_indicators::MemoryLeakIndicatorsPipeline::new()?),
    ])
}

