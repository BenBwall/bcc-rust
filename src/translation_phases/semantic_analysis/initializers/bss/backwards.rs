//! Inspect surviving leaves before any sparse replay. Scalar GNU ranges are
//! periodic writes; replacements are intervals in a fixed-depth coverage tree.
//! Stop at the first nonzero leaf not replaced by a later operation.
//!
//! C99: §6.7.8p19, p. 127; PDF p. 139 (replacement in list order).

use super::{
    Coverage,
    Image,
    InitOperation,
    InitializerPlan,
    Pattern,
    SemanticTranslationUnit,
    Work,
    scalar,
    string,
};
use crate::{
    translation_phases::Context,
    util::bump::{
        ArenaMap,
        ArenaVec,
        Bump,
    },
};

pub(super) fn check<'tu>(
    context: &Context<'_>,
    sema: &SemanticTranslationUnit<'tu>,
    plan: &InitializerPlan<'tu>,
    size: u64,
    arena: &Bump,
    work: &mut Work,
) -> Option<Scan> {
    let mut image = Image {
        ranges: ArenaVec::new_in(arena),
        work,
    };
    let mut selected = ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, arena);
    let mut operations = ArenaVec::new_in(arena);
    let mut index = 0;
    // Resolve union switches once in forward order. Group a scalar range
    // with its body instead of visiting one operation per array element.
    while index < plan.operations.len() {
        image.work.charge(1)?;
        let mut reset = false;
        let mut count = 1;
        let mut stride = 0;
        let mut operation = plan.operations[index];
        index += 1;
        if let InitOperation::Repeat {
            count: n,
            stride: step,
            length,
        } = operation
        {
            let end = index.checked_add(length as usize)?;
            if n == 0 {
                index = end;
                continue;
            }
            if length != 1 {
                return Some(Scan::Replay);
            }
            operation = *plan.operations.get(index)?;
            let destination = match operation {
                | InitOperation::Store {
                    destination,
                    bit_field: None,
                    ..
                }
                | InitOperation::EmptyScalar { destination, .. } => destination,
                | _ => return Some(Scan::Replay),
            };
            if n > 1 && sema.types.layout(destination.ty)?.size != step {
                return Some(Scan::Replay);
            }
            count = n;
            stride = step.checked_mul(8)?;
            index = end;
        }
        if let InitOperation::Select {
            destination,
            member,
            ..
        } = operation
        {
            reset = selected
                .insert((destination.offset, destination.ty.index), member)
                .is_some_and(|old| old != member);
        }
        operations.push((operation, reset, count, stride));
    }
    let mut coverage = Coverage::new(arena);
    // Flexible-array replacements can shorten the final object. Writes past
    // its end do not contribute bytes, and partial writes drop whole fixups.
    coverage.cover(size.checked_mul(8)?, u64::MAX, image.work)?;
    for (operation, reset, count, stride) in operations.into_iter().rev() {
        image.work.charge(1)?;
        image.ranges.clear();
        let (destination, field) = match operation {
            | InitOperation::Store {
                destination,
                value,
                bit_field,
                ..
            } => {
                if scalar(
                    &mut image,
                    sema,
                    plan.values[value.0 as usize].expression,
                    destination.ty,
                    destination.offset,
                    bit_field,
                )
                .is_none()
                {
                    return Some(Scan::Replay);
                }
                (destination, bit_field)
            },
            | InitOperation::String {
                destination,
                literal,
            } => {
                if string(
                    &mut image,
                    context,
                    sema,
                    literal,
                    destination.ty,
                    destination.offset,
                )
                .is_none()
                {
                    return Some(Scan::Replay);
                }
                (destination, None)
            },
            | InitOperation::Zero { destination, .. }
            | InitOperation::EmptyScalar { destination, .. } => (destination, None),
            | InitOperation::Select { destination, .. } if reset => (destination, None),
            | InitOperation::Select { .. } => continue,
            | InitOperation::Repeat { .. }
            | InitOperation::Copy { .. }
            | InitOperation::Unmodeled { .. } => return Some(Scan::Replay),
        };
        let start = destination.offset.checked_mul(8)?;
        let width = sema.types.layout(destination.ty)?.size.checked_mul(8)?;
        let (start, width) = if let Some(field) = field {
            (
                start.checked_add(u64::from(field.bit_offset))?,
                u64::from(field.width),
            )
        } else {
            (start, width)
        };
        let end = start
            .checked_add(width)?
            .checked_add((count - 1).checked_mul(stride)?)?;
        for range in &image.ranges {
            let pattern = Pattern {
                start: range.start,
                end: range.end,
                count,
                stride,
            };
            if range.fixup {
                // A later write touching any byte removes the entire fixup.
                // For partially covered relocation ranges, defer to replay;
                // bits in ordinary integer ranges need no such expansion.
                if count > 1 {
                    let envelope = Pattern {
                        start,
                        end,
                        count: 1,
                        stride: 0,
                    };
                    if !coverage.any(envelope, false, image.work)? {
                        continue;
                    }
                    if coverage.any(envelope, true, image.work)? {
                        return Some(Scan::Replay);
                    }
                    return Some(Scan::Known(Some(true)));
                }
                if !coverage.any(pattern, true, image.work)? {
                    return Some(Scan::Known(Some(true)));
                }
            } else if coverage.any(pattern, false, image.work)? {
                return Some(Scan::Known(Some(false)));
            }
        }
        coverage.cover(start, end, image.work)?;
    }
    Some(Scan::Known(None))
}

pub(super) enum Scan {
    Known(Option<bool>),
    Replay,
}
