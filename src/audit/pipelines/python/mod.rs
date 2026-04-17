pub mod primitives;

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
    Ok(vec![])
}

pub fn scalability_pipelines() -> Result<Vec<AnyPipeline>> {
    Ok(vec![])
}
