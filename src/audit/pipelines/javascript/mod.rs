pub mod primitives;

pub mod argument_mutation;
pub mod callback_hell;
pub mod console_log_in_prod;
pub mod event_listener_leak;
pub mod implicit_globals;
pub mod loose_equality;
pub mod loose_truthiness;
pub mod magic_numbers;
pub mod no_optional_chaining;
pub mod shallow_spread_copy;
pub mod unhandled_promise;
pub mod var_usage;

pub mod cognitive;
pub mod comment_ratio;
pub mod cyclomatic;
pub mod function_length;

pub mod coupling;
pub mod dead_code;
pub mod duplicate_code;

pub mod code_injection;
pub mod command_injection;
pub mod insecure_deserialization;
pub mod path_traversal;
pub mod prototype_pollution;
pub mod redos_resource_exhaustion;
pub mod ssrf;
pub mod timing_weak_crypto;
pub mod xss_dom_injection;

use anyhow::Result;
use crate::audit::pipeline::Pipeline;
use crate::language::Language;

pub fn tech_debt_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(var_usage::VarUsagePipeline::new()?),
        Box::new(callback_hell::CallbackHellPipeline::new()?),
        Box::new(implicit_globals::ImplicitGlobalsPipeline::new()?),
        Box::new(loose_equality::LooseEqualityPipeline::new()?),
        Box::new(unhandled_promise::UnhandledPromisePipeline::new()?),
        Box::new(argument_mutation::ArgumentMutationPipeline::new()?),
        Box::new(console_log_in_prod::ConsoleLogPipeline::new()?),
        Box::new(event_listener_leak::EventListenerLeakPipeline::new()?),
        Box::new(loose_truthiness::LooseTruthinessPipeline::new()?),
        Box::new(no_optional_chaining::NoOptionalChainingPipeline::new()?),
        Box::new(magic_numbers::JsMagicNumbersPipeline::new()?),
        Box::new(shallow_spread_copy::ShallowSpreadCopyPipeline::new()?),
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

pub fn security_pipelines(language: Language) -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(xss_dom_injection::XssDomInjectionPipeline::new(language)?),
        Box::new(code_injection::CodeInjectionPipeline::new(language)?),
        Box::new(command_injection::CommandInjectionPipeline::new(language)?),
        Box::new(path_traversal::PathTraversalPipeline::new(language)?),
        Box::new(prototype_pollution::PrototypePollutionPipeline::new(language)?),
        Box::new(redos_resource_exhaustion::RedosResourceExhaustionPipeline::new(language)?),
        Box::new(ssrf::SsrfPipeline::new(language)?),
        Box::new(insecure_deserialization::InsecureDeserializationPipeline::new(language)?),
        Box::new(timing_weak_crypto::TimingWeakCryptoPipeline::new(language)?),
    ])
}
