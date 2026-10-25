//! Recycled parse-arena storage for the vectors and boxed state that grammar
//! productions use while they run.
//!
//! Nearly every production keeps a few short vectors (operands, provenance
//! segments, list elements) that it copies into the translation-unit arena
//! when it completes. These come from the parse arena, which frees nothing
//! until parsing ends, so storage a returning production dropped would stay
//! behind for the rest of the translation unit. Instead each production
//! borrows spare vectors on entry and gives them back, emptied, before it
//! returns; boxed per-production state is recycled the same way. The spares
//! never outnumber the most that were lent at once, so this storage is
//! bounded by nesting depth and the longest lists, not by input length.
//!
//! Infrastructure for translation phase 7 syntax analysis (§5.1.1.2
//! paragraph 1, p. 10; PDF p. 22). It encodes no rule of the standard.

use std::mem;

use super::{
    declaration_syntax::{
        Declaration,
        DirectDeclarator,
        Enumerator,
        InitDeclarator,
        InitializerElement,
        ParameterDeclaration,
        PointerLevel,
        StructDeclaration,
        StructDeclarator,
    },
    extensions::{
        gnu::{
            AsmOperand,
            OffsetMember,
        },
        modern::{
            AttributeArgument,
            GenericAssociation,
            SyntaxOperand,
        },
    },
    grammar::{
        CallState,
        DesignationState,
        ExpressionOperand,
        expression_operators::LanguageExpressionOperator,
    },
    syntax::{
        BlockItem,
        Identifier,
    },
};
use crate::{
    translation_phases::{
        SourceVectors,
        preprocessing::OperatorTokenType,
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// Spare production storage in the parse arena, one pool per element type.
pub(super) struct ScratchPools<'tu, 'p> {
    arena: &'p Bump,
    pub(super) source_vectors: VecPool<'p, SourceVectors>,
    pub(super) operands: VecPool<'p, ExpressionOperand<'tu>>,
    pub(super) operators: VecPool<'p, LanguageExpressionOperator>,
    pub(super) pointer_levels: VecPool<'p, PointerLevel<'tu>>,
    pub(super) direct_declarators: VecPool<'p, DirectDeclarator<'tu>>,
    pub(super) init_declarators: VecPool<'p, InitDeclarator<'tu>>,
    pub(super) block_items: VecPool<'p, BlockItem<'tu>>,
    pub(super) initializers: VecPool<'p, InitializerElement<'tu>>,
    pub(super) parameters: VecPool<'p, ParameterDeclaration<'tu>>,
    pub(super) identifiers: VecPool<'p, Identifier>,
    pub(super) struct_members: VecPool<'p, StructDeclaration<'tu>>,
    pub(super) struct_declarators: VecPool<'p, StructDeclarator<'tu>>,
    pub(super) enumerators: VecPool<'p, Enumerator<'tu>>,
    pub(super) declarations: VecPool<'p, &'tu Declaration<'tu>>,
    /// Call states, reused with the capacity of their lists.
    pub(super) calls: BoxPool<'p, CallState<'tu, 'p>>,
    /// Designation states, reused with the capacity of their lists.
    pub(super) designations: BoxPool<'p, DesignationState<'tu, 'p>>,
    pub(super) opaque_tokens: VecPool<'p, crate::translation_phases::preprocessing::Token>,
    pub(super) asm_operands: VecPool<'p, AsmOperand<'tu>>,
    pub(super) builtin_operands: VecPool<'p, SyntaxOperand<'tu>>,
    pub(super) modern_associations: VecPool<'p, GenericAssociation<'tu>>,
    pub(super) attribute_arguments: VecPool<'p, AttributeArgument<'tu>>,
    pub(super) delimiters: VecPool<'p, OperatorTokenType>,
    pub(super) offset_members: VecPool<'p, OffsetMember<'tu>>,
}

impl<'p, T> VecPool<'p, T> {
    /// Gives `vector` a spare allocation if it has none yet.
    pub(super) fn lend(&mut self, vector: &mut ArenaVec<'p, T>) {
        if vector.capacity() == 0
            && let Some(spare) = self.spare.pop()
        {
            *vector = spare;
        }
    }

    /// Takes back `vector`'s allocation, discarding its contents. Every
    /// allocation is kept, since the arena could not reuse a dropped one.
    pub(super) fn reclaim(&mut self, vector: &mut ArenaVec<'p, T>) {
        if vector.capacity() != 0 {
            vector.clear();
            let empty = ArenaVec::new_in(*vector.allocator());
            self.spare.push(mem::replace(vector, empty));
        }
    }
}

impl<'p, T> BoxPool<'p, T> {
    /// Takes back a box that its owner no longer needs.
    pub(super) fn reclaim(&mut self, boxed: PoolBox<'p, T>) {
        self.spare.push(boxed);
    }

    /// A spare box, or a new one holding `make()`. A spare keeps whatever
    /// its last owner left in it.
    fn take(&mut self, arena: &'p Bump, make: impl FnOnce() -> T) -> PoolBox<'p, T> {
        self.spare
            .pop()
            .unwrap_or_else(|| PoolBox::new_in(make(), arena))
    }
}

/// An owned value in the parse arena. Productions keep one only while the
/// pools can take it back, so its storage is reused rather than abandoned.
pub(super) type PoolBox<'p, T> = allocator_api2::boxed::Box<T, &'p Bump>;

/// Spare vectors of one element type.
pub(super) struct VecPool<'p, T> {
    spare: ArenaVec<'p, ArenaVec<'p, T>>,
}

/// Spare boxes of one type, reused whole.
pub(super) struct BoxPool<'p, T> {
    spare: ArenaVec<'p, PoolBox<'p, T>>,
}

impl<'p, T> VecPool<'p, T> {
    fn new_in(arena: &'p Bump) -> Self {
        Self {
            spare: ArenaVec::new_in(arena),
        }
    }
}

impl<'p, T> BoxPool<'p, T> {
    fn new_in(arena: &'p Bump) -> Self {
        Self {
            spare: ArenaVec::new_in(arena),
        }
    }
}

impl<'tu, 'p> ScratchPools<'tu, 'p> {
    /// An emptied call state.
    pub(super) fn take_call(&mut self) -> PoolBox<'p, CallState<'tu, 'p>> {
        let arena = self.arena;
        self.calls.take(arena, || CallState::new_in(arena))
    }

    /// A designation state with no designation under way.
    pub(super) fn take_designation(&mut self) -> PoolBox<'p, DesignationState<'tu, 'p>> {
        let arena = self.arena;
        self.designations
            .take(arena, || DesignationState::new_in(arena))
    }

    pub(super) fn new_in(arena: &'p Bump) -> Self {
        Self {
            arena,
            source_vectors: VecPool::new_in(arena),
            operands: VecPool::new_in(arena),
            operators: VecPool::new_in(arena),
            pointer_levels: VecPool::new_in(arena),
            direct_declarators: VecPool::new_in(arena),
            init_declarators: VecPool::new_in(arena),
            block_items: VecPool::new_in(arena),
            initializers: VecPool::new_in(arena),
            parameters: VecPool::new_in(arena),
            identifiers: VecPool::new_in(arena),
            struct_members: VecPool::new_in(arena),
            struct_declarators: VecPool::new_in(arena),
            enumerators: VecPool::new_in(arena),
            declarations: VecPool::new_in(arena),
            calls: BoxPool::new_in(arena),
            designations: BoxPool::new_in(arena),
            opaque_tokens: VecPool::new_in(arena),
            asm_operands: VecPool::new_in(arena),
            builtin_operands: VecPool::new_in(arena),
            modern_associations: VecPool::new_in(arena),
            attribute_arguments: VecPool::new_in(arena),
            delimiters: VecPool::new_in(arena),
            offset_members: VecPool::new_in(arena),
        }
    }
}
