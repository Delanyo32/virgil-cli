pub mod primitives;

pub mod c_style_cast;
pub mod endl_flush;
pub mod exception_across_boundary;
pub mod excessive_includes;
pub mod large_object_by_value;
pub mod magic_numbers;
pub mod missing_override;
pub mod raw_memory_management;
pub mod raw_union;
pub mod rule_of_five;
pub mod shared_ptr_cycle_risk;
pub mod uninitialized_member;

pub mod coupling;
pub mod dead_code;
pub mod duplicate_code;

pub mod cpp_buffer_overflow;
pub mod cpp_exception_safety;
pub mod cpp_injection;
pub mod cpp_integer_overflow;
pub mod cpp_memory_mismanagement;
pub mod cpp_path_traversal;
pub mod cpp_race_conditions;
pub mod cpp_type_confusion;
pub mod cpp_weak_randomness;

pub mod memory_leak_indicators;
pub mod sync_blocking_in_async;

use crate::audit::pipeline::{AnyPipeline, Pipeline};
use anyhow::Result;

pub fn tech_debt_pipelines() -> Result<Vec<AnyPipeline>> {
    Ok(vec![
        AnyPipeline::Graph(Box::new(raw_memory_management::RawMemoryManagementPipeline::new()?)),
        AnyPipeline::Graph(Box::new(rule_of_five::RuleOfFivePipeline::new()?)),
        AnyPipeline::Node(Box::new(c_style_cast::CStyleCastPipeline::new()?)),
        AnyPipeline::Node(Box::new(large_object_by_value::LargeObjectByValuePipeline::new()?)),
        AnyPipeline::Graph(Box::new(endl_flush::EndlFlushPipeline::new()?)),
        AnyPipeline::Graph(Box::new(missing_override::MissingOverridePipeline::new()?)),
        AnyPipeline::Node(Box::new(raw_union::RawUnionPipeline::new()?)),
        AnyPipeline::Graph(Box::new(excessive_includes::ExcessiveIncludesPipeline::new()?)),
        AnyPipeline::Graph(Box::new(exception_across_boundary::ExceptionAcrossBoundaryPipeline::new()?)),
        AnyPipeline::Graph(Box::new(uninitialized_member::UninitializedMemberPipeline::new()?)),
        AnyPipeline::Graph(Box::new(shared_ptr_cycle_risk::SharedPtrCycleRiskPipeline::new()?)),
        AnyPipeline::Node(Box::new(magic_numbers::CppMagicNumbersPipeline::new()?)),
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
        Box::new(cpp_injection::CppInjectionPipeline::new()?),
        Box::new(cpp_weak_randomness::CppWeakRandomnessPipeline::new()?),
        Box::new(cpp_type_confusion::CppTypeConfusionPipeline::new()?),
        Box::new(cpp_buffer_overflow::CppBufferOverflowPipeline::new()?),
        Box::new(cpp_integer_overflow::CppIntegerOverflowPipeline::new()?),
        Box::new(cpp_exception_safety::CppExceptionSafetyPipeline::new()?),
        Box::new(cpp_memory_mismanagement::CppMemoryMismanagementPipeline::new()?),
        Box::new(cpp_race_conditions::CppRaceConditionsPipeline::new()?),
        Box::new(cpp_path_traversal::CppPathTraversalPipeline::new()?),
    ])
}

pub fn scalability_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(sync_blocking_in_async::SyncBlockingInAsyncPipeline::new()?),
        Box::new(memory_leak_indicators::MemoryLeakIndicatorsPipeline::new()?),
    ])
}

