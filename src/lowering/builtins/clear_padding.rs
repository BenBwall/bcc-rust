//! `__builtin_clear_padding`: zeroes the bits of an object that hold no part
//! of its value and leaves the others alone.
//!
//! [`Mask`] walks the type from the target's layout and describes the object
//! as [`Run`]s of bytes: all value, all padding, a mix of the two, or a
//! repeated element. A union's value bits are those of any member, so its
//! members are merged on a flat copy of its bytes. Emission follows the runs
//! in order: a run of padding bytes becomes one fill, a mixed byte a load, a
//! mask and a store, and an array an inline sequence or, when that would be
//! long, a loop over its elements. An array of variable length is a loop over
//! its constant-size elements, whose extents multiply to the byte count
//! semantic analysis keeps a size formula for.
//!
//! What counts as value bits is what GCC 14 and Clang 23 were measured to
//! keep: every bit of a `_Bool`, of a pointer and of an integer, the 10 value
//! bytes of an x87 `long double` in 16 and the `width` low bits of a `_BitInt`.
//! They disagree in two places, and each identity follows its compiler. GCC
//! keeps a bit-field that is a direct member of a union to the whole bytes it
//! reaches and takes a `nullptr_t` in a record for padding, where Clang keeps
//! exactly the bit-field's bits and a `nullptr_t`'s value.
//!
//! The load and the stores are not volatile even for a `volatile` object, as
//! in both compilers. A mixed byte that was never written reads as poison, so
//! it is frozen before the mask, as a bit-field store freezes the bytes it
//! shares.
//!
//! GNU extension: GCC manual, "Other Builtins".
//! <https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html>
//! C99: §6.2.6.1 paragraph 6, p. 38; PDF p. 50 (padding bytes); §6.7.2.1
//! paragraphs 13-15, p. 103; PDF p. 115 (where a record's padding is).

use super::super::{
    FunctionLowerer,
    LoweringError,
    LoweringErrorKind,
    module_items::at,
    types,
};
use crate::{
    configuration::RuleSet,
    ir::{
        Align,
        InstData,
        InstFlags,
        IntCC,
        Opcode,
        Type,
        Value,
    },
    translation_phases::{
        SourceVectors,
        semantic_analysis::{
            ArrayBound,
            Member,
            Scalar,
            Tag,
            TagKind,
            TypeId,
            TypeKind,
            Types,
        },
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// An array whose elements would take more fills and masked bytes than this
/// inline clears them in a loop.
const INLINE_OPERATIONS: u64 = 32;
/// The largest union merged on a flat copy of its bytes. A larger one is
/// supported only if a member covers it with value bits.
const FLAT_UNION_BYTES: u64 = 1 << 20;
/// The most runs one call's type may describe: a limit on code size, for the
/// type whose nested records double in size at each level, which no array
/// compresses.
const MAX_RUNS: u32 = 1 << 20;

impl FunctionLowerer<'_, '_, '_, '_, '_> {
    /// Clears the padding of the object of type `ty` at `pointer`. A `void`
    /// type, which semantic analysis retains for a call that clears nothing,
    /// has no runs. GNU extension: GCC manual, "Other Builtins".
    pub(super) fn clear_padding(
        &mut self,
        pointer: Value,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<(), LoweringError> {
        let sema = self.unit.sema;
        let types = &sema.types;
        if types.kind(ty) == TypeKind::Scalar(Scalar::Void) {
            return Ok(());
        }
        let mut element = ty;
        while types.layout(element).is_none()
            && let TypeKind::Array(inner, _) = types.kind(element)
        {
            element = inner;
        }
        let mut mask = Mask {
            types,
            scratch: self.scratch,
            gcc: self.unit.context.configuration.compiler_identity().rules() == RuleSet::Gcc,
            source,
            runs_left: MAX_RUNS,
        };
        let mut runs = RunList::new_in(self.scratch);
        mask.object(element, &mut runs)?;
        let runs = runs.finish();
        if element == ty {
            let mut cursor = Cursor::new(pointer);
            self.clear_runs(runs, &mut cursor);
            self.flush_zeros(&mut cursor);
        } else if operations(runs) != 0 {
            let stride = mask.size(element)?;
            let total = self.runtime_size(ty, source)?;
            self.clear_elements(runs, pointer, stride, total);
        }
        Ok(())
    }

    /// Emits the runs from the cursor's position, leaving a run of padding
    /// open so that it joins the next element's first.
    fn clear_runs(&mut self, runs: &[Run<'_>], cursor: &mut Cursor) {
        for run in runs {
            match *run {
                | Run::Value(bytes) => {
                    self.flush_zeros(cursor);
                    cursor.at += bytes;
                },
                | Run::Padding(bytes) => {
                    _ = cursor.zeros_from.get_or_insert(cursor.at);
                    cursor.at += bytes;
                },
                | Run::Mixed(value_bits) => {
                    self.flush_zeros(cursor);
                    self.clear_mixed_byte(cursor.base, cursor.at, value_bits);
                    cursor.at += 1;
                },
                | Run::Repeat { body, size, count } => {
                    if operations(body).saturating_mul(count) <= INLINE_OPERATIONS {
                        for _ in 0..count {
                            self.clear_runs(body, cursor);
                        }
                    } else {
                        self.flush_zeros(cursor);
                        let start = self.offset_address(cursor.base, cursor.at);
                        let total = self.iconst(Type::I64, i128::from(size * count));
                        self.clear_elements(body, start, size, total);
                        cursor.at += size * count;
                    }
                },
            }
        }
    }

    /// A loop over the elements in the `total` bytes from `start`, each of
    /// `stride` bytes, which no element's padding joins across.
    fn clear_elements(&mut self, body: &[Run<'_>], start: Value, stride: u64, total: Value) {
        let header = self.draft.create_block(&[Type::I64]);
        let zero = self.iconst(Type::I64, 0);
        self.draft.jump(header, &[zero]);
        self.draft.switch_to(header);
        let index = self.draft.block_params(header)[0];
        let more = self.icmp(IntCC::Ult, index, total);
        let element = self.draft.create_block(&[]);
        let exit = self.draft.create_block(&[]);
        self.draft.brif(more, element, exit);
        self.draft.seal(element);
        self.draft.switch_to(element);
        let mut cursor = Cursor::new(self.ptr_add(start, index));
        self.clear_runs(body, &mut cursor);
        self.flush_zeros(&mut cursor);
        let stride = self.iconst(Type::I64, i128::from(stride));
        let next = self.binary_inst(Opcode::Iadd, InstFlags::empty(), index, stride);
        self.draft.jump(header, &[next]);
        self.draft.seal(header);
        self.draft.seal(exit);
        self.draft.switch_to(exit);
    }

    /// Zeroes the open run of padding bytes, if any.
    fn flush_zeros(&mut self, cursor: &mut Cursor) {
        let Some(from) = cursor.zeros_from.take() else {
            return;
        };
        let address = self.offset_address(cursor.base, from);
        let zero = self.iconst(Type::I8, 0);
        let size = self.iconst(Type::I64, i128::from(cursor.at - from));
        _ = self.draft.push(
            self.memory_op(InstData::MemoryRange {
                opcode: Opcode::Fill,
                flags:  InstFlags::empty(),
                align:  Align::BYTE,
                args:   [address, zero, size],
            }),
            None,
        );
    }

    /// Keeps only the `value_bits` of the byte at `offset`.
    fn clear_mixed_byte(&mut self, base: Value, offset: u64, value_bits: u8) {
        let address = self.offset_address(base, offset);
        let old = self.inst(
            InstData::Load {
                ty:    Type::I8,
                flags: InstFlags::empty(),
                align: Align::BYTE,
                tag:   None,
                addr:  address,
            },
            Type::I8,
        );
        let old = self.convert(Opcode::Freeze, Type::I8, old);
        let keep = self.iconst(Type::I8, i128::from(value_bits));
        let value = self.binary_inst(Opcode::And, InstFlags::empty(), old, keep);
        _ = self.draft.push(
            self.memory_op(InstData::Store {
                ty:    Type::I8,
                flags: InstFlags::empty(),
                align: Align::BYTE,
                tag:   None,
                args:  [value, address],
            }),
            None,
        );
    }
}

/// Where emission has got to in an object.
struct Cursor {
    base:       Value,
    at:         u64,
    /// The start of the open run of padding bytes, which ends at `at`.
    zeros_from: Option<u64>,
}

impl Cursor {
    const fn new(base: Value) -> Self {
        Self {
            base,
            at: 0,
            zeros_from: None,
        }
    }
}

/// A part of an object, in ascending address order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Run<'s> {
    /// Bytes whose every bit holds value.
    Value(u64),
    /// Bytes with no bit of value.
    Padding(u64),
    /// One byte with these value bits and some padding bits.
    Mixed(u8),
    /// `count` elements of `size` bytes, each described by `body`.
    Repeat {
        body:  &'s [Run<'s>],
        size:  u64,
        count: u64,
    },
}

/// How many fills and masked bytes the runs take without a loop.
fn operations(runs: &[Run<'_>]) -> u64 {
    runs.iter().fold(0_u64, |sum, run| {
        sum.saturating_add(match *run {
            | Run::Value(_) => 0,
            | Run::Padding(_) | Run::Mixed(_) => 1,
            | Run::Repeat { body, count, .. } => operations(body).saturating_mul(count),
        })
    })
}

/// A list of runs under construction, with neighbors of a kind merged.
struct RunList<'s> {
    runs:  ArenaVec<'s, Run<'s>>,
    /// The bytes the runs cover so far.
    bytes: u64,
}

impl<'s> RunList<'s> {
    fn new_in(scratch: &'s Bump) -> Self {
        Self {
            runs:  ArenaVec::new_in(scratch),
            bytes: 0,
        }
    }

    fn finish(self) -> &'s [Run<'s>] {
        self.runs.leak()
    }

    /// The only run, if the list is one.
    fn single(&self) -> Option<Run<'s>> {
        match *self.runs.as_slice() {
            | [run] => Some(run),
            | _ => None,
        }
    }
}

/// The value bits of the objects of one call, as [`Run`]s.
struct Mask<'t, 'tu, 's> {
    types:     &'t Types<'tu>,
    scratch:   &'s Bump,
    /// GCC's rules for what the module map says they differ on.
    gcc:       bool,
    source:    SourceVectors,
    runs_left: u32,
}

impl<'s> Mask<'_, '_, 's> {
    /// Appends the runs of an object of type `ty`, covering exactly its size.
    /// The native depth is that of the type's nesting, which the parser's
    /// nesting limit bounds on the compilation stack.
    fn object(&mut self, ty: TypeId, out: &mut RunList<'s>) -> Result<(), LoweringError> {
        let size = self.size_or_empty(ty)?;
        match self.types.kind(ty) {
            | TypeKind::Scalar(scalar) => match scalar.representation() {
                | Scalar::LongDouble if self.types.target.long_double_is_x87() => {
                    self.value(out, X87_VALUE_BYTES)?;
                    self.padding(out, size - X87_VALUE_BYTES)?;
                },
                | Scalar::ComplexLongDouble if self.types.target.long_double_is_x87() => {
                    for _ in 0..2 {
                        self.value(out, X87_VALUE_BYTES)?;
                        self.padding(out, size / 2 - X87_VALUE_BYTES)?;
                    }
                },
                | _ => self.value(out, size)?,
            },
            | TypeKind::Pointer(_) => self.value(out, size)?,
            | TypeKind::Nullptr =>
                if self.gcc {
                    self.padding(out, size)?;
                } else {
                    self.value(out, size)?;
                },
            | TypeKind::BitInt { width, .. } => {
                let width = u64::from(width);
                self.value(out, width / 8)?;
                let top = width % 8;
                if top != 0 {
                    self.mixed(out, (1_u8 << top) - 1)?;
                }
                self.padding(out, size - width.div_ceil(8))?;
            },
            | TypeKind::Atomic(value) => {
                self.object(value, out)?;
                self.padding(out, size - self.size(value)?)?;
            },
            | TypeKind::Vector { element, count, .. } => {
                let lane = self.size(element)?;
                let mut body = RunList::new_in(self.scratch);
                self.object(element, &mut body)?;
                self.repeat(out, &body, lane, count)?;
                self.padding(out, size - lane * count)?;
            },
            | TypeKind::Array(element, ArrayBound::Constant(count)) => {
                let stride = self.size(element)?;
                let mut body = RunList::new_in(self.scratch);
                self.object(element, &mut body)?;
                self.repeat(out, &body, stride, count)?;
            },
            // A flexible array member adds nothing to its record's size.
            | TypeKind::Array(_, ArrayBound::Incomplete) => {},
            | TypeKind::Tag(id) => {
                let tag = self.types.tags[id];
                match tag.kind {
                    | TagKind::Enum => self.value(out, size)?,
                    | TagKind::Struct => self.structure(tag, size, out)?,
                    | TagKind::Union => self.union_type(tag, size, out)?,
                }
            },
            | TypeKind::Unknown | TypeKind::Unmodeled(_) =>
                return Err(at(Self::unanalyzed(self.types, ty), self.source)),
            | TypeKind::Array(..) | TypeKind::Function { .. } => return Err(self.refused()),
        }
        Ok(())
    }

    /// A structure's members in order, with the gaps between them padding.
    /// C99: §6.7.2.1 paragraphs 13 and 15, p. 103; PDF p. 115.
    fn structure(
        &mut self,
        tag: &Tag<'_>,
        size: u64,
        out: &mut RunList<'s>,
    ) -> Result<(), LoweringError> {
        let start = out.bytes;
        // The bytes of a run of bit-fields, which may share them: the value
        // bits of `fields`, from the object's byte `fields_at`.
        let mut fields = ArenaVec::new_in(self.scratch);
        let mut fields_at = 0;
        for member in tag.members.get() {
            let member: &Member = member;
            if let Some(width) = member.width {
                if member.name.is_none() || width == 0 {
                    continue;
                }
                if fields.is_empty() || member.offset > fields_at + fields.len() as u64 {
                    self.flush_fields(out, &mut fields)?;
                    self.padding_to(out, start + member.offset)?;
                    fields_at = member.offset;
                }
                let bit = (member.offset - fields_at) * 8 + u64::from(member.bit_offset);
                let end = index((bit + u64::from(width)).div_ceil(8));
                if fields.len() < end {
                    fields.resize(end, 0);
                }
                set_bits(&mut fields, bit, u64::from(width));
            } else {
                self.flush_fields(out, &mut fields)?;
                self.padding_to(out, start + member.offset)?;
                self.object(member.ty, out)?;
            }
        }
        self.flush_fields(out, &mut fields)?;
        self.padding_to(out, start + size)
    }

    /// A union's value bits are those of any of its members, so the padding
    /// is what none of them covers.
    /// C99: §6.7.2.1 paragraph 14, p. 103; PDF p. 115.
    fn union_type(
        &mut self,
        tag: &Tag<'_>,
        size: u64,
        out: &mut RunList<'s>,
    ) -> Result<(), LoweringError> {
        let mut members = ArenaVec::new_in(self.scratch);
        for member in tag.members.get() {
            if member.width.is_some() {
                continue;
            }
            let mut runs = RunList::new_in(self.scratch);
            self.object(member.ty, &mut runs)?;
            if member.offset == 0 && runs.single() == Some(Run::Value(size)) {
                return self.value(out, size);
            }
            members.push((member.offset, runs.finish()));
        }
        if size > FLAT_UNION_BYTES {
            return Err(self.refused());
        }
        let mut bytes = ArenaVec::new_in(self.scratch);
        bytes.resize(index(size), 0_u8);
        for member in tag.members.get() {
            if let Some(width) = member.width.filter(|_| member.name.is_some()) {
                // GCC keeps the whole bytes a bit-field reaches here.
                let width = u64::from(width);
                let width = if self.gcc {
                    width.next_multiple_of(8)
                } else {
                    width
                };
                let bit = member.offset * 8 + u64::from(member.bit_offset);
                set_bits(&mut bytes, bit, width);
            }
        }
        for (offset, runs) in members {
            apply(&mut bytes, offset, runs);
        }
        self.flush_fields(out, &mut bytes)
    }

    /// Appends `count` elements of `size` bytes described by `body`.
    fn repeat(
        &mut self,
        out: &mut RunList<'s>,
        body: &RunList<'s>,
        size: u64,
        count: u64,
    ) -> Result<(), LoweringError> {
        if size == 0 || count == 0 {
            return Ok(());
        }
        match body.single() {
            | Some(Run::Value(_)) => self.value(out, size * count),
            | Some(Run::Padding(_)) => self.padding(out, size * count),
            // An array of arrays is one loop over the innermost elements.
            | Some(Run::Repeat {
                body,
                size,
                count: inner,
            }) => self.push(
                out,
                Run::Repeat {
                    body,
                    size,
                    count: inner * count,
                },
            ),
            | _ if count == 1 => {
                for &run in &body.runs {
                    self.push(out, run)?;
                }
                Ok(())
            },
            | _ => {
                let mut copy = ArenaVec::with_capacity_in(body.runs.len(), self.scratch);
                copy.extend_from_slice(&body.runs);
                self.push(
                    out,
                    Run::Repeat {
                        body: copy.leak(),
                        size,
                        count,
                    },
                )
            },
        }
    }

    /// Appends the bytes of `fields`, by their value bits, and empties it.
    fn flush_fields(
        &mut self,
        out: &mut RunList<'s>,
        fields: &mut ArenaVec<'s, u8>,
    ) -> Result<(), LoweringError> {
        for &bits in fields.iter() {
            match bits {
                | 0 => self.padding(out, 1)?,
                | u8::MAX => self.value(out, 1)?,
                | _ => self.mixed(out, bits)?,
            }
        }
        fields.clear();
        Ok(())
    }

    fn padding_to(&mut self, out: &mut RunList<'s>, end: u64) -> Result<(), LoweringError> {
        self.padding(out, end.saturating_sub(out.bytes))
    }

    fn value(&mut self, out: &mut RunList<'s>, bytes: u64) -> Result<(), LoweringError> {
        self.push(out, Run::Value(bytes))
    }

    fn padding(&mut self, out: &mut RunList<'s>, bytes: u64) -> Result<(), LoweringError> {
        self.push(out, Run::Padding(bytes))
    }

    fn mixed(&mut self, out: &mut RunList<'s>, value_bits: u8) -> Result<(), LoweringError> {
        self.push(out, Run::Mixed(value_bits))
    }

    /// Appends a run, merged with the last if they are of a kind.
    fn push(&mut self, out: &mut RunList<'s>, run: Run<'s>) -> Result<(), LoweringError> {
        out.bytes += match run {
            | Run::Value(bytes) | Run::Padding(bytes) => bytes,
            | Run::Mixed(_) => 1,
            | Run::Repeat { size, count, .. } => size * count,
        };
        match (out.runs.last_mut(), run) {
            | (_, Run::Value(0) | Run::Padding(0)) => return Ok(()),
            | (Some(Run::Value(last)), Run::Value(bytes))
            | (Some(Run::Padding(last)), Run::Padding(bytes)) => {
                *last += bytes;
                return Ok(());
            },
            | _ => {},
        }
        if self.runs_left == 0 {
            return Err(self.refused());
        }
        self.runs_left -= 1;
        out.runs.push(run);
        Ok(())
    }

    /// The size of a complete object type, as `sizeof` has it.
    fn size(&self, ty: TypeId) -> Result<u64, LoweringError> {
        types::layout(self.types, ty)
            .map(|layout| layout.size)
            .map_err(|kind| at(kind, self.source))
    }

    /// As [`Self::size`], and 0 for a flexible array member.
    fn size_or_empty(&self, ty: TypeId) -> Result<u64, LoweringError> {
        match self.types.kind(ty) {
            | TypeKind::Array(_, ArrayBound::Incomplete) => Ok(0),
            | TypeKind::Unknown | TypeKind::Unmodeled(_) =>
                Err(at(Self::unanalyzed(self.types, ty), self.source)),
            | _ => self.size(ty),
        }
    }

    fn unanalyzed(types: &Types<'_>, ty: TypeId) -> LoweringErrorKind {
        LoweringError::unanalyzed_kind(types, ty)
    }

    /// A type too large to describe, refused by name.
    fn refused(&self) -> LoweringError {
        at(
            LoweringErrorKind::UnsupportedBuiltin(Some("__builtin_clear_padding")),
            self.source,
        )
    }
}

/// The bytes of an x87 extended value: a 64-bit significand, then the sign
/// and the exponent.
const X87_VALUE_BYTES: u64 = 10;

/// A position in a buffer of at most [`FLAT_UNION_BYTES`] or a bit-field's few
/// bytes, which every host can address.
fn index(position: u64) -> usize {
    usize::try_from(position).expect("a position in a small buffer")
}

/// Sets `width` bits of `bytes` from bit `start`, counted from the low bit
/// of the first byte.
fn set_bits(bytes: &mut [u8], start: u64, width: u64) {
    let end = start + width;
    let mut bit = start;
    while bit < end {
        let low = bit % 8;
        let count = (8 - low).min(end - bit);
        bytes[index(bit / 8)] |= (u8::MAX >> (8 - count)) << low;
        bit += count;
    }
}

/// Merges the value bits of `runs` into `bytes` from byte `at`.
fn apply(bytes: &mut [u8], at: u64, runs: &[Run<'_>]) {
    let mut at = at;
    for run in runs {
        match *run {
            | Run::Value(count) => {
                for byte in &mut bytes[index(at)..index(at + count)] {
                    *byte = u8::MAX;
                }
                at += count;
            },
            | Run::Padding(count) => at += count,
            | Run::Mixed(value_bits) => {
                bytes[index(at)] |= value_bits;
                at += 1;
            },
            | Run::Repeat { body, size, count } =>
                for _ in 0..count {
                    apply(bytes, at, body);
                    at += size;
                },
        }
    }
}
