//! Validate final BSS initializer contents before any backend runs. Walk the
//! retained plan backwards, stopping at the first surviving nonzero leaf.
//! `coverage.rs` records later writes and queries scalar ranges symbolically.
//! Complicated ranges use a bounded sparse replay; if its linear work budget
//! expires, the backend's final-content validation owns the refusal instead.
//! Neither path allocates storage proportional to an object's extent.
//!
//! GNU section extension: ELF NOBITS contains zero-initialized storage only.
//! C99: §6.7.8p11, p. 126; PDF p. 138 (assignment conversion), and p19-21,
//! p. 127; PDF p. 139 (replacement and implicit zero initialization).

mod backwards;
mod coverage;

use backwards::Scan;
use coverage::{
    Coverage,
    Pattern,
};

use super::super::{
    BitRange,
    ConstantClass,
    DefinitionKind,
    InitOperation,
    InitializerPlan,
    Scalar,
    SemanticError,
    SemanticErrorKind,
    SemanticTranslationUnit,
    TagKind,
    TypeId,
    TypeKind,
};
use crate::{
    float_parsing::bits,
    ir::Type,
    target::{
        Target,
        sections::ElfSectionKind,
    },
    translation_phases::{
        Context,
        TranslationError,
        parsing::syntax::{
            Expression,
            ExpressionType,
        },
        preprocessing::{
            LiteralEncoding,
            StringTokenType,
        },
    },
    util::bump::{
        ArenaMap,
        ArenaVec,
        Bump,
    },
};

/// Diagnose every initialized object in a named BSS family, after entity
/// completion has supplied inherited section attributes and initializers.
pub(in crate::translation_phases::semantic_analysis) fn check<'tu>(
    context: &mut Context<'tu>,
    sema: &mut SemanticTranslationUnit<'tu>,
    scratch: &Bump,
) {
    if !matches!(
        context.configuration.target(),
        Target::LinuxGnu | Target::LinuxMusl
    ) {
        return;
    }
    let mut reachable = None;
    for definition in sema.definitions {
        if definition.kind != DefinitionKind::Object {
            continue;
        }
        let binding = definition.binding;
        let attributes = sema.entity_attributes(binding);
        let Some(section) = attributes
            .and_then(|a| a.section)
            .filter(|section| ElfSectionKind::named(section).is_bss())
        else {
            continue;
        };
        let Some(initializer) = sema.entity(binding).and_then(|e| e.initializer) else {
            continue;
        };
        let Some(plan) = sema.initializers.plan(initializer) else {
            continue;
        };
        // Keep source-backed diagnostics for exactly the objects emission
        // retains, including evaluated references from emitted initializers.
        if !reachable
            .get_or_insert_with(|| sema.reachable_bindings(context, scratch, |_| false))
            .contains(&binding)
        {
            continue;
        }
        let size = sema
            .flexible_object_size(binding)
            .or_else(|| sema.types.layout(plan.ty).map(|l| l.size));
        let Some(fixups) = size.and_then(|size| contents(context, sema, plan, size, scratch))
        else {
            continue;
        };
        let name = context.string_cache.intern(section);
        let source_vectors = context.file_location(initializer.source_vectors);
        sema.errors += 1;
        context.append_pending_errors([TranslationError::Semantic(SemanticError {
            extension_severity: None,
            kind: if fixups {
                SemanticErrorKind::BssFixups
            } else {
                SemanticErrorKind::BssInitializer
            },
            source_vectors,
            name: Some(name),
            previous: None,
        })]);
    }
}

/// `Some` means invalid contents (the boolean distinguishes fixups); zero
/// contents and unmodeled operations return `None`. Existing semantic or
/// backend refusals still own unmodeled initializers.
fn contents<'tu>(
    context: &Context<'_>,
    sema: &SemanticTranslationUnit<'tu>,
    plan: &InitializerPlan<'tu>,
    size: u64,
    arena: &Bump,
) -> Option<bool> {
    let mut work = Work::new(plan.operations.len());
    contents_with_work(context, sema, plan, size, arena, &mut work)
}

fn contents_with_work<'tu>(
    context: &Context<'_>,
    sema: &SemanticTranslationUnit<'tu>,
    plan: &InitializerPlan<'tu>,
    size: u64,
    arena: &Bump,
    work: &mut Work,
) -> Option<bool> {
    match backwards::check(context, sema, plan, size, arena, work)? {
        | Scan::Known(result) => result,
        | Scan::Replay => replay(context, sema, plan, size, arena, work),
    }
}

fn replay<'tu>(
    context: &Context<'_>,
    sema: &SemanticTranslationUnit<'tu>,
    plan: &InitializerPlan<'tu>,
    size: u64,
    arena: &Bump,
    budget: &mut Work,
) -> Option<bool> {
    let mut image = Image {
        ranges: ArenaVec::new_in(arena),
        work:   budget,
    };
    let mut selected = ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, arena);
    let mut work = ArenaVec::new_in(arena);
    work.push((0_usize, plan.operations.len(), 0_u64, 1_u64, 0_u64));
    while let Some((mut index, end, shift, count, stride)) = work.pop() {
        if count > 1 {
            work.push((index, end, shift.checked_add(stride)?, count - 1, stride));
        }
        while index < end {
            image.work.charge(1)?;
            let op = plan.operations[index];
            index += 1;
            match op {
                | InitOperation::Repeat {
                    count,
                    stride,
                    length,
                } => {
                    let stop = index + length as usize;
                    work.push((stop, end, shift, 1, 0));
                    if count > 0 {
                        work.push((index, stop, shift, count, stride));
                    }
                    break;
                },
                | InitOperation::Zero { destination, .. }
                | InitOperation::EmptyScalar { destination, .. } => {
                    let start = destination.offset.checked_add(shift)?.checked_mul(8)?;
                    let width = sema.types.layout(destination.ty)?.size.checked_mul(8)?;
                    image.clear(start, start.checked_add(width)?)?;
                },
                | InitOperation::Select {
                    destination,
                    member,
                    ..
                } => {
                    let offset = destination.offset.checked_add(shift)?;
                    if selected
                        .insert((offset, destination.ty.index), member)
                        .is_some_and(|old| old != member)
                    {
                        let start = offset.checked_mul(8)?;
                        image.clear(
                            start,
                            start.checked_add(
                                sema.types.layout(destination.ty)?.size.checked_mul(8)?,
                            )?,
                        )?;
                    }
                },
                | InitOperation::Store {
                    value,
                    destination,
                    bit_field,
                    ..
                } => {
                    scalar(
                        &mut image,
                        sema,
                        plan.values[value.0 as usize].expression,
                        destination.ty,
                        destination.offset.checked_add(shift)?,
                        bit_field,
                    )?;
                },
                | InitOperation::String {
                    literal,
                    destination,
                } => {
                    string(
                        &mut image,
                        context,
                        sema,
                        literal,
                        destination.ty,
                        destination.offset.checked_add(shift)?,
                    )?;
                },
                | InitOperation::Copy { .. } | InitOperation::Unmodeled { .. } => return None,
            }
        }
    }
    // GNU flexible-array replacement can shorten the final object.
    let size = size.checked_mul(8)?;
    image.clear(size, u64::MAX)?;
    (!image.ranges.is_empty()).then(|| image.ranges.iter().any(|r| r.fixup))
}

/// Store a literal's converted code units, truncated to the destination.
/// C99: §6.7.8p14-15, p. 126; PDF p. 138.
fn string<'tu>(
    image: &mut Image<'_, '_>,
    context: &Context<'_>,
    sema: &SemanticTranslationUnit<'tu>,
    mut literal: &Expression<'tu>,
    ty: TypeId,
    offset: u64,
) -> Option<()> {
    let start = offset.checked_mul(8)?;
    let width = sema.types.layout(ty)?.size.checked_mul(8)?;
    image.clear(start, start.checked_add(width)?)?;
    while let ExpressionType::Parenthesized { expression: inner } = literal.kind {
        image.work.charge(1)?;
        literal = inner;
    }
    let (id, encoding, unit_width) = match literal.kind {
        | ExpressionType::StringLiteral(StringTokenType::String(id)) =>
            (id, LiteralEncoding::Utf8, 1),
        | ExpressionType::StringLiteral(StringTokenType::WideString(id)) =>
            (id, LiteralEncoding::Utf32, 4),
        | ExpressionType::StringLiteral(StringTokenType::EncodedString(id, encoding)) => (
            id,
            encoding,
            match encoding {
                | LiteralEncoding::Utf8 => 1,
                | LiteralEncoding::Utf16 => 2,
                | LiteralEncoding::Utf32 => 4,
            },
        ),
        | _ => return None,
    };
    for (index, unit) in context.encoded_literal_units(id, encoding).enumerate() {
        let offset = index as u64 * unit_width * 8;
        if offset >= width {
            break;
        }
        image.bits(
            start + offset,
            (unit_width * 8).min(width - offset),
            u128::from(unit),
        )?;
    }
    Some(())
}

/// Convert a scalar before checking its object bits: numerical zero alone
/// misses negative floating zero, and pre-conversion values can truncate to
/// zero. Reuse lowering's exact target floating encoders.
fn scalar<'tu>(
    image: &mut Image<'_, '_>,
    sema: &SemanticTranslationUnit<'tu>,
    expression: &Expression<'tu>,
    ty: TypeId,
    offset: u64,
    field: Option<BitRange>,
) -> Option<()> {
    let info = sema.expression_info(expression)?;
    let size = sema.types.layout(ty)?.size;
    let ty = sema.types.non_atomic(ty);
    let kind = sema.types.kind(ty);
    let scalar = match kind {
        | TypeKind::Scalar(s) => Some(s),
        | TypeKind::Tag(id) if sema.types.tags[id].kind == TagKind::Enum =>
            Some(sema.types.tags[id].compatible.get()),
        | _ => None,
    };
    let start = offset.checked_mul(8)?;
    let width = size.checked_mul(8)?;
    let address = info.constant == ConstantClass::Address
        || (info.static_address
            && matches!(
                sema.types.kind(info.ty),
                TypeKind::Array(..) | TypeKind::Function { .. }
            ));
    let integer = info.integer.filter(|_| {
        matches!(kind, TypeKind::Pointer(_))
            || !matches!(
                sema.types.kind(info.ty),
                TypeKind::Pointer(_) | TypeKind::Array(..)
            )
    });
    if let Some(s) = scalar {
        if s.integer() {
            let value = if s == Scalar::Bool {
                if let Some(value) = info.integer {
                    i128::from(value.value != 0)
                } else if let Some(value) = info.floating() {
                    i128::from(!value.real.is_zero() || !value.imag.is_zero())
                } else if let Some(value) = info.binary128() {
                    i128::from(!value.real.is_zero() || !value.imag.is_zero())
                } else if address {
                    1
                } else {
                    return None;
                }
            } else if let Some(value) = integer {
                value.value
            } else if let Some(value) = info.floating() {
                let (width, signed) = sema.types.target.integer(s)?;
                bits::to_integer(&value.real, width, signed)?.cast_signed()
            } else if let Some(value) = info.binary128() {
                let (width, signed) = sema.types.target.integer(s)?;
                bits::binary128_to_integer(value.real, width, signed)?.cast_signed()
            } else if address || info.constant == ConstantClass::LabelDifference {
                return fixup(image, start, width);
            } else {
                return None;
            };
            let (start, width) = field.map_or((start, width), |f| {
                (start + u64::from(f.bit_offset), u64::from(f.width))
            });
            image.bits(start, width, value as u128)?;
            return Some(());
        }
        let (float, complex) = match s.representation() {
            | Scalar::Float => (Type::F32, false),
            | Scalar::Double => (Type::F64, false),
            | Scalar::LongDouble => (if size == 8 { Type::F64 } else { Type::F80 }, false),
            | Scalar::ComplexFloat => (Type::F32, true),
            | Scalar::ComplexDouble => (Type::F64, true),
            | Scalar::ComplexLongDouble => (if size == 16 { Type::F64 } else { Type::F80 }, true),
            | Scalar::Float128 => (Type::F128, false),
            | Scalar::ComplexFloat128 => (Type::F128, true),
            | _ => return None,
        };
        let real = if let Some(value) = info.binary128() {
            bits::binary128_bits(value.real, float)?
        } else if let Some(value) = info.floating() {
            bits::constant_bits(&value.real, float)?
        } else {
            let value = integer?;
            if value.signed {
                bits::from_integer(value.value, float)?
            } else {
                bits::from_unsigned(value.value as u128, float)?
            }
        };
        let component = if complex { width / 2 } else { width };
        image.bits(start, component, real)?;
        if complex {
            let imag = if let Some(value) = info.binary128() {
                bits::binary128_bits(value.imag, float)?
            } else {
                info.floating()
                    .map_or(Some(0), |value| bits::constant_bits(&value.imag, float))?
            };
            image.bits(start + component, component, imag)?;
        }
        return Some(());
    }
    if matches!(kind, TypeKind::Pointer(_) | TypeKind::Nullptr) {
        if matches!(sema.types.kind(info.ty), TypeKind::Nullptr) {
            image.bits(start, width, 0)?;
            return Some(());
        }
        if let Some(value) = integer {
            image.bits(start, width, value.value as u128)?;
            return Some(());
        }
        return fixup(image, start, width);
    }
    None
}

fn fixup(image: &mut Image<'_, '_>, start: u64, width: u64) -> Option<()> {
    let end = start.checked_add(width)?;
    image.clear(start, end)?;
    image.ranges.push(Range {
        start,
        end,
        fixup: true,
    });
    Some(())
}

/// Bound all analysis, including fallback overlap scans, by written plan size.
/// Exhaustion conservatively defers to the backend's final-content guard;
/// it never rejects valid zero data or lets an invalid object be emitted.
struct Work {
    remaining: usize,
    #[cfg(test)]
    steps:     usize,
}

impl Work {
    fn new(operations: usize) -> Self {
        Self {
            remaining:          operations.saturating_mul(2048).saturating_add(4096),
            #[cfg(test)]
            steps:              0,
        }
    }

    fn charge(&mut self, cost: usize) -> Option<()> {
        let remaining = self.remaining.checked_sub(cost);
        #[cfg(test)]
        {
            self.steps += cost.min(self.remaining);
        }
        self.remaining = remaining.unwrap_or(0);
        remaining.map(|_| ())
    }
}

#[cfg(test)]
pub(in crate::translation_phases::semantic_analysis) fn measured_contents<'tu>(
    context: &Context<'_>,
    sema: &SemanticTranslationUnit<'tu>,
    plan: &InitializerPlan<'tu>,
    size: u64,
    arena: &Bump,
) -> (Option<bool>, usize) {
    let mut work = Work::new(plan.operations.len());
    let result = contents_with_work(context, sema, plan, size, arena, &mut work);
    (result, work.steps)
}

struct Image<'a, 'w> {
    ranges: ArenaVec<'a, Range>,
    work:   &'w mut Work,
}

impl Image<'_, '_> {
    fn clear(&mut self, start: u64, end: u64) -> Option<()> {
        let length = self.ranges.len();
        self.work.charge(length + 1)?;
        for index in 0..length {
            let range = self.ranges[index];
            if range.start >= end || range.end <= start {
                continue;
            }
            if range.fixup {
                // Any overlapping write discards a relocation as a whole.
                self.ranges[index].end = range.start;
            } else {
                self.ranges[index].end = range.end.min(start).max(range.start);
                if range.end > end {
                    self.ranges.push(Range {
                        start: end.max(range.start),
                        ..range
                    });
                }
            }
        }
        self.ranges.retain(|range| range.start < range.end);
        Some(())
    }

    fn bits(&mut self, start: u64, width: u64, value: u128) -> Option<()> {
        self.clear(start, start.checked_add(width)?)?;
        self.work.charge(width.min(128) as usize)?;
        for bit in 0..width.min(128) {
            if value & (1_u128 << bit) != 0 {
                if let Some(last) = self.ranges.last_mut()
                    && !last.fixup
                    && last.end == start + bit
                {
                    last.end += 1;
                } else {
                    self.ranges.push(Range {
                        start: start + bit,
                        end:   start + bit + 1,
                        fixup: false,
                    });
                }
            }
        }
        Some(())
    }
}

#[derive(Clone, Copy)]
struct Range {
    start: u64,
    end:   u64,
    fixup: bool,
}
