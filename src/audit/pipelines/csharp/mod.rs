pub mod primitives;

pub mod anemic_domain_model;
pub mod disposable_not_disposed;
pub mod exception_control_flow;
pub mod god_class;
pub mod god_controller;
pub mod hardcoded_config;
pub mod missing_cancellation_token;
pub mod null_reference_risk;
pub mod static_global_state;
pub mod stringly_typed;
pub mod sync_over_async;
pub mod thread_sleep;

pub mod cognitive;
pub mod comment_ratio;
pub mod cyclomatic;
pub mod function_length;

use anyhow::Result;
use crate::audit::pipeline::Pipeline;

pub fn tech_debt_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(sync_over_async::SyncOverAsyncPipeline::new()?),
        Box::new(null_reference_risk::NullReferenceRiskPipeline::new()?),
        Box::new(exception_control_flow::ExceptionControlFlowPipeline::new()?),
        Box::new(static_global_state::StaticGlobalStatePipeline::new()?),
        Box::new(disposable_not_disposed::DisposableNotDisposedPipeline::new()?),
        Box::new(god_class::GodClassPipeline::new()?),
        Box::new(stringly_typed::StringlyTypedPipeline::new()?),
        Box::new(god_controller::GodControllerPipeline::new()?),
        Box::new(thread_sleep::ThreadSleepPipeline::new()?),
        Box::new(missing_cancellation_token::MissingCancellationTokenPipeline::new()?),
        Box::new(hardcoded_config::HardcodedConfigPipeline::new()?),
        Box::new(anemic_domain_model::AnemicDomainModelPipeline::new()?),
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
