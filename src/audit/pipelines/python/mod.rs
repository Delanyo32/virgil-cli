pub mod primitives;

pub mod bare_except;
pub mod deep_nesting;
pub mod duplicate_logic;
pub mod god_functions;
pub mod magic_numbers;
pub mod missing_type_hints;
pub mod mutable_default_args;
pub mod stringly_typed;

pub mod coupling;
pub mod dead_code;
pub mod duplicate_code;

pub mod sql_injection;
pub mod ssrf;

pub mod empty_test_files;
pub mod test_assertions;
pub mod test_hygiene;
pub mod test_pollution;

use crate::audit::pipeline::AnyPipeline;
use anyhow::Result;

pub fn tech_debt_pipelines() -> Result<Vec<AnyPipeline>> {
    Ok(vec![
        AnyPipeline::Graph(Box::new(bare_except::BareExceptPipeline::new()?)),
        AnyPipeline::Graph(Box::new(mutable_default_args::MutableDefaultArgsPipeline::new()?)),
        AnyPipeline::Graph(Box::new(magic_numbers::PythonMagicNumbersPipeline::new()?)),
        AnyPipeline::Graph(Box::new(god_functions::GodFunctionsPipeline::new()?)),
        AnyPipeline::Graph(Box::new(missing_type_hints::MissingTypeHintsPipeline::new()?)),
        AnyPipeline::Graph(Box::new(stringly_typed::StringlyTypedPipeline::new()?)),
        AnyPipeline::Graph(Box::new(deep_nesting::DeepNestingPipeline::new()?)),
        AnyPipeline::Graph(Box::new(duplicate_logic::DuplicateLogicPipeline::new()?)),
        AnyPipeline::Graph(Box::new(test_assertions::TestAssertionsPipeline::new()?)),
        AnyPipeline::Graph(Box::new(test_pollution::TestPollutionPipeline::new()?)),
        AnyPipeline::Graph(Box::new(test_hygiene::TestHygienePipeline::new()?)),
        AnyPipeline::Graph(Box::new(empty_test_files::EmptyTestFilesPipeline::new()?)),
    ])
}

pub fn complexity_pipelines() -> Result<Vec<AnyPipeline>> {
    Ok(vec![])
}

pub fn code_style_pipelines() -> Result<Vec<AnyPipeline>> {
    Ok(vec![
        AnyPipeline::Graph(Box::new(dead_code::DeadCodePipeline::new()?)),
        AnyPipeline::Graph(Box::new(duplicate_code::DuplicateCodePipeline::new()?)),
        AnyPipeline::Graph(Box::new(coupling::CouplingPipeline::new()?)),
    ])
}

pub fn security_pipelines() -> Result<Vec<AnyPipeline>> {
    Ok(vec![
        AnyPipeline::Graph(Box::new(sql_injection::SqlInjectionPipeline::new()?)),
        AnyPipeline::Graph(Box::new(ssrf::SsrfPipeline::new()?)),
    ])
}

pub fn scalability_pipelines() -> Result<Vec<AnyPipeline>> {
    Ok(vec![])
}

