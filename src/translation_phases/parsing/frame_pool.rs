//! Recycled storage for the vectors grammar frames grow while they run.
//!
//! Nearly every frame owns a few short vectors (operands, provenance
//! segments, list elements) that it drains into the syntax arenas when it
//! reduces. Allocating them afresh for each frame dominated small-frame
//! cost, so the driver lends each pushed frame spare vectors and takes them
//! back, emptied, when the frame is popped.

use std::mem;

use super::{
    declaration_syntax::{
        DirectDeclarator,
        Enumerator,
        InitDeclarator,
        InitializerElement,
        ParameterDeclaration,
        StructDeclaration,
        StructDeclarator,
        TypeQualifiers,
    },
    expression::{
        CallState,
        ExpressionOperand,
    },
    expression_operators::LanguageExpressionOperator,
    syntax::{
        BlockItem,
        Identifier,
    },
};
use crate::translation_phases::SourceVectors;

/// Spare vectors of one element type.
pub(super) struct VecPool<T> {
    spare: Vec<Vec<T>>,
}

impl<T> Default for VecPool<T> {
    fn default() -> Self {
        Self { spare: Vec::new() }
    }
}

impl<T> VecPool<T> {
    /// Larger vectors are freed rather than kept, so one huge list does not
    /// pin its memory for the rest of the translation unit.
    const MAX_CAPACITY: usize = 1 << 10;
    /// Spare vectors kept per element type; deeper nesting allocates.
    const MAX_SPARE: usize = 64;

    /// Gives `vector` a spare allocation if it has none yet.
    pub(super) fn lend(&mut self, vector: &mut Vec<T>) {
        if vector.capacity() == 0
            && let Some(spare) = self.spare.pop()
        {
            *vector = spare;
        }
    }

    /// Takes back `vector`'s allocation, discarding its contents.
    pub(super) fn reclaim(&mut self, vector: &mut Vec<T>) {
        if vector.capacity() != 0
            && vector.capacity() <= Self::MAX_CAPACITY
            && self.spare.len() < Self::MAX_SPARE
        {
            vector.clear();
            self.spare.push(mem::take(vector));
        }
    }
}

/// Spare frame vectors, one pool per element type.
#[derive(Default)]
pub(super) struct FramePools {
    pub(super) source_vectors:     VecPool<SourceVectors>,
    pub(super) operands:           VecPool<ExpressionOperand>,
    pub(super) operators:          VecPool<LanguageExpressionOperator>,
    pub(super) pointer_qualifiers: VecPool<TypeQualifiers>,
    pub(super) direct_declarators: VecPool<DirectDeclarator>,
    pub(super) init_declarators:   VecPool<InitDeclarator>,
    pub(super) block_items:        VecPool<BlockItem>,
    pub(super) initializers:       VecPool<InitializerElement>,
    pub(super) parameters:         VecPool<ParameterDeclaration>,
    pub(super) identifiers:        VecPool<Identifier>,
    pub(super) struct_members:     VecPool<StructDeclaration>,
    pub(super) struct_declarators: VecPool<StructDeclarator>,
    pub(super) enumerators:        VecPool<Enumerator>,
    /// Emptied call states. Expression frames hold them boxed, so keeping
    /// the boxes saves an allocation per call.
    #[expect(
        clippy::vec_box,
        reason = "Expression frames own call states as boxes, which are reused whole."
    )]
    pub(super) calls:              Vec<Box<CallState>>,
}

impl FramePools {
    /// Keeps an emptied call state for the next call expression.
    pub(super) fn reclaim_call(&mut self, call: Box<CallState>) {
        if self.calls.len() < 64 {
            self.calls.push(call);
        }
    }
}
