pub mod primitives;

pub mod sql_injection;
pub mod ssrf_open_redirect;

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
        Box::new(ssrf_open_redirect::SsrfOpenRedirectPipeline::new()?),
    ])
}

pub fn scalability_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![])
}
