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
        Language::Java => {
            let god_class = pipelines::java::god_class::GodClassPipeline::new()?;
            let null_returns = pipelines::java::null_returns::NullReturnsPipeline::new()?;
            let exception_swallowing =
                pipelines::java::exception_swallowing::ExceptionSwallowingPipeline::new()?;
            let mutable_public_fields =
                pipelines::java::mutable_public_fields::MutablePublicFieldsPipeline::new()?;
            let string_concat =
                pipelines::java::string_concat_in_loops::StringConcatInLoopsPipeline::new()?;
            let instanceof_chains =
                pipelines::java::instanceof_chains::InstanceofChainsPipeline::new()?;
            let resource_leaks =
                pipelines::java::resource_leaks::ResourceLeaksPipeline::new()?;
            let static_utility =
                pipelines::java::static_utility_sprawl::StaticUtilitySprawlPipeline::new()?;
            let magic_strings =
                pipelines::java::magic_strings::MagicStringsPipeline::new()?;
            let raw_types = pipelines::java::raw_types::RawTypesPipeline::new()?;
            let missing_final =
                pipelines::java::missing_final::MissingFinalPipeline::new()?;
            Ok(vec![
                Box::new(god_class),
                Box::new(null_returns),
                Box::new(exception_swallowing),
                Box::new(mutable_public_fields),
                Box::new(string_concat),
                Box::new(instanceof_chains),
                Box::new(resource_leaks),
                Box::new(static_utility),
                Box::new(magic_strings),
                Box::new(raw_types),
                Box::new(missing_final),
            ])
        }
        Language::JavaScript => {
            let var_usage =
                pipelines::javascript::var_usage::VarUsagePipeline::new()?;
            let callback_hell =
                pipelines::javascript::callback_hell::CallbackHellPipeline::new()?;
            let implicit_globals =
                pipelines::javascript::implicit_globals::ImplicitGlobalsPipeline::new()?;
            let loose_equality =
                pipelines::javascript::loose_equality::LooseEqualityPipeline::new()?;
            let unhandled_promise =
                pipelines::javascript::unhandled_promise::UnhandledPromisePipeline::new()?;
            let argument_mutation =
                pipelines::javascript::argument_mutation::ArgumentMutationPipeline::new()?;
            let console_log =
                pipelines::javascript::console_log_in_prod::ConsoleLogPipeline::new()?;
            let event_listener =
                pipelines::javascript::event_listener_leak::EventListenerLeakPipeline::new()?;
            let loose_truthiness =
                pipelines::javascript::loose_truthiness::LooseTruthinessPipeline::new()?;
            let no_optional_chaining =
                pipelines::javascript::no_optional_chaining::NoOptionalChainingPipeline::new()?;
            let magic_numbers =
                pipelines::javascript::magic_numbers::JsMagicNumbersPipeline::new()?;
            let shallow_spread =
                pipelines::javascript::shallow_spread_copy::ShallowSpreadCopyPipeline::new()?;
            Ok(vec![
                Box::new(var_usage),
                Box::new(callback_hell),
                Box::new(implicit_globals),
                Box::new(loose_equality),
                Box::new(unhandled_promise),
                Box::new(argument_mutation),
                Box::new(console_log),
                Box::new(event_listener),
                Box::new(loose_truthiness),
                Box::new(no_optional_chaining),
                Box::new(magic_numbers),
                Box::new(shallow_spread),
            ])
        }
        Language::TypeScript | Language::Tsx => {
            let any_escape =
                pipelines::typescript::any_escape_hatch::AnyEscapeHatchPipeline::new(language)?;
            let type_assertions =
                pipelines::typescript::type_assertions::TypeAssertionsPipeline::new(language)?;
            let optional =
                pipelines::typescript::optional_everything::OptionalEverythingPipeline::new(language)?;
            let duplication =
                pipelines::typescript::type_duplication::TypeDuplicationPipeline::new(language)?;
            let record_any =
                pipelines::typescript::record_string_any::RecordStringAnyPipeline::new(language)?;
            let enum_usage =
                pipelines::typescript::enum_usage::EnumUsagePipeline::new(language)?;
            let implicit_any =
                pipelines::typescript::implicit_any::ImplicitAnyPipeline::new(language)?;
            let unchecked_index =
                pipelines::typescript::unchecked_index_access::UncheckedIndexAccessPipeline::new(language)?;
            let mutable =
                pipelines::typescript::mutable_types::MutableTypesPipeline::new(language)?;
            let unconstrained =
                pipelines::typescript::unconstrained_generics::UnconstrainedGenericsPipeline::new(language)?;
            let leaking =
                pipelines::typescript::leaking_impl_types::LeakingImplTypesPipeline::new(language)?;
            Ok(vec![
                Box::new(any_escape),
                Box::new(type_assertions),
                Box::new(optional),
                Box::new(duplication),
                Box::new(record_any),
                Box::new(enum_usage),
                Box::new(implicit_any),
                Box::new(unchecked_index),
                Box::new(mutable),
                Box::new(unconstrained),
                Box::new(leaking),
            ])
        }
        Language::C => {
            let buffer_overflows =
                pipelines::c::buffer_overflows::BufferOverflowsPipeline::new()?;
            let unchecked_malloc =
                pipelines::c::unchecked_malloc::UncheckedMallocPipeline::new()?;
            let memory_leaks =
                pipelines::c::memory_leaks::MemoryLeaksPipeline::new()?;
            let signed_unsigned =
                pipelines::c::signed_unsigned_mismatch::SignedUnsignedMismatchPipeline::new()?;
            let magic_numbers =
                pipelines::c::magic_numbers::CMagicNumbersPipeline::new()?;
            let global_mutable =
                pipelines::c::global_mutable_state::GlobalMutableStatePipeline::new()?;
            let typedef_pointer =
                pipelines::c::typedef_pointer_hiding::TypedefPointerHidingPipeline::new()?;
            let define_inline =
                pipelines::c::define_instead_of_inline::DefineInsteadOfInlinePipeline::new()?;
            let ignored_return =
                pipelines::c::ignored_return_values::IgnoredReturnValuesPipeline::new()?;
            let void_pointer =
                pipelines::c::void_pointer_abuse::VoidPointerAbusePipeline::new()?;
            let missing_const =
                pipelines::c::missing_const::MissingConstPipeline::new()?;
            let raw_struct =
                pipelines::c::raw_struct_serialization::RawStructSerializationPipeline::new()?;
            Ok(vec![
                Box::new(buffer_overflows),
                Box::new(unchecked_malloc),
                Box::new(memory_leaks),
                Box::new(signed_unsigned),
                Box::new(magic_numbers),
                Box::new(global_mutable),
                Box::new(typedef_pointer),
                Box::new(define_inline),
                Box::new(ignored_return),
                Box::new(void_pointer),
                Box::new(missing_const),
                Box::new(raw_struct),
            ])
        }
        Language::Cpp => {
            let raw_memory =
                pipelines::cpp::raw_memory_management::RawMemoryManagementPipeline::new()?;
            let rule_of_five =
                pipelines::cpp::rule_of_five::RuleOfFivePipeline::new()?;
            let c_style_cast =
                pipelines::cpp::c_style_cast::CStyleCastPipeline::new()?;
            let large_object =
                pipelines::cpp::large_object_by_value::LargeObjectByValuePipeline::new()?;
            let endl_flush =
                pipelines::cpp::endl_flush::EndlFlushPipeline::new()?;
            let missing_override =
                pipelines::cpp::missing_override::MissingOverridePipeline::new()?;
            let raw_union =
                pipelines::cpp::raw_union::RawUnionPipeline::new()?;
            let excessive_includes =
                pipelines::cpp::excessive_includes::ExcessiveIncludesPipeline::new()?;
            let exception_boundary =
                pipelines::cpp::exception_across_boundary::ExceptionAcrossBoundaryPipeline::new()?;
            let uninitialized =
                pipelines::cpp::uninitialized_member::UninitializedMemberPipeline::new()?;
            let shared_ptr_cycle =
                pipelines::cpp::shared_ptr_cycle_risk::SharedPtrCycleRiskPipeline::new()?;
            let magic_numbers =
                pipelines::cpp::magic_numbers::CppMagicNumbersPipeline::new()?;
            Ok(vec![
                Box::new(raw_memory),
                Box::new(rule_of_five),
                Box::new(c_style_cast),
                Box::new(large_object),
                Box::new(endl_flush),
                Box::new(missing_override),
                Box::new(raw_union),
                Box::new(excessive_includes),
                Box::new(exception_boundary),
                Box::new(uninitialized),
                Box::new(shared_ptr_cycle),
                Box::new(magic_numbers),
            ])
        }
        Language::CSharp => {
            let sync_over_async =
                pipelines::csharp::sync_over_async::SyncOverAsyncPipeline::new()?;
            let null_ref =
                pipelines::csharp::null_reference_risk::NullReferenceRiskPipeline::new()?;
            let exception =
                pipelines::csharp::exception_control_flow::ExceptionControlFlowPipeline::new()?;
            let static_state =
                pipelines::csharp::static_global_state::StaticGlobalStatePipeline::new()?;
            let disposable =
                pipelines::csharp::disposable_not_disposed::DisposableNotDisposedPipeline::new()?;
            let god_class =
                pipelines::csharp::god_class::GodClassPipeline::new()?;
            let stringly =
                pipelines::csharp::stringly_typed::StringlyTypedPipeline::new()?;
            let god_controller =
                pipelines::csharp::god_controller::GodControllerPipeline::new()?;
            let thread_sleep =
                pipelines::csharp::thread_sleep::ThreadSleepPipeline::new()?;
            let cancellation =
                pipelines::csharp::missing_cancellation_token::MissingCancellationTokenPipeline::new()?;
            let hardcoded =
                pipelines::csharp::hardcoded_config::HardcodedConfigPipeline::new()?;
            let anemic =
                pipelines::csharp::anemic_domain_model::AnemicDomainModelPipeline::new()?;
            Ok(vec![
                Box::new(sync_over_async),
                Box::new(null_ref),
                Box::new(exception),
                Box::new(static_state),
                Box::new(disposable),
                Box::new(god_class),
                Box::new(stringly),
                Box::new(god_controller),
                Box::new(thread_sleep),
                Box::new(cancellation),
                Box::new(hardcoded),
                Box::new(anemic),
            ])
        }
        _ => Ok(vec![]),
    }
}

pub fn supported_audit_languages() -> Vec<Language> {
    vec![Language::Rust, Language::Go, Language::Python, Language::Php, Language::Java, Language::JavaScript, Language::TypeScript, Language::Tsx, Language::C, Language::Cpp, Language::CSharp]
}

pub fn complexity_pipelines_for_language(language: Language) -> Result<Vec<Box<dyn Pipeline>>> {
    match language {
        Language::Java => Ok(pipelines::complexity::java::complexity_pipelines()?),
        Language::JavaScript => Ok(pipelines::complexity::javascript::complexity_pipelines()?),
        Language::TypeScript | Language::Tsx => Ok(pipelines::complexity::typescript::complexity_pipelines(language)?),
        Language::Python => Ok(pipelines::complexity::python::complexity_pipelines()?),
        Language::Php => Ok(pipelines::complexity::php::complexity_pipelines()?),
        Language::C => Ok(pipelines::complexity::c::complexity_pipelines()?),
        Language::Cpp => Ok(pipelines::complexity::cpp::complexity_pipelines()?),
        Language::CSharp => Ok(pipelines::complexity::csharp::complexity_pipelines()?),
        Language::Rust => Ok(pipelines::complexity::rust::complexity_pipelines()?),
        Language::Go => Ok(pipelines::complexity::go::complexity_pipelines()?),
        _ => Ok(vec![]),
    }
}

pub fn supported_complexity_languages() -> Vec<Language> {
    vec![
        Language::Java, Language::JavaScript, Language::TypeScript, Language::Tsx,
        Language::Python, Language::Php, Language::C, Language::Cpp, Language::CSharp,
        Language::Rust, Language::Go,
    ]
}
