// All non-taint tech-debt + code-style pipelines migrated to JSON in src/audit/builtin/
// Only taint-based security pipelines remain as Rust

pub mod primitives;

pub mod ssrf;
pub mod xss_dom_injection;

use crate::audit::pipeline::{AnyPipeline, Pipeline};
use crate::language::Language;
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

pub fn security_pipelines(language: Language) -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(xss_dom_injection::XssDomInjectionPipeline::new(language)?),
        Box::new(ssrf::SsrfPipeline::new(language)?),
    ])
}

pub fn scalability_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![])
}
