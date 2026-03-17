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

pub mod cognitive;
pub mod comment_ratio;
pub mod cyclomatic;
pub mod function_length;

pub mod coupling;
pub mod dead_code;
pub mod duplicate_code;

use anyhow::Result;
use crate::audit::pipeline::Pipeline;

pub fn tech_debt_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(raw_memory_management::RawMemoryManagementPipeline::new()?),
        Box::new(rule_of_five::RuleOfFivePipeline::new()?),
        Box::new(c_style_cast::CStyleCastPipeline::new()?),
        Box::new(large_object_by_value::LargeObjectByValuePipeline::new()?),
        Box::new(endl_flush::EndlFlushPipeline::new()?),
        Box::new(missing_override::MissingOverridePipeline::new()?),
        Box::new(raw_union::RawUnionPipeline::new()?),
        Box::new(excessive_includes::ExcessiveIncludesPipeline::new()?),
        Box::new(exception_across_boundary::ExceptionAcrossBoundaryPipeline::new()?),
        Box::new(uninitialized_member::UninitializedMemberPipeline::new()?),
        Box::new(shared_ptr_cycle_risk::SharedPtrCycleRiskPipeline::new()?),
        Box::new(magic_numbers::CppMagicNumbersPipeline::new()?),
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
