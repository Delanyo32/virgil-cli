use anyhow::Result;
use tree_sitter::Tree;

use crate::language::Language;

use super::models::AuditFinding;
use super::pipelines;

pub trait Pipeline: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn check(&self, tree: &Tree, source: &[u8], file_path: &str) -> Vec<AuditFinding>;
}

pub fn pipelines_for_language(language: Language) -> Result<Vec<Box<dyn Pipeline>>> {
    match language {
        Language::Rust => {
            let panic = pipelines::panic_detection::PanicDetectionPipeline::new()?;
            let clone = pipelines::clone_detection::CloneDetectionPipeline::new()?;
            let god_object =
                pipelines::god_object_detection::GodObjectDetectionPipeline::new()?;
            Ok(vec![Box::new(panic), Box::new(clone), Box::new(god_object)])
        }
        _ => Ok(vec![]),
    }
}
