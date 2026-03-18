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

pub mod coupling;
pub mod dead_code;
pub mod duplicate_code;

pub mod command_injection;
pub mod sql_injection;
pub mod go_path_traversal;
pub mod go_race_conditions;
pub mod go_resource_exhaustion;
pub mod go_integer_overflow;
pub mod go_type_confusion;
pub mod ssrf_open_redirect;

pub mod n_plus_one_queries;
pub mod sync_blocking_in_async;
pub mod memory_leak_indicators;

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
        Box::new(sql_injection::SqlInjectionPipeline::new()?),
        Box::new(go_path_traversal::GoPathTraversalPipeline::new()?),
        Box::new(go_race_conditions::GoRaceConditionsPipeline::new()?),
        Box::new(go_resource_exhaustion::GoResourceExhaustionPipeline::new()?),
        Box::new(go_integer_overflow::GoIntegerOverflowPipeline::new()?),
        Box::new(go_type_confusion::GoTypeConfusionPipeline::new()?),
        Box::new(ssrf_open_redirect::SsrfOpenRedirectPipeline::new()?),
    ])
}

pub fn scalability_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(n_plus_one_queries::NPlusOneQueriesPipeline::new()?),
        Box::new(sync_blocking_in_async::SyncBlockingInAsyncPipeline::new()?),
        Box::new(memory_leak_indicators::MemoryLeakIndicatorsPipeline::new()?),
    ])
}
