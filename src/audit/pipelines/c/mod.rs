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

pub mod coupling;
pub mod dead_code;
pub mod duplicate_code;

use crate::audit::pipeline::{AnyPipeline, Pipeline};
use anyhow::Result;

pub fn tech_debt_pipelines() -> Result<Vec<AnyPipeline>> {
    Ok(vec![
        AnyPipeline::Graph(Box::new(buffer_overflows::BufferOverflowsPipeline::new()?)),
        AnyPipeline::Graph(Box::new(unchecked_malloc::UncheckedMallocPipeline::new()?)),
        AnyPipeline::Graph(Box::new(memory_leaks::MemoryLeaksPipeline::new()?)),
        AnyPipeline::Graph(Box::new(signed_unsigned_mismatch::SignedUnsignedMismatchPipeline::new()?)),
        AnyPipeline::Graph(Box::new(magic_numbers::CMagicNumbersPipeline::new()?)),
        AnyPipeline::Graph(Box::new(global_mutable_state::GlobalMutableStatePipeline::new()?)),
        AnyPipeline::Graph(Box::new(typedef_pointer_hiding::TypedefPointerHidingPipeline::new()?)),
        AnyPipeline::Graph(Box::new(define_instead_of_inline::DefineInsteadOfInlinePipeline::new()?)),
        AnyPipeline::Graph(Box::new(ignored_return_values::IgnoredReturnValuesPipeline::new()?)),
        AnyPipeline::Graph(Box::new(void_pointer_abuse::VoidPointerAbusePipeline::new()?)),
        AnyPipeline::Graph(Box::new(missing_const::MissingConstPipeline::new()?)),
        AnyPipeline::Graph(Box::new(raw_struct_serialization::RawStructSerializationPipeline::new()?)),
    ])
}

pub fn complexity_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![])
}

pub fn code_style_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![
        Box::new(dead_code::DeadCodePipeline::new()?),
        Box::new(duplicate_code::DuplicateCodePipeline::new()?),
        Box::new(coupling::CouplingPipeline::new()?),
    ])
}

pub fn security_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![])
}

pub fn scalability_pipelines() -> Result<Vec<Box<dyn Pipeline>>> {
    Ok(vec![])
}

