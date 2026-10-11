//! Recycled parse-arena storage for the vectors and boxed state that grammar
//! frames use while they run.
//!
//! Nearly every frame owns a few short vectors (operands, provenance
//! segments, list elements) that it copies into the translation-unit arena
//! when it reduces. These come from the parse arena, which frees nothing until
//! parsing ends, so storage a popped frame dropped would stay behind for the
//! rest of the translation unit. Instead the driver lends each pushed frame
//! spare vectors and takes them back, emptied, when the frame is popped;
//! boxed frames and boxed per-frame state are recycled the same way. The
//! spares never outnumber the most that were lent at once, so this storage is
//! bounded by nesting depth and the longest lists, not by input length.
//!
//! Infrastructure for translation phase 7 syntax analysis (§5.1.1.2
//! paragraph 1, p. 10; PDF p. 22). It encodes no rule of the standard.

use std::{
    cell::Cell,
    mem,
};

use super::{
    Parser,
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
            GnuFrame,
            OffsetMember,
        },
        modern::{
            GenericAssociation,
            ModernFrame,
            ModernKind,
            SyntaxOperand,
        },
        msvc::MsvcFrame,
    },
    frames::{
        expression::{
            CallState,
            ExpressionOperand,
        },
        expression_operators::LanguageExpressionOperator,
        initializer::DesignationState,
        parameter_list::ParameterListFrame,
        struct_or_union::StructOrUnionSpecifierFrame,
    },
    machine::ParseFrame,
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

/// Spare frame storage in the parse arena, one pool per element type.
pub(super) struct FramePools<'tu, 'p> {
    arena: &'p Bump,
    pub(super) source_vectors: VecPool<'p, SourceVectors>,
    pub(super) operands: VecPool<'p, ExpressionOperand<'tu>>,
    pub(super) operators: VecPool<'p, LanguageExpressionOperator<'tu>>,
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
    pub(super) delimiters: VecPool<'p, OperatorTokenType>,
    pub(super) offset_members: VecPool<'p, OffsetMember<'tu>>,
    pub(super) gnu_frames: BoxPool<'p, GnuFrame<'tu, 'p>>,
    pub(super) modern_frames: BoxPool<'p, ModernFrame<'tu, 'p>>,
    pub(super) msvc_frames: BoxPool<'p, MsvcFrame<'tu, 'p>>,
    pub(super) parameter_lists: BoxPool<'p, ParameterListFrame<'tu, 'p>>,
    pub(super) struct_or_union_specifiers: BoxPool<'p, StructOrUnionSpecifierFrame<'tu, 'p>>,
    /// Flags shared along a chain of parenthesized declarators.
    chain_flags: ArenaVec<'p, &'p Cell<bool>>,
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

/// An owned value in the parse arena. Frames keep one only while the pools
/// can take it back, so its storage is reused rather than abandoned.
pub(super) type PoolBox<'p, T> = allocator_api2::boxed::Box<T, &'p Bump>;

/// Spare vectors of one element type.
pub(super) struct VecPool<'p, T> {
    spare: ArenaVec<'p, ArenaVec<'p, T>>,
}

/// Spare boxes of one type, reused whole.
pub(super) struct BoxPool<'p, T> {
    spare: ArenaVec<'p, PoolBox<'p, T>>,
}

impl<'tu, 'p> Parser<'_, 'tu, 'p> {
    pub(super) fn push_frame(&mut self, mut frame: ParseFrame<'tu, 'p>) {
        frame.lend_pooled(&mut self.pools);
        self.retained_frame_nodes = self
            .retained_frame_nodes
            .checked_add(frame.retained_node_count())
            .expect("retained syntax node count overflows usize");
        self.frames.push(frame);
    }

    pub(super) fn pop_frame(&mut self) -> ParseFrame<'tu, 'p> {
        let frame = self.frames.pop().expect("parser frame stack is nonempty");
        self.retained_frame_nodes = self
            .retained_frame_nodes
            .checked_sub(frame.retained_node_count())
            .expect("pending syntax count matches the frame stack");
        frame
    }

    /// Reuses the parser arena's pooled storage for a modern syntax frame.
    pub(super) fn pooled_modern_frame(&mut self, kind: ModernKind) -> ParseFrame<'tu, 'p> {
        ParseFrame::Modern(self.pools.modern(ModernFrame::new(
            self.arena,
            kind,
            self.hard_error_count,
        )))
    }
}

impl<'p, T> VecPool<'p, T> {
    fn new_in(arena: &'p Bump) -> Self {
        Self {
            spare: ArenaVec::new_in(arena),
        }
    }
}

impl<'p, T> BoxPool<'p, T> {
    /// Boxes `value`, reusing a spare box when one exists.
    fn boxed(&mut self, arena: &'p Bump, value: T) -> PoolBox<'p, T> {
        match self.spare.pop() {
            | Some(mut spare) => {
                *spare = value;
                spare
            },
            | None => PoolBox::new_in(value, arena),
        }
    }

    fn new_in(arena: &'p Bump) -> Self {
        Self {
            spare: ArenaVec::new_in(arena),
        }
    }
}

impl<'tu, 'p> FramePools<'tu, 'p> {
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

    pub(super) fn gnu(&mut self, frame: GnuFrame<'tu, 'p>) -> PoolBox<'p, GnuFrame<'tu, 'p>> {
        self.gnu_frames.boxed(self.arena, frame)
    }

    pub(super) fn modern(
        &mut self,
        frame: ModernFrame<'tu, 'p>,
    ) -> PoolBox<'p, ModernFrame<'tu, 'p>> {
        self.modern_frames.boxed(self.arena, frame)
    }

    pub(super) fn msvc(&mut self, frame: MsvcFrame<'tu, 'p>) -> PoolBox<'p, MsvcFrame<'tu, 'p>> {
        self.msvc_frames.boxed(self.arena, frame)
    }

    pub(super) fn parameter_list(
        &mut self,
        frame: ParameterListFrame<'tu, 'p>,
    ) -> PoolBox<'p, ParameterListFrame<'tu, 'p>> {
        self.parameter_lists.boxed(self.arena, frame)
    }

    pub(super) fn struct_or_union_specifier(
        &mut self,
        frame: StructOrUnionSpecifierFrame<'tu, 'p>,
    ) -> PoolBox<'p, StructOrUnionSpecifierFrame<'tu, 'p>> {
        self.struct_or_union_specifiers.boxed(self.arena, frame)
    }

    /// A cleared flag for a new chain of parenthesized declarators.
    pub(super) fn take_chain_flag(&mut self) -> &'p Cell<bool> {
        let flag = self
            .chain_flags
            .pop()
            .unwrap_or_else(|| self.arena.alloc(Cell::new(false)));
        flag.set(false);
        flag
    }

    /// Takes back a chain flag once no declarator of its chain remains.
    pub(super) fn reclaim_chain_flag(&mut self, flag: &'p Cell<bool>) {
        self.chain_flags.push(flag);
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
            parameter_lists: BoxPool::new_in(arena),
            gnu_frames: BoxPool::new_in(arena),
            opaque_tokens: VecPool::new_in(arena),
            asm_operands: VecPool::new_in(arena),
            builtin_operands: VecPool::new_in(arena),
            modern_associations: VecPool::new_in(arena),
            delimiters: VecPool::new_in(arena),
            offset_members: VecPool::new_in(arena),
            modern_frames: BoxPool::new_in(arena),
            msvc_frames: BoxPool::new_in(arena),
            struct_or_union_specifiers: BoxPool::new_in(arena),
            chain_flags: ArenaVec::new_in(arena),
        }
    }
}
