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
            let panic = pipelines::rust::panic_detection::PanicDetectionPipeline::new()?;
            let clone = pipelines::rust::clone_detection::CloneDetectionPipeline::new()?;
            let god_object =
                pipelines::rust::god_object_detection::GodObjectDetectionPipeline::new()?;
            let stringly = pipelines::rust::stringly_typed::StringlyTypedPipeline::new()?;
            let must_use = pipelines::rust::must_use_ignored::MustUseIgnoredPipeline::new()?;
            let mutex = pipelines::rust::mutex_overuse::MutexOverusePipeline::new()?;
            let pub_field = pipelines::rust::pub_field_leakage::PubFieldLeakagePipeline::new()?;
            let missing_trait =
                pipelines::rust::missing_trait_abstraction::MissingTraitAbstractionPipeline::new()?;
            let async_blocking = pipelines::rust::async_blocking::AsyncBlockingPipeline::new()?;
            let magic = pipelines::rust::magic_numbers::MagicNumbersPipeline::new()?;
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
        Language::Python => {
            let bare_except = pipelines::python::bare_except::BareExceptPipeline::new()?;
            let mutable_default = pipelines::python::mutable_default_args::MutableDefaultArgsPipeline::new()?;
            let magic = pipelines::python::magic_numbers::PythonMagicNumbersPipeline::new()?;
            let god_fn = pipelines::python::god_functions::GodFunctionsPipeline::new()?;
            let type_hints = pipelines::python::missing_type_hints::MissingTypeHintsPipeline::new()?;
            let stringly = pipelines::python::stringly_typed::StringlyTypedPipeline::new()?;
            let deep_nesting = pipelines::python::deep_nesting::DeepNestingPipeline::new()?;
            let duplicate = pipelines::python::duplicate_logic::DuplicateLogicPipeline::new()?;
            Ok(vec![
                Box::new(bare_except),
                Box::new(mutable_default),
                Box::new(magic),
                Box::new(god_fn),
                Box::new(type_hints),
                Box::new(stringly),
                Box::new(deep_nesting),
                Box::new(duplicate),
            ])
        }
        Language::Php => {
            let deprecated_mysql = pipelines::php::deprecated_mysql_api::DeprecatedMysqlApiPipeline::new()?;
            let sql_injection = pipelines::php::sql_injection::SqlInjectionPipeline::new()?;
            let error_suppression = pipelines::php::error_suppression::ErrorSuppressionPipeline::new()?;
            let missing_types = pipelines::php::missing_type_declarations::MissingTypeDeclarationsPipeline::new()?;
            let god_class = pipelines::php::god_class::GodClassPipeline::new()?;
            let extract_usage = pipelines::php::extract_usage::ExtractUsagePipeline::new()?;
            let silent_exception = pipelines::php::silent_exception::SilentExceptionPipeline::new()?;
            let unsafe_include = pipelines::php::unsafe_include::UnsafeIncludePipeline::new()?;
            let unescaped_output = pipelines::php::unescaped_output::UnescapedOutputPipeline::new()?;
            let logic_in_views = pipelines::php::logic_in_views::LogicInViewsPipeline::new()?;
            Ok(vec![
                Box::new(deprecated_mysql),
                Box::new(sql_injection),
                Box::new(error_suppression),
                Box::new(missing_types),
                Box::new(god_class),
                Box::new(extract_usage),
                Box::new(silent_exception),
                Box::new(unsafe_include),
                Box::new(unescaped_output),
                Box::new(logic_in_views),
            ])
        }
        _ => Ok(vec![]),
    }
}

pub fn supported_audit_languages() -> Vec<Language> {
    vec![Language::Rust, Language::Go, Language::Python, Language::Php]
}
