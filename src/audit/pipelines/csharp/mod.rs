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

pub mod coupling;
pub mod dead_code;
pub mod duplicate_code;

pub mod command_injection;
pub mod csharp_path_traversal;
pub mod csharp_race_conditions;
pub mod csharp_ssrf;
pub mod insecure_deserialization;
pub mod reflection_unsafe;
pub mod sql_injection;
pub mod weak_cryptography;
pub mod xxe;

pub mod memory_leak_indicators;
pub mod sync_blocking_in_async;

use crate::audit::pipeline::{AnyPipeline, Pipeline};
use anyhow::Result;

pub fn tech_debt_pipelines() -> Result<Vec<AnyPipeline>> {
    Ok(vec![
        AnyPipeline::Graph(Box::new(sync_over_async::SyncOverAsyncPipeline::new()?)),
        AnyPipeline::Graph(Box::new(null_reference_risk::NullReferenceRiskPipeline::new()?)),
        AnyPipeline::Graph(Box::new(exception_control_flow::ExceptionControlFlowPipeline::new()?)),
        AnyPipeline::Graph(Box::new(static_global_state::StaticGlobalStatePipeline::new()?)),
        AnyPipeline::Graph(Box::new(disposable_not_disposed::DisposableNotDisposedPipeline::new()?)),
        AnyPipeline::Graph(Box::new(god_class::GodClassPipeline::new()?)),
        AnyPipeline::Graph(Box::new(stringly_typed::StringlyTypedPipeline::new()?)),
        AnyPipeline::Graph(Box::new(god_controller::GodControllerPipeline::new()?)),
        AnyPipeline::Graph(Box::new(thread_sleep::ThreadSleepPipeline::new()?)),
        AnyPipeline::Graph(Box::new(missing_cancellation_token::MissingCancellationTokenPipeline::new()?)),
        AnyPipeline::Graph(Box::new(hardcoded_config::HardcodedConfigPipeline::new()?)),
        AnyPipeline::Graph(Box::new(anemic_domain_model::AnemicDomainModelPipeline::new()?)),
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
        Box::new(sql_injection::SqlInjectionPipeline::new()?),
        Box::new(command_injection::CommandInjectionPipeline::new()?),
        Box::new(weak_cryptography::WeakCryptographyPipeline::new()?),
        Box::new(insecure_deserialization::InsecureDeserializationPipeline::new()?),
        Box::new(csharp_path_traversal::CSharpPathTraversalPipeline::new()?),
        Box::new(xxe::XxePipeline::new()?),
        Box::new(csharp_ssrf::CSharpSsrfPipeline::new()?),
        Box::new(csharp_race_conditions::CSharpRaceConditionsPipeline::new()?),
        Box::new(reflection_unsafe::ReflectionUnsafePipeline::new()?),
    ])
}

pub fn scalability_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(sync_blocking_in_async::SyncBlockingInAsyncPipeline::new()?),
        Box::new(memory_leak_indicators::MemoryLeakIndicatorsPipeline::new()?),
    ])
}

