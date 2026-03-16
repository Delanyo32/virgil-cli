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
            let stringly = pipelines::stringly_typed::StringlyTypedPipeline::new()?;
            let must_use = pipelines::must_use_ignored::MustUseIgnoredPipeline::new()?;
            let mutex = pipelines::mutex_overuse::MutexOverusePipeline::new()?;
            let pub_field = pipelines::pub_field_leakage::PubFieldLeakagePipeline::new()?;
            let missing_trait =
                pipelines::missing_trait_abstraction::MissingTraitAbstractionPipeline::new()?;
            let async_blocking = pipelines::async_blocking::AsyncBlockingPipeline::new()?;
            let magic = pipelines::magic_numbers::MagicNumbersPipeline::new()?;
            Ok(vec![
                Box::new(panic),
                Box::new(clone),
                Box::new(god_object),
                Box::new(stringly),
                Box::new(must_use),
                Box::new(mutex),
                Box::new(pub_field),
                Box::new(missing_trait),
                Box::new(async_blocking),
                Box::new(magic),
            ])
        }
        Language::Go => {
            let error_swallow = pipelines::go::error_swallowing::ErrorSwallowingPipeline::new()?;
            let god_struct = pipelines::go::god_struct::GodStructPipeline::new()?;
            let naked_iface = pipelines::go::naked_interface::NakedInterfacePipeline::new()?;
            let context = pipelines::go::context_not_propagated::ContextNotPropagatedPipeline::new()?;
            let init = pipelines::go::init_abuse::InitAbusePipeline::new()?;
            let mutex = pipelines::go::mutex_misuse::MutexMisusePipeline::new()?;
            let goroutine = pipelines::go::goroutine_leak::GoroutineLeakPipeline::new()?;
            let stringly = pipelines::go::stringly_typed_config::StringlyTypedConfigPipeline::new()?;
            let concrete = pipelines::go::concrete_return_type::ConcreteReturnTypePipeline::new()?;
            let magic = pipelines::go::magic_numbers::GoMagicNumbersPipeline::new()?;
            Ok(vec![
                Box::new(error_swallow),
                Box::new(god_struct),
                Box::new(naked_iface),
                Box::new(context),
                Box::new(init),
                Box::new(mutex),
                Box::new(goroutine),
                Box::new(stringly),
                Box::new(concrete),
                Box::new(magic),
            ])
        }
        _ => Ok(vec![]),
    }
}

pub fn supported_audit_languages() -> Vec<Language> {
    vec![Language::Rust, Language::Go]
}
