// All non-taint tech-debt + code-style pipelines migrated to JSON.
// Security pipelines delegated to javascript module (taint-based exceptions).

use crate::audit::pipeline::Pipeline;
use crate::audit::pipelines;
use crate::language::Language;
use anyhow::Result;

pub fn tech_debt_pipelines(_language: Language) -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![])
}

pub fn complexity_pipelines(_language: Language) -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![])
}

pub fn code_style_pipelines(_language: Language) -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![])
}

pub fn security_pipelines(language: Language) -> Result<Vec<Box<dyn Pipeline>>> {
    pipelines::javascript::security_pipelines(language)
}

pub fn scalability_pipelines(_language: Language) -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![])
}
