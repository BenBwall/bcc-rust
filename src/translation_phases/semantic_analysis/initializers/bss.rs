//! Validate final BSS initializer contents before any backend runs. A sparse
//! image retains only nonzero bits and fixups, so a large zero-filled object
//! does not require storage proportional to its size. Plans are replayed in
//! source order, including replacements, union selections and GNU ranges.
//!
//! GNU section extension: ELF NOBITS contains zero-initialized storage only.
//! C99: §6.7.8p11, p. 126; PDF p. 138 (assignment conversion), and p19-21,
//! p. 127; PDF p. 139 (replacement and implicit zero initialization).

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
    for definition in sema.definitions {
        if definition.kind != DefinitionKind::Object {
            continue;
        }
        let binding = definition.binding;
        let Some(section) = sema
            .entity_attributes(binding)
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
    context: &Context<'tu>,
    sema: &SemanticTranslationUnit<'tu>,
    plan: &InitializerPlan<'tu>,
    size: u64,
    arena: &Bump,
) -> Option<bool> {
    let mut image = Image(ArenaVec::new_in(arena));
    let mut selected = ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, arena);
    let mut work = ArenaVec::new_in(arena);
    work.push((0_usize, plan.operations.len(), 0_u64, 1_u64, 0_u64));
    while let Some((mut index, end, shift, count, stride)) = work.pop() {
        if count > 1 {
            work.push((index, end, shift.checked_add(stride)?, count - 1, stride));
        }
        while index < end {
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
                    image.clear(start, start.checked_add(width)?);
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
                        );
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
                    let start = destination.offset.checked_add(shift)?.checked_mul(8)?;
                    let width = sema.types.layout(destination.ty)?.size.checked_mul(8)?;
                    image.clear(start, start.checked_add(width)?);
                    let mut literal = literal;
                    while let ExpressionType::Parenthesized { expression: inner } = literal.kind {
                        literal = inner;
                    }
                    let (id, encoding, unit_width) = match literal.kind {
                        | ExpressionType::StringLiteral(StringTokenType::String(id)) =>
                            (id, LiteralEncoding::Utf8, 1),
                        | ExpressionType::StringLiteral(StringTokenType::WideString(id)) =>
                            (id, LiteralEncoding::Utf32, 4),
                        | ExpressionType::StringLiteral(StringTokenType::EncodedString(
                            id,
                            encoding,
                        )) => (
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
                        );
                    }
                },
                | InitOperation::Copy { .. } | InitOperation::Unmodeled { .. } => return None,
            }
        }
    }
    // GNU flexible-array replacement can shorten the final object.
    let size = size.checked_mul(8)?;
    image.clear(size, u64::MAX);
    (!image.0.is_empty()).then(|| image.0.iter().any(|r| r.fixup))
}

/// Convert a scalar before checking its object bits: numerical zero alone
/// misses negative floating zero, and pre-conversion values can truncate to
/// zero. Reuse lowering's exact target floating encoders.
fn scalar<'tu>(
    image: &mut Image<'_>,
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
                } else if let Some(value) = info.floating {
                    i128::from(!value.real.is_zero() || !value.imag.is_zero())
                } else if address {
                    1
                } else {
                    return None;
                }
            } else if let Some(value) = integer {
                value.value
            } else if let Some(value) = info.floating {
                bits::to_integer(&value.real)?
            } else if address || info.constant == ConstantClass::LabelDifference {
                return fixup(image, start, width);
            } else {
                return None;
            };
            let (start, width) = field.map_or((start, width), |f| {
                (start + u64::from(f.bit_offset), u64::from(f.width))
            });
            image.bits(start, width, value as u128);
            return Some(());
        }
        let (float, complex) = match s {
            | Scalar::Float => (Type::F32, false),
            | Scalar::Double => (Type::F64, false),
            | Scalar::LongDouble => (if size == 8 { Type::F64 } else { Type::F80 }, false),
            | Scalar::ComplexFloat => (Type::F32, true),
            | Scalar::ComplexDouble => (Type::F64, true),
            | Scalar::ComplexLongDouble => (if size == 16 { Type::F64 } else { Type::F80 }, true),
            | _ => return None,
        };
        let real = if let Some(value) = info.floating {
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
        image.bits(start, component, real);
        if complex {
            let imag = info
                .floating
                .map_or(Some(0), |value| bits::constant_bits(&value.imag, float))?;
            image.bits(start + component, component, imag);
        }
        return Some(());
    }
    if matches!(kind, TypeKind::Pointer(_) | TypeKind::Nullptr) {
        if matches!(sema.types.kind(info.ty), TypeKind::Nullptr) {
            image.bits(start, width, 0);
            return Some(());
        }
        if let Some(value) = integer {
            image.bits(start, width, value.value as u128);
            return Some(());
        }
        return fixup(image, start, width);
    }
    None
}

fn fixup(image: &mut Image<'_>, start: u64, width: u64) -> Option<()> {
    let end = start.checked_add(width)?;
    image.clear(start, end);
    image.0.push(Range {
        start,
        end,
        fixup: true,
    });
    Some(())
}

struct Image<'a>(ArenaVec<'a, Range>);

impl Image<'_> {
    fn clear(&mut self, start: u64, end: u64) {
        let length = self.0.len();
        for index in 0..length {
            let range = self.0[index];
            if range.start >= end || range.end <= start {
                continue;
            }
            if range.fixup {
                // Any overlapping write discards a relocation as a whole.
                self.0[index].end = range.start;
            } else {
                self.0[index].end = range.end.min(start).max(range.start);
                if range.end > end {
                    self.0.push(Range {
                        start: end.max(range.start),
                        ..range
                    });
                }
            }
        }
        self.0.retain(|range| range.start < range.end);
    }

    fn bits(&mut self, start: u64, width: u64, value: u128) {
        self.clear(start, start + width);
        for bit in 0..width.min(128) {
            if value & (1_u128 << bit) != 0 {
                self.0.push(Range {
                    start: start + bit,
                    end:   start + bit + 1,
                    fixup: false,
                });
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Range {
    start: u64,
    end:   u64,
    fixup: bool,
}
