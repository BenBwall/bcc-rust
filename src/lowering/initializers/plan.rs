//! Emitting a retained initializer plan as leaves. Semantic analysis chose
//! every subobject; this replays its operations in source order, repeating a
//! range's operations for each of its elements with an explicit cursor, so
//! nothing proportional to a range's extent is retained before emission.
//! A union selection replaces the previously selected member's bytes, which
//! appears as a reset [`Leaf::Zero`] when the member changes; a whole copy
//! forgets the selections inside it. Lowering refuses a destination type it
//! cannot represent where the plan reaches it.
//!
//! A range's first element is replayed as above. When its leaves write one
//! contiguous run of whole bytes whatever the element held before, and
//! selected no union member, every later element would get the same bytes:
//! one [`Leaf::Replicate`] copies the run instead (`replication`), so the
//! leaves of `{ [0 ... 999][0 ... 999] = 0 }` are one store and two
//! replications. Any other range is replayed element by element.
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
        Repr,
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
            TypeKind,
        },
    },
    util::bump::{
        ArenaMap,
        ArenaVec,
        Bump,
    },
};

/// The stores of `plan`, in source order, as leaves at byte offsets from the
/// object's start, for an object of `storage` duration.
/// C99: §6.7.8 paragraphs 14-19, pp. 126-127; PDF pp. 138-139.
pub(super) fn plan_leaves<'tu, 's>(
    sema: &SemanticTranslationUnit<'tu>,
    plan: &InitializerPlan<'tu>,
    storage: Storage,
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
    // Union selections and whole copies replayed so far.
    let mut selections = 0_usize;
    let mut copies = 0_usize;
    let mut shift = 0_u64;
    let mut index = 0;
    loop {
        #[cfg(test)]
        STEPS.set(STEPS.get() + 1);
        if let Some(repeat) = repeats.last_mut()
            && index == repeat.end
        {
            close(&mut leaves, &mut open, repeat.open);
            if repeat.position == 0 && repeat.count > 1 {
                let first = Element {
                    leaves:   repeat.leaves,
                    selected: selections != repeat.selections,
                    copied:   copies != repeat.copies,
                };
                if let Some(rest) =
                    replication(sema, &leaves, first, repeat, &selected, storage, arena)
                {
                    if let Rest::Copied(leaf) = rest {
                        leaves.push(leaf);
                    }
                    shift = repeat.shift;
                    _ = repeats.pop();
                    continue;
                }
            }
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
        // A wide bit-precise integer is a scalar held in memory.
        let check = |ty: TypeId, source: SourceVectors| {
            repr(&sema.types, ty)
                .map(|representation| {
                    representation.is_scalar()
                        || super::super::wide_bit_int::wide_bit_int(&sema.types, ty).is_some()
                })
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
                selections += 1;
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
                copies += 1;
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
                        leaves: leaves.len(),
                        selections,
                        copies,
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

/// Where the object an initializer stores into lives.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::lowering) enum Storage {
    /// Static storage duration: the leaves become bytes and relocations.
    Static,
    /// Automatic storage duration: the leaves become stores.
    Automatic,
}

/// What a range's first element replayed: its first leaf, and whether it
/// selected a union member or copied a whole structure or union.
#[derive(Clone, Copy)]
struct Element {
    leaves:   usize,
    selected: bool,
    copied:   bool,
}

/// The fewest elements an automatic range copies its first element into;
/// a shorter range keeps one store per element.
const AUTOMATIC_REPLICATION: u64 = 16;

/// How a range continues after its first element, whose leaves are
/// `leaves[first.leaves..]`, if not by replaying each later element.
///
/// A range's values are evaluated once (GNU), so each element's leaves write
/// the same values at offsets `stride` apart, inside the element. They give
/// every element the same bytes and relocations, whatever it held before,
/// when:
/// - they select no union member, since a selection resets the member depending
///   on the element's earlier selection, and later selections compare with it;
/// - any whole copy among them, which forgets the selections inside it, finds
///   none recorded in a later element;
/// - they write one run of whole bytes, which includes every byte whose
///   relocations a bit-field store discards.
///
/// The bytes outside the run keep their earlier values in every element. An
/// automatic copy also needs the run to be the whole element, no volatile
/// leaf, and at least [`AUTOMATIC_REPLICATION`] elements.
/// C99: §6.7.8 paragraphs 19 and 21, p. 127; PDF p. 139.
/// GNU extension: GCC manual, "Designated Initializers" (ranges).
fn replication<'tu>(
    sema: &SemanticTranslationUnit<'tu>,
    leaves: &[Leaf<'tu>],
    first: Element,
    repeat: &Repeat,
    selected: &ArenaMap<'_, (u64, usize), u32>,
    storage: Storage,
    arena: &Bump,
) -> Option<Rest<'tu>> {
    if first.selected || repeat.count < minimum_count(storage) {
        return None;
    }
    let body = &leaves[first.leaves..];
    // Bit ranges written, bytes whose relocations are discarded, and the
    // extents of earlier copies whose runs are not adjacent.
    let mut runs = ArenaVec::with_capacity_in(body.len(), arena);
    let mut contained = ArenaVec::new_in(arena);
    let mut zero = true;
    for &leaf in body {
        if storage == Storage::Automatic && volatile(sema, leaf.ty()) {
            return None;
        }
        match leaf {
            | Leaf::Replicate {
                offset,
                size,
                stride,
                count,
                zero: copied_zero,
                ..
            } => {
                zero &= copied_zero;
                let end = offset.checked_add(count.checked_mul(stride)?.checked_add(size)?)?;
                if size == stride {
                    runs.push((8 * (offset + size), 8 * end));
                } else {
                    contained.push((8 * (offset + stride), 8 * end));
                }
            },
            | _ => {
                let (start, stop) = leaf.bits(&sema.types).ok()?;
                runs.push((start, stop));
                if let Leaf::Scalar {
                    offset,
                    bit_field: Some(bits),
                    ..
                } = leaf
                {
                    let written = u64::from((bits.bit_offset + bits.width).div_ceil(8));
                    contained.push((8 * offset, 8 * (offset + written)));
                }
                zero &= zero_leaf(sema, leaf);
            },
        }
    }
    runs.retain(|&(start, stop)| start < stop);
    runs.sort_unstable();
    let Some((&(start, mut stop), rest)) = runs.split_first() else {
        return Some(Rest::Unchanged);
    };
    for &(next, end) in rest {
        if next > stop {
            return None;
        }
        stop = stop.max(end);
    }
    let all_inside = contained
        .iter()
        .all(|&(from, to)| from >= start && to <= stop);
    if !all_inside || start % 8 != 0 || stop % 8 != 0 {
        return None;
    }
    let (offset, size, stride) = (start / 8, (stop - start) / 8, repeat.stride);
    if size > stride || (storage == Storage::Automatic && size != stride) {
        return None;
    }
    // The later elements lie between the run's end and the last element's.
    let end = offset.checked_add(repeat.count.checked_mul(stride)?)?;
    if first.copied
        && selected
            .keys()
            .any(|&(at, _)| (offset + size..end).contains(&at))
    {
        return None;
    }
    Some(Rest::Copied(Leaf::Replicate {
        offset,
        size,
        stride,
        count: repeat.count - 1,
        ty: body[0].ty(),
        zero,
    }))
}

/// A range's later elements, when its first element's leaves give them all
/// its bytes.
enum Rest<'tu> {
    /// The first element's leaves write nothing, so neither would theirs.
    Unchanged,
    /// The leaf that copies the first element's bytes into them.
    Copied(Leaf<'tu>),
}

/// Whether a leaf writes only zero bytes: an implicit zero, or a scalar
/// integer constant zero converted to an integer or pointer.
/// C99: §6.6 paragraph 6, p. 95; PDF p. 107.
fn zero_leaf<'tu>(sema: &SemanticTranslationUnit<'tu>, leaf: Leaf<'tu>) -> bool {
    match leaf {
        | Leaf::Zero { .. } => true,
        | Leaf::Replicate { zero, .. } => zero,
        | Leaf::Scalar { expression, ty, .. } =>
            matches!(
                repr(&sema.types, ty),
                Ok(Repr::Int { .. } | Repr::Bool | Repr::Pointer)
            ) && sema
                .expression_info(expression)
                .is_some_and(|info| info.ice && info.integer.is_some_and(|value| value.value == 0)),
        | Leaf::String { .. } | Leaf::Copy { .. } => false,
    }
}

/// Whether `ty` or its array elements are volatile.
/// C99: §6.7.3 paragraphs 6 and 8, p. 109; PDF p. 121.
fn volatile(sema: &SemanticTranslationUnit<'_>, mut ty: TypeId) -> bool {
    loop {
        if types::is_volatile(ty) {
            return true;
        }
        match sema.types.kind(ty) {
            | TypeKind::Array(element, _) => ty = element,
            | _ => return false,
        }
    }
}

/// The fewest elements a range of `storage` copies its first element into.
fn minimum_count(storage: Storage) -> u64 {
    #[cfg(test)]
    if let Some(minimum) = MINIMUM.get() {
        return minimum;
    }
    match storage {
        | Storage::Static => 2,
        | Storage::Automatic => AUTOMATIC_REPLICATION,
    }
}

#[cfg(test)]
thread_local! {
    /// Replay steps on this thread, for the scaling tests.
    #[cfg_attr(
        windows,
        expect(
            clippy::missing_const_for_thread_local,
            reason = "The const initializer still trips the lint in the Windows std expansion."
        )
    )]
    pub(in crate::lowering) static STEPS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    /// Replaces [`minimum_count`] for both storage durations; `u64::MAX`
    /// replays every element.
    #[cfg_attr(
        windows,
        expect(
            clippy::missing_const_for_thread_local,
            reason = "The const initializer still trips the lint in the Windows std expansion."
        )
    )]
    pub(in crate::lowering) static MINIMUM: std::cell::Cell<Option<u64>> = const {
        std::cell::Cell::new(None)
    };
}

/// A range repeat being emitted: its operations, its elements, and the
/// state to restore after it.
struct Repeat {
    start:      usize,
    end:        usize,
    count:      u64,
    stride:     u64,
    position:   u64,
    /// The shift before the repeat.
    shift:      u64,
    /// The open brace pairs before the repeat.
    open:       usize,
    /// The leaves, union selections and whole copies before the repeat.
    leaves:     usize,
    selections: usize,
    copies:     usize,
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
