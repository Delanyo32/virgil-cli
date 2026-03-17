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

pub mod cognitive;
pub mod comment_ratio;
pub mod cyclomatic;
pub mod function_length;

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
