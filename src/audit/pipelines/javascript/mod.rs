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

pub mod coupling;
pub mod dead_code;
pub mod duplicate_code;

pub mod ssrf;
pub mod xss_dom_injection;

use crate::audit::pipeline::{AnyPipeline, Pipeline};
use crate::language::Language;
use anyhow::Result;

pub fn tech_debt_pipelines() -> Result<Vec<AnyPipeline>> {
    Ok(vec![
        AnyPipeline::Node(Box::new(var_usage::VarUsagePipeline::new()?)),
        AnyPipeline::Node(Box::new(callback_hell::CallbackHellPipeline::new()?)),
        AnyPipeline::Graph(Box::new(implicit_globals::ImplicitGlobalsPipeline::new()?)),
        AnyPipeline::Node(Box::new(loose_equality::LooseEqualityPipeline::new()?)),
        AnyPipeline::Node(Box::new(unhandled_promise::UnhandledPromisePipeline::new()?)),
        AnyPipeline::Node(Box::new(argument_mutation::ArgumentMutationPipeline::new()?)),
        AnyPipeline::Node(Box::new(console_log_in_prod::ConsoleLogPipeline::new()?)),
        AnyPipeline::Graph(Box::new(event_listener_leak::EventListenerLeakPipeline::new()?)),
        AnyPipeline::Node(Box::new(loose_truthiness::LooseTruthinessPipeline::new()?)),
        AnyPipeline::Node(Box::new(no_optional_chaining::NoOptionalChainingPipeline::new()?)),
        AnyPipeline::Node(Box::new(magic_numbers::JsMagicNumbersPipeline::new()?)),
        AnyPipeline::Node(Box::new(shallow_spread_copy::ShallowSpreadCopyPipeline::new()?)),
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

pub fn security_pipelines(language: Language) -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(xss_dom_injection::XssDomInjectionPipeline::new(language)?),
        Box::new(ssrf::SsrfPipeline::new(language)?),
    ])
}

pub fn scalability_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![])
}

