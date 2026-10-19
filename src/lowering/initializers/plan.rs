//! Emitting a retained initializer plan as leaves. Semantic analysis chose
//! every subobject; this replays its operations in source order, repeating a
//! range's operations for each of its elements with an explicit cursor, so
//! nothing proportional to a range's extent is retained before emission.
//! A union selection replaces the previously selected member's bytes, which
//! appears as a reset [`Leaf::Zero`] when the member changes; a whole copy
//! forgets the selections inside it. Lowering refuses a destination type it
//! cannot represent where the plan reaches it.
//!
//! C99: §6.7.8 paragraphs 17-21, pp. 126-127; PDF pp. 138-139.
//! GNU extension: GCC manual, "Designated Initializers" (ranges).
//! <https://gcc.gnu.org/onlinedocs/gcc/Designated-Inits.html>

use super::{
    Construct,
    Leaf,
    LoweringError,
    at,
    types::{
        self,
        repr,
    },
};
use crate::{
    lowering::work::BitFieldBits,
    translation_phases::{
        SourceVectors,
        semantic_analysis::{
            InitOperation,
            InitializerPlan,
            SemanticTranslationUnit,
            TypeId,
        },
    },
    util::bump::{
        ArenaMap,
        ArenaVec,
        Bump,
    },
};

/// The stores of `plan`, in source order, as leaves at byte offsets from the
/// object's start.
/// C99: §6.7.8 paragraphs 14-19, pp. 126-127; PDF pp. 138-139.
pub(super) fn plan_leaves<'tu, 's>(
    sema: &SemanticTranslationUnit<'tu>,
    plan: &InitializerPlan<'tu>,
    arena: &'s Bump,
) -> Result<ArenaVec<'s, Leaf<'tu>>, LoweringError> {
    let operations = plan.operations;
    let mut leaves = ArenaVec::with_capacity_in(operations.len(), arena);
    // The leaf of each brace pair's zero scope, by operation index.
    let mut scopes = ArenaVec::with_capacity_in(operations.len(), arena);
    scopes.resize(operations.len(), usize::MAX);
    // Brace pairs whose list has not ended: their end and their leaf.
    let mut open: ArenaVec<'_, (usize, usize)> = ArenaVec::new_in(arena);
    let mut repeats: ArenaVec<'_, Repeat> = ArenaVec::new_in(arena);
    // Each union's selected member, by its offset and type; the type tells
    // nested unions at one offset apart.
    let mut selected = ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, arena);
    let mut shift = 0_u64;
    let mut index = 0;
    loop {
        if let Some(repeat) = repeats.last_mut()
            && index == repeat.end
        {
            close(&mut leaves, &mut open, repeat.open);
            repeat.position += 1;
            if repeat.position < repeat.count {
                shift += repeat.stride;
                index = repeat.start;
            } else {
                shift = repeat.shift;
                _ = repeats.pop();
            }
            continue;
        }
        while let Some(&(end, leaf)) = open.last()
            && end <= index
        {
            set_end(&mut leaves, leaf);
            _ = open.pop();
        }
        let Some(&operation) = operations.get(index) else {
            break;
        };
        index += 1;
        let place = |offset: u64, source: SourceVectors| {
            offset
                .checked_add(shift)
                .ok_or_else(|| LoweringError::unsupported(Construct::Initializer, source))
        };
        let check = |ty: TypeId, source: SourceVectors| {
            repr(&sema.types, ty)
                .map(types::Repr::is_scalar)
                .map_err(|kind| at(kind, source))
        };
        let size = |ty: TypeId, source: SourceVectors| {
            types::layout(&sema.types, ty)
                .map(|layout| layout.size)
                .map_err(|kind| at(kind, source))
        };
        match operation {
            | InitOperation::Zero {
                destination,
                end,
                source,
            } => {
                _ = check(destination.ty, source)?;
                let leaf = leaves.len();
                leaves.push(Leaf::Zero {
                    offset: place(destination.offset, source)?,
                    size:   size(destination.ty, source)?,
                    ty:     destination.ty,
                    end:    usize::MAX,
                    scope:  leaf,
                });
                scopes[index - 1] = leaf;
                open.push((end as usize, leaf));
            },
            | InitOperation::Select {
                destination,
                member,
                scope,
                source,
            } => {
                let offset = place(destination.offset, source)?;
                let previous = selected.insert((offset, destination.ty.index), member);
                if previous.is_some_and(|previous| previous != member) {
                    let scope = scopes
                        .get(scope as usize)
                        .copied()
                        .filter(|&scope| scope != usize::MAX)
                        .ok_or_else(|| {
                            LoweringError::missing("a union selection has no brace scope", source)
                        })?;
                    leaves.push(Leaf::Zero {
                        offset,
                        size: size(destination.ty, source)?,
                        ty: destination.ty,
                        end: usize::MAX,
                        scope,
                    });
                }
            },
            | InitOperation::Store {
                value,
                destination,
                bit_field,
                source,
            } => {
                if !check(destination.ty, source)? {
                    return Err(LoweringError::missing(
                        "an initializer stores a value into a non-scalar",
                        source,
                    ));
                }
                leaves.push(Leaf::Scalar {
                    offset: place(destination.offset, source)?,
                    ty: destination.ty,
                    expression: plan.values[value.0 as usize].expression,
                    value,
                    bit_field: bit_field.map(|bits| BitFieldBits {
                        bit_offset: bits.bit_offset,
                        width:      bits.width,
                    }),
                });
            },
            | InitOperation::Copy {
                value,
                destination,
                enclosing_unions,
            } => {
                let expression = plan.values[value.0 as usize].expression;
                let source = expression.source_vectors;
                _ = check(destination.ty, source)?;
                let offset = place(destination.offset, source)?;
                let end = offset + size(destination.ty, source)?;
                // C99 §6.7.8p19: the copy replaces every member selected
                // inside it; the unions it is a member of keep theirs.
                selected.retain(|&(at, ty), _| {
                    at < offset
                        || at >= end
                        || enclosing_unions.iter().any(|union| union.index == ty)
                });
                leaves.push(Leaf::Copy {
                    offset,
                    ty: destination.ty,
                    expression,
                    value,
                });
            },
            | InitOperation::String {
                literal,
                destination,
            } => {
                let source = literal.source_vectors;
                _ = check(destination.ty, source)?;
                leaves.push(Leaf::String {
                    ty: destination.ty,
                    offset: place(destination.offset, source)?,
                    size: sema
                        .types
                        .layout(destination.ty)
                        .ok_or_else(|| LoweringError::missing("an array has no size", source))?
                        .size,
                    literal,
                });
            },
            | InitOperation::Repeat {
                count,
                stride,
                length,
            } =>
                if length > 0 && count > 0 {
                    repeats.push(Repeat {
                        start: index,
                        end: index + length as usize,
                        count,
                        stride,
                        position: 0,
                        shift,
                        open: open.len(),
                    });
                } else {
                    index += length as usize;
                },
            | InitOperation::EmptyScalar {
                destination,
                source,
            } => {
                _ = check(destination.ty, source)?;
                return Err(LoweringError::unsupported(Construct::Initializer, source));
            },
            | InitOperation::Unmodeled {
                destination,
                source,
            } => {
                _ = check(destination.ty, source)?;
                return Err(LoweringError::missing(
                    "an initializer places a value inside an unmodeled subobject",
                    source,
                ));
            },
        }
    }
    close(&mut leaves, &mut open, 0);
    Ok(leaves)
}

/// A range repeat being emitted: its operations, its elements, and the
/// state to restore after it.
struct Repeat {
    start:    usize,
    end:      usize,
    count:    u64,
    stride:   u64,
    position: u64,
    /// The shift before the repeat.
    shift:    u64,
    /// The open brace pairs before the repeat.
    open:     usize,
}

/// Ends every brace pair above the first `keep`.
fn close(leaves: &mut [Leaf<'_>], open: &mut ArenaVec<'_, (usize, usize)>, keep: usize) {
    while open.len() > keep {
        let (_, leaf) = open.pop().expect("an open brace pair");
        set_end(leaves, leaf);
    }
}

fn set_end(leaves: &mut [Leaf<'_>], leaf: usize) {
    let length = leaves.len();
    if let Leaf::Zero { end, .. } = &mut leaves[leaf] {
        *end = length;
    }
}
