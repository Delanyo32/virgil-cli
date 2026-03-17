pub mod primitives;

pub mod buffer_overflows;
pub mod define_instead_of_inline;
pub mod global_mutable_state;
pub mod ignored_return_values;
pub mod magic_numbers;
pub mod memory_leaks;
pub mod missing_const;
pub mod raw_struct_serialization;
pub mod signed_unsigned_mismatch;
pub mod typedef_pointer_hiding;
pub mod unchecked_malloc;
pub mod void_pointer_abuse;

pub mod c_buffer_overflow_security;
pub mod c_command_injection;
pub mod c_integer_overflow;
pub mod c_memory_mismanagement;
pub mod c_path_traversal;
pub mod c_toctou;
pub mod c_uninitialized_memory;
pub mod c_weak_randomness;
pub mod format_string;

pub mod cognitive;
pub mod comment_ratio;
pub mod cyclomatic;
pub mod function_length;

pub mod coupling;
pub mod dead_code;
pub mod duplicate_code;

use anyhow::Result;
use crate::audit::pipeline::Pipeline;

pub fn tech_debt_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(buffer_overflows::BufferOverflowsPipeline::new()?),
        Box::new(unchecked_malloc::UncheckedMallocPipeline::new()?),
        Box::new(memory_leaks::MemoryLeaksPipeline::new()?),
        Box::new(signed_unsigned_mismatch::SignedUnsignedMismatchPipeline::new()?),
        Box::new(magic_numbers::CMagicNumbersPipeline::new()?),
        Box::new(global_mutable_state::GlobalMutableStatePipeline::new()?),
        Box::new(typedef_pointer_hiding::TypedefPointerHidingPipeline::new()?),
        Box::new(define_instead_of_inline::DefineInsteadOfInlinePipeline::new()?),
        Box::new(ignored_return_values::IgnoredReturnValuesPipeline::new()?),
        Box::new(void_pointer_abuse::VoidPointerAbusePipeline::new()?),
        Box::new(missing_const::MissingConstPipeline::new()?),
        Box::new(raw_struct_serialization::RawStructSerializationPipeline::new()?),
    ])
}

pub fn complexity_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(cyclomatic::CyclomaticComplexityPipeline::new()?),
        Box::new(function_length::FunctionLengthPipeline::new()?),
        Box::new(cognitive::CognitiveComplexityPipeline::new()?),
        Box::new(comment_ratio::CommentToCodeRatioPipeline::new()?),
    ])
}

pub fn code_style_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(dead_code::DeadCodePipeline::new()?),
        Box::new(duplicate_code::DuplicateCodePipeline::new()?),
        Box::new(coupling::CouplingPipeline::new()?),
    ])
}

pub fn security_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(format_string::FormatStringPipeline::new()?),
        Box::new(c_command_injection::CCommandInjectionPipeline::new()?),
        Box::new(c_weak_randomness::CWeakRandomnessPipeline::new()?),
        Box::new(c_buffer_overflow_security::CBufferOverflowSecurityPipeline::new()?),
        Box::new(c_integer_overflow::CIntegerOverflowPipeline::new()?),
        Box::new(c_toctou::CToctouPipeline::new()?),
        Box::new(c_memory_mismanagement::CMemoryMismanagementPipeline::new()?),
        Box::new(c_path_traversal::CPathTraversalPipeline::new()?),
        Box::new(c_uninitialized_memory::CUninitializedMemoryPipeline::new()?),
    ])
}
