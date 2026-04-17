// All non-taint tech-debt + code-style pipelines migrated to JSON in src/audit/builtin/

pub mod primitives;

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

pub fn security_pipelines(_language: Language) -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![])
}

pub fn scalability_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![])
}
