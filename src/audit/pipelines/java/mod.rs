pub mod primitives;

pub mod java_ssrf;
pub mod sql_injection;
pub mod xxe;

use crate::audit::pipeline::{AnyPipeline, Pipeline};
use anyhow::Result;

pub fn tech_debt_pipelines() -> Result<Vec<AnyPipeline>> {
    Ok(vec![])
}

pub fn complexity_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![])
}

pub fn code_style_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![])
}

pub fn security_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(sql_injection::SqlInjectionPipeline::new()?),
        Box::new(xxe::XxePipeline::new()?),
        Box::new(java_ssrf::JavaSsrfPipeline::new()?),
    ])
}

pub fn scalability_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![])
}
