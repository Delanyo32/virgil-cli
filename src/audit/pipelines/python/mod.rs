pub mod primitives;

pub mod sql_injection;
pub mod ssrf;

use crate::audit::pipeline::AnyPipeline;
use anyhow::Result;

pub fn tech_debt_pipelines() -> Result<Vec<AnyPipeline>> {
    Ok(vec![])
}

pub fn complexity_pipelines() -> Result<Vec<AnyPipeline>> {
    Ok(vec![])
}

pub fn code_style_pipelines() -> Result<Vec<AnyPipeline>> {
    Ok(vec![])
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
