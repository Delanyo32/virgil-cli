pub mod primitives;

pub mod exception_swallowing;
pub mod god_class;
pub mod instanceof_chains;
pub mod magic_strings;
pub mod missing_final;
pub mod mutable_public_fields;
pub mod null_returns;
pub mod raw_types;
pub mod resource_leaks;
pub mod static_utility_sprawl;
pub mod string_concat_in_loops;

pub mod cognitive;
pub mod comment_ratio;
pub mod cyclomatic;
pub mod function_length;

pub mod coupling;
pub mod dead_code;
pub mod duplicate_code;

pub mod sql_injection;
pub mod command_injection;
pub mod weak_cryptography;
pub mod insecure_deserialization;
pub mod java_path_traversal;
pub mod xxe;
pub mod java_ssrf;
pub mod reflection_injection;
pub mod java_race_conditions;

use anyhow::Result;
use crate::audit::pipeline::Pipeline;

pub fn tech_debt_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(god_class::GodClassPipeline::new()?),
        Box::new(null_returns::NullReturnsPipeline::new()?),
        Box::new(exception_swallowing::ExceptionSwallowingPipeline::new()?),
        Box::new(mutable_public_fields::MutablePublicFieldsPipeline::new()?),
        Box::new(string_concat_in_loops::StringConcatInLoopsPipeline::new()?),
        Box::new(instanceof_chains::InstanceofChainsPipeline::new()?),
        Box::new(resource_leaks::ResourceLeaksPipeline::new()?),
        Box::new(static_utility_sprawl::StaticUtilitySprawlPipeline::new()?),
        Box::new(magic_strings::MagicStringsPipeline::new()?),
        Box::new(raw_types::RawTypesPipeline::new()?),
        Box::new(missing_final::MissingFinalPipeline::new()?),
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
        Box::new(sql_injection::SqlInjectionPipeline::new()?),
        Box::new(command_injection::CommandInjectionPipeline::new()?),
        Box::new(weak_cryptography::WeakCryptographyPipeline::new()?),
        Box::new(insecure_deserialization::InsecureDeserializationPipeline::new()?),
        Box::new(java_path_traversal::JavaPathTraversalPipeline::new()?),
        Box::new(xxe::XxePipeline::new()?),
        Box::new(java_ssrf::JavaSsrfPipeline::new()?),
        Box::new(reflection_injection::ReflectionInjectionPipeline::new()?),
        Box::new(java_race_conditions::JavaRaceConditionsPipeline::new()?),
    ])
}
