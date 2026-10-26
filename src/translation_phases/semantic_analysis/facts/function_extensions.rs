//! Retained Clang function extensions. Ordinary function types retain their C
//! parameter list; these sparse records own overload identity, deferred call
//! predicates and the additional `size_t` argument associated with a pointer.
//! Lowering consumes the checked records and never reads attribute tokens.
//! C99: §6.5.2.2p6-7, pp. 71-72; PDF pp. 83-84 (call conversions, extended
//! here by Clang's `overloadable`/`pass_object_size` family).
//! <https://clang.llvm.org/docs/AttributeReference.html#overloadable>
//! <https://clang.llvm.org/docs/AttributeReference.html#pass-object-size-pass-dynamic-object-size>

use rustc_hash::FxBuildHasher;

use crate::{
    translation_phases::{
        SourceVectors,
        parsing::syntax::Expression,
        semantic_analysis::{
            Parameter,
            TypeId,
        },
    },
    util::{
        arena_list::ArenaList,
        bump::{
            ArenaMap,
            Bump,
        },
        string_cache::StringCacheId,
    },
};

/// Sparse facts: an ordinary function and ordinary call add no record.
#[derive(Debug)]
pub(crate) struct FunctionExtensions<'tu> {
    pub(in crate::translation_phases::semantic_analysis) functions:
        ArenaMap<'tu, usize, &'tu FunctionExtension<'tu>>,
    pub(in crate::translation_phases::semantic_analysis) calls:
        ArenaMap<'tu, usize, ExtensionCall<'tu>>,
    pub(in crate::translation_phases::semantic_analysis) values: ArenaMap<'tu, usize, usize>,
    /// Anonymous tag ordinals, assigned on first mangled use per enclosing
    /// function; owner zero denotes file scope.
    pub(in crate::translation_phases::semantic_analysis) anonymous_names:
        ArenaMap<'tu, usize, (usize, usize)>,
    /// Named local tags with the same spelling need a discriminator within
    /// their enclosing function, assigned on first mangled use.
    pub(in crate::translation_phases::semantic_analysis) local_names:
        ArenaMap<'tu, usize, (usize, StringCacheId, usize)>,
    /// Emitted names indexed by the stable semantic overload identity.
    pub(in crate::translation_phases::semantic_analysis) abi_names:
        ArenaMap<'tu, StringCacheId, StringCacheId>,
    pub(in crate::translation_phases::semantic_analysis) written_types:
        ArenaMap<'tu, usize, ArenaList<'tu, WrittenFunction<'tu>>>,
    pub(in crate::translation_phases::semantic_analysis) written_fields:
        ArenaMap<'tu, (usize, StringCacheId), ArenaList<'tu, WrittenFunction<'tu>>>,
    pub(in crate::translation_phases::semantic_analysis) written_type_names:
        ArenaMap<'tu, usize, ArenaList<'tu, WrittenFunction<'tu>>>,
    /// Only a direct typedef in the anonymous tag's defining declaration
    /// grants that tag a typedef name for ABI purposes.
    pub(in crate::translation_phases::semantic_analysis) typedef_names:
        ArenaMap<'tu, usize, StringCacheId>,
    pub(in crate::translation_phases::semantic_analysis) object_sizes:
        ArenaMap<'tu, usize, ObjectSize<'tu>>,
    /// Hidden size of a cleanup call's synthetic address argument, keyed by
    /// the automatic object's binding rather than expression syntax.
    pub(in crate::translation_phases::semantic_analysis) cleanup_sizes:
        ArenaMap<'tu, usize, ObjectSize<'tu>>,
}

impl<'tu> FunctionExtensions<'tu> {
    pub(in crate::translation_phases::semantic_analysis) fn new(arena: &'tu Bump) -> Self {
        Self {
            functions:          ArenaMap::with_hasher_in(FxBuildHasher, arena),
            calls:              ArenaMap::with_hasher_in(FxBuildHasher, arena),
            values:             ArenaMap::with_hasher_in(FxBuildHasher, arena),
            anonymous_names:    ArenaMap::with_hasher_in(FxBuildHasher, arena),
            local_names:        ArenaMap::with_hasher_in(FxBuildHasher, arena),
            abi_names:          ArenaMap::with_hasher_in(FxBuildHasher, arena),
            written_types:      ArenaMap::with_hasher_in(FxBuildHasher, arena),
            written_fields:     ArenaMap::with_hasher_in(FxBuildHasher, arena),
            written_type_names: ArenaMap::with_hasher_in(FxBuildHasher, arena),
            typedef_names:      ArenaMap::with_hasher_in(FxBuildHasher, arena),
            object_sizes:       ArenaMap::with_hasher_in(FxBuildHasher, arena),
            cleanup_sizes:      ArenaMap::with_hasher_in(FxBuildHasher, arena),
        }
    }

    pub(crate) fn function(&self, binding: usize) -> Option<&'tu FunctionExtension<'tu>> {
        self.functions.get(&binding).copied()
    }

    pub(crate) fn call(&self, expression: &Expression<'tu>) -> Option<ExtensionCall<'tu>> {
        self.calls.get(&identity(expression)).copied()
    }

    pub(crate) fn value(&self, expression: &Expression<'tu>) -> Option<usize> {
        self.values.get(&identity(expression)).copied()
    }

    pub(crate) fn object_size(&self, expression: &Expression<'tu>) -> Option<ObjectSize<'tu>> {
        self.object_sizes.get(&identity(expression)).copied()
    }

    pub(crate) fn cleanup_size(&self, variable: usize) -> Option<ObjectSize<'tu>> {
        self.cleanup_sizes.get(&variable).copied()
    }
}

/// A declaration's extension contract. Compatible redeclarations share the
/// entity's assembler identity while predicates keep their own provenance.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FunctionExtension<'tu> {
    pub(crate) overloadable:        bool,
    pub(crate) size_type:           TypeId,
    /// The mangled identity, or the original identifier for an unmangled
    /// function with call predicates or size parameters.
    pub(crate) identity:            StringCacheId,
    /// Emitted ABI symbol; semantic identity stays independent when the
    /// Microsoft ABI, or an unmarked C predicate, omits overload distinctions.
    pub(crate) abi_name:            StringCacheId,
    /// Written parameter types preserve Microsoft ABI qualifier/array sugar.
    pub(crate) declared_parameters: &'tu [Parameter<'tu>],
    pub(crate) written_functions:   ArenaList<'tu, WrittenFunction<'tu>>,
    pub(crate) sizes:               &'tu [SizeParameter],
    /// Prototype bindings in source parameter order. Predicates refer to
    /// these bindings; call substitution uses their ordinal, never spelling.
    pub(crate) parameters:          &'tu [Option<usize>],
    pub(crate) predicates:          &'tu [CallPredicate<'tu>],
    pub(crate) unavailable:         Option<AttributeMessage>,
}

/// Microsoft names retain written parameter qualifiers in nested function
/// types. Depth follows pointer/array/function-result derivations from the
/// original type; semantic type identity remains canonical.
#[derive(Debug, Clone, Copy)]
pub(crate) struct WrittenFunction<'tu> {
    pub(crate) depth:      usize,
    pub(crate) parameters: &'tu [Parameter<'tu>],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SizeParameter {
    pub(crate) index:   u32,
    pub(crate) mode:    u8,
    pub(crate) dynamic: bool,
    pub(crate) source:  SourceVectors,
}

/// A checked expression in the declaring prototype's parameter environment.
/// Its value is reconsidered for each call with actual arguments substituted.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CallPredicate<'tu> {
    pub(crate) parameters: &'tu [Option<usize>],
    pub(crate) expression: &'tu Expression<'tu>,
    pub(crate) kind:       PredicateKind,
    pub(crate) message:    AttributeMessage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PredicateKind {
    Enable,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AttributeMessage {
    pub(crate) text:   StringCacheId,
    pub(crate) source: SourceVectors,
}

/// The selected extension callee and the hidden arguments, in source pointer
/// parameter order. `CallFacts` separately retains the written arguments and
/// their C conversions.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ExtensionCall<'tu> {
    pub(crate) binding: usize,
    pub(crate) sizes:   &'tu [SizeArgument<'tu>],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SizeArgument<'tu> {
    pub(crate) parameter: u32,
    pub(crate) value:     ObjectSize<'tu>,
}

/// A checked object-size value. Dynamic remainders use an already evaluated
/// pointer at calls; a query schedules its pointer only when that remainder
/// needs it. Array extents use the shared `SizeFacts` DAG and saved VLA bounds.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ObjectSize<'tu> {
    Unknown,
    Constant(u64),
    Dynamic {
        ty:     TypeId,
        offset: u64,
    },
    Remainder {
        base: &'tu Expression<'tu>,
        ty:   TypeId,
    },
    Incoming {
        binding: usize,
        offset:  u64,
    },
}

pub(in crate::translation_phases::semantic_analysis) fn identity(
    expression: &Expression<'_>,
) -> usize {
    std::ptr::from_ref(expression).addr()
}
