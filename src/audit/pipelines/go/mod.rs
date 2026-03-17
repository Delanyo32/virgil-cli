pub mod primitives;

pub mod concrete_return_type;
pub mod context_not_propagated;
pub mod error_swallowing;
pub mod god_struct;
pub mod goroutine_leak;
pub mod init_abuse;
pub mod magic_numbers;
pub mod mutex_misuse;
pub mod naked_interface;
pub mod stringly_typed_config;

pub mod cognitive;
pub mod comment_ratio;
pub mod cyclomatic;
pub mod function_length;

use anyhow::Result;
use crate::audit::pipeline::Pipeline;

pub fn tech_debt_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(error_swallowing::ErrorSwallowingPipeline::new()?),
        Box::new(god_struct::GodStructPipeline::new()?),
        Box::new(naked_interface::NakedInterfacePipeline::new()?),
        Box::new(context_not_propagated::ContextNotPropagatedPipeline::new()?),
        Box::new(init_abuse::InitAbusePipeline::new()?),
        Box::new(mutex_misuse::MutexMisusePipeline::new()?),
        Box::new(goroutine_leak::GoroutineLeakPipeline::new()?),
        Box::new(stringly_typed_config::StringlyTypedConfigPipeline::new()?),
        Box::new(concrete_return_type::ConcreteReturnTypePipeline::new()?),
        Box::new(magic_numbers::GoMagicNumbersPipeline::new()?),
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
