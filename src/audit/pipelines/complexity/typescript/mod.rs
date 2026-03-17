pub mod cyclomatic;
pub mod function_length;
pub mod cognitive;
pub mod comment_ratio;

use anyhow::Result;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

pub fn complexity_pipelines(language: Language) -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(cyclomatic::CyclomaticComplexityPipeline::new(language)?),
        Box::new(function_length::FunctionLengthPipeline::new(language)?),
        Box::new(cognitive::CognitiveComplexityPipeline::new(language)?),
        Box::new(comment_ratio::CommentToCodeRatioPipeline::new(language)?),
    ])
}
