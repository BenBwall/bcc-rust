//! What an x86 `target` attribute asks of the code lowering emits. Semantic
//! analysis read the attribute
//! (`semantic_analysis/attributes/function_target.rs`); the IR function carries
//! its features to the back ends (`symbols.rs`). Here lowering refuses the
//! emitted functions it cannot honor, and the calls both compilers refuse,
//! which they check only in code they emit:
//!
//! - A direct call of an `always_inline` function whose features the calling
//!   function lacks, when either has a `target` attribute: the function would
//!   be expanded into code that may not use its instructions. Clang names the
//!   first feature missing (`CodeGenFunction::checkTargetFeatures`); GCC
//!   reports that inlining failed (`ix86_can_inline_p`). Without either
//!   attribute both functions have the command line's features. Intrinsics of
//!   bcc's headers whose functions carry `target` are checked this way.
//! - A call of an x86 builtin that needs a feature the calling function lacks,
//!   which Clang names ("needs target feature"; GCC declares such a builtin
//!   only where a target enables its feature, and bcc gives GCC's words for its
//!   reading). Lowering has no x86 instructions, so a call the function's
//!   features allow is refused as an unsupported builtin.
//! - A function turning off a feature every function bcc compiles uses (x87,
//!   SSE2 and their kin), or asking for general registers only.
//! - A function whose attribute turns AVX or AVX-512 on or off where bcc's own
//!   calls pass a vector of 32 bytes or more in registers, as they do under the
//!   command line's AVX: LLVM would give the function's vectors the registers
//!   its own features allow, and its callers others.
//!
//! GNU extension: GCC manual, "x86 Function Attributes", `target`, and
//! "Common Function Attributes", `always_inline`. Clang extension: Clang
//! Attributes Reference, "target".

use super::{
    Construct,
    FunctionLowerer,
    LoweringError,
    LoweringErrorKind,
    module_items::{
        Unit,
        at,
    },
};
use crate::{
    builtins::{
        BuiltinOperation,
        UnsupportedReason,
    },
    target::FunctionTarget,
    translation_phases::{
        SourceVectors,
        parsing::syntax::Expression,
        semantic_analysis::{
            Callee,
            Inlining,
            Linkage,
            TypeKind,
            UnsupportedAttribute,
        },
    },
};

impl Unit<'_, '_, '_> {
    /// Refuses the definition of function `binding` when its `target`
    /// attribute asks what bcc cannot compile, at its name.
    pub(super) fn check_function_target(&self, binding: usize) -> Result<(), LoweringError> {
        let source = self.sema.bindings[binding].name.source_vectors;
        let Some(target) = self.attribute_target(binding) else {
            return Ok(());
        };
        if target.refused() {
            return Err(LoweringError::unsupported(
                Construct::Attribute(UnsupportedAttribute::TargetBaseline),
                source,
            ));
        }
        // Another object can reach an external function, whose refusal
        // `check_foreign_vectors` gives.
        if self.sema.bindings[binding].linkage == Linkage::Internal
            && self.moves_vector_registers(binding)
        {
            let TypeKind::Function {
                result, parameters, ..
            } = self.sema.types.kind(self.sema.bindings[binding].ty)
            else {
                return Ok(());
            };
            for ty in parameters.iter().copied().chain([result]) {
                if super::wide_vectors::widest_vector(&self.sema.types, ty)
                    .map_err(|kind| at(kind, source))?
                    >= 32
                {
                    return Err(LoweringError::unsupported(
                        Construct::TargetVectorRegisters,
                        source,
                    ));
                }
            }
        }
        Ok(())
    }

    /// Whether function `binding`'s `target` attribute changes the registers
    /// its vectors of 32 bytes and more travel in, in the code LLVM writes
    /// for it, from those bcc's calls use: the command line has AVX, which
    /// gives such vectors registers, and the attribute turns AVX off or
    /// AVX-512 on.
    pub(super) fn moves_vector_registers(&self, binding: usize) -> bool {
        self.context.configuration.code_options().x86_features.avx()
            && self.target_vector_abi(binding)
    }

    /// Function `binding`'s `target` attribute, read over the command line.
    pub(super) fn attribute_target(&self, binding: usize) -> Option<&FunctionTarget> {
        self.sema
            .entity_attributes(binding)
            .and_then(|attributes| attributes.target)
    }
}

impl<'tu> FunctionLowerer<'_, '_, 'tu, '_, '_> {
    /// Refuses a call `e` of an x86 builtin that needs a feature this
    /// function lacks.
    pub(super) fn check_builtin_features(
        &self,
        e: &'tu Expression<'tu>,
    ) -> Result<(), LoweringError> {
        let unit = &*self.unit;
        let Some(Callee::Builtin {
            binding: Some(binding),
            ..
        }) = unit.sema.call_facts(e).map(|call| call.callee)
        else {
            return Ok(());
        };
        let Some(descriptor) = unit.sema.bindings[binding].origin.descriptor() else {
            return Ok(());
        };
        if descriptor.operation != BuiltinOperation::Unsupported(UnsupportedReason::X86) {
            return Ok(());
        }
        let command_line;
        let target = match self
            .record
            .binding
            .and_then(|caller| unit.attribute_target(caller))
        {
            | Some(target) => target,
            | None => {
                command_line = unit
                    .context
                    .configuration
                    .code_options()
                    .x86_features
                    .target_attribute("");
                &command_line
            },
        };
        match target.lacks_builtin_feature(descriptor.spelling) {
            | Some(feature) => Err(at(
                LoweringErrorKind::BuiltinFeature {
                    builtin: descriptor.spelling,
                    feature,
                },
                e.source_vectors,
            )),
            | None => Ok(()),
        }
    }

    /// Refuses a direct call of `always_inline` function `callee` whose
    /// features this function lacks, as both compilers do, when either has a
    /// `target` attribute.
    pub(super) fn check_inlined_features(
        &self,
        callee: usize,
        source: SourceVectors,
    ) -> Result<(), LoweringError> {
        let unit = &*self.unit;
        if unit
            .sema
            .entity_attributes(callee)
            .is_none_or(|attributes| attributes.inlining != Inlining::Always)
        {
            return Ok(());
        }
        let caller = self.record.binding;
        let callee_target = unit.attribute_target(callee);
        let caller_target = caller.and_then(|caller| unit.attribute_target(caller));
        if callee_target.is_none() && caller_target.is_none() {
            return Ok(());
        }
        let command_line = unit
            .context
            .configuration
            .code_options()
            .x86_features
            .target_attribute("");
        let feature = caller_target
            .unwrap_or(&command_line)
            .missing(callee_target.unwrap_or(&command_line));
        match (feature, caller) {
            | (Some(feature), Some(caller)) => Err(at(
                LoweringErrorKind::TargetFeature {
                    callee: unit.sema.bindings[callee].name.name,
                    caller: unit.sema.bindings[caller].name.name,
                    feature,
                },
                source,
            )),
            | _ => Ok(()),
        }
    }
}
