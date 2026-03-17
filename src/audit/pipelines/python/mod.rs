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
