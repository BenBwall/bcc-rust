//! GNU vector values. A vector has no IR type: like an array it lives in
//! memory, its lanes `stride` bytes apart, and its value is the address of
//! an object holding it ([`Repr::Aggregate`](super::types::Repr)). Objects,
//! members, arrays, assignment, calls, results and `?:` therefore treat it
//! as they treat a structure, and every operator works lane by lane: it
//! loads each lane of its operands, computes the lane as the scalar
//! operator would, without C's promotions, and stores it into a fresh
//! object, the result. A back end with vector registers gets the lanes as
//! scalars; LLVM's SLP vectorizer may put them back together at `-O2`.
//!
//! For `v4si a, b; a + b`, lowering reads the address of `a` and of `b`,
//! makes a 16-byte stack slot, and for each lane `i` emits `load.i32` of
//! both operands at offset `4 * i`, an `iadd.i32` and a `store.i32` into
//! the slot. A scalar operand was converted to the vector type first by
//! semantic analysis (`v + 1`), which here splats it into every lane.
//!
//! Signed lane arithmetic wraps rather than carrying `nsw`: GCC and Clang
//! leave vector overflow undefined, so wrapping is one of its meanings, and
//! it is the one both compilers' unoptimized code shows.
//!
//! The calling conventions pass a vector as the claimed compiler does
//! ([`abi_layout`]): System V in one SSE register for 8 and 16 bytes and in
//! a general register for 1, 2 and 4, and Microsoft x64 by reference with
//! results in XMM0. The few shapes where GCC and Clang disagree follow the
//! identity's rules.
//!
//! GNU extension: GCC manual, "Vector Extensions"
//! (<https://gcc.gnu.org/onlinedocs/gcc/Vector-Extensions.html>); Clang
//! Language Extensions, "Vectors and Extended Vectors"
//! (<https://clang.llvm.org/docs/LanguageExtensions.html#vectors-and-extended-vectors>).
//! The lane operators are those of C99 §6.5.3.3 and §6.5.5-§6.5.10,
//! pp. 78-88; PDF pp. 90-100, applied to each lane without §6.3.1.1's
//! integer promotions.

use super::{
    FunctionLowerer,
    LoweringError,
    LoweringErrorKind,
    alignment::at_offset,
    expressions::{
        comparison,
        float_comparison,
    },
    floats,
    types::{
        self,
        Repr,
        int_type,
    },
    work::{
        Item,
        Operand,
        Place,
        Task,
    },
};
use crate::{
    abi::Convention,
    builtins::{
        BuiltinId,
        BuiltinOperation,
        UnsupportedReason,
    },
    configuration::RuleSet,
    ir::{
        AggregateFlags,
        AggregateLayout,
        AggregateScalar,
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
        parsing::{
            Builtin,
            SyntaxOperand,
            syntax::{
                BinaryOperator,
                Expression,
                ExpressionType,
                UnaryOperator,
            },
        },
        preprocessing::KeywordTokenType,
        semantic_analysis::{
            Callee,
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

/// The facts lowering needs about a vector type: its lanes and its object.
#[derive(Clone, Copy, Debug)]
pub(super) struct Shape {
    /// The lane type, unqualified.
    pub(super) element: TypeId,
    pub(super) count:   u64,
    /// The bytes from one lane to the next: the element's size.
    pub(super) stride:  u64,
    /// The object's size, the lanes' rounded up to a power of two.
    pub(super) size:    u64,
    pub(super) align:   Align,
    /// The lane representation: an integer or a real floating type.
    pub(super) lane:    Repr,
}

/// The shape of `ty` if it is a vector type, after qualifiers and
/// atomicity.
pub(super) fn shape(types: &Types<'_>, ty: TypeId) -> Option<Shape> {
    let TypeKind::Vector { element, count, .. } = types.kind(types.non_atomic(ty)) else {
        return None;
    };
    let layout = types.layout(ty)?;
    Some(Shape {
        element: element.unqualified(),
        count,
        stride: types.layout(element)?.size,
        size: layout.size,
        align: Align::from_bytes(types.storage_alignment(ty)?)?,
        lane: types::repr(types, element).ok()?,
    })
}

impl Shape {
    /// The IR type of the lanes.
    fn lane_type(self, source: SourceVectors) -> Result<Type, LoweringError> {
        self.lane
            .value_type()
            .ok_or_else(|| LoweringError::missing("a vector lane has no value type", source))
    }
}

/// The address of the object holding a vector operand.
pub(super) fn vector_address(item: Item, source: SourceVectors) -> Result<Value, LoweringError> {
    match item.operand {
        | Operand::Aggregate(address) | Operand::Place(Place::Memory(address)) => Ok(address),
        | _ => Err(LoweringError::missing(
            "a vector operand has no object",
            source,
        )),
    }
}

impl FunctionLowerer<'_, '_, '_, '_, '_> {
    /// The shape of `ty` if it is a vector type.
    pub(super) fn vector(&self, ty: TypeId) -> Option<Shape> {
        shape(&self.unit.sema.types, ty)
    }

    /// A binary operator on two vectors of the operation's type, which
    /// semantic analysis converted them to: lane `i` of the result is the
    /// scalar operator on lane `i` of each, and a comparison yields all
    /// ones for true and zero for false in a lane of its signed mask type
    /// `ty`.
    /// GNU extension: GCC manual, "Vector Extensions": "the operation
    /// behaves like the corresponding scalar one applied to each element";
    /// "vector comparison is supported ... the result is a vector of the
    /// same width and number of elements ... with -1 for true".
    /// C99: §6.5.5-§6.5.10, pp. 82-88; PDF pp. 94-100.
    pub(super) fn vector_binary(
        &mut self,
        operator: BinaryOperator,
        left: Item,
        right: Item,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        let operand = self.vector_shape(left.ty, source)?;
        // A shift count vector keeps its own lane type.
        let counts = self.vector_shape(right.ty, source)?;
        let result = self.vector_shape(ty, source)?;
        let a = vector_address(left, source)?;
        let b = vector_address(right, source)?;
        let object = self.vector_object(result, source)?;
        let signed = operand.lane.signed();
        for lane in 0..operand.count {
            let x = self.load_lane(a, operand, lane);
            let y = self.load_lane(b, counts, lane);
            let y = if counts.stride == operand.stride {
                y
            } else {
                let ty = self.draft.value_type(x);
                self.extend(y, counts.lane.signed(), ty)
            };
            let value = if let Some(cc) = comparison(operator) {
                let truth = if matches!(operand.lane, Repr::Float { .. }) {
                    self.fcmp(float_comparison(cc), x, y)
                } else {
                    self.icmp(if signed { cc } else { unsigned(cc) }, x, y)
                };
                let mask = result.lane_type(source)?;
                self.extend(truth, true, mask)
            } else {
                self.lane_binary(operator, operand, x, y, source)?
            };
            self.store_lane(value, object, result, lane);
        }
        Ok(Item {
            operand: Operand::Aggregate(object),
            ty,
        })
    }

    /// One lane of an arithmetic, bitwise or shift operator. Integer lanes
    /// wrap (module documentation); a shift count is the other operand's
    /// lane, of the same type.
    fn lane_binary(
        &mut self,
        operator: BinaryOperator,
        shape: Shape,
        x: Value,
        y: Value,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        use BinaryOperator as B;
        let signed = shape.lane.signed();
        let opcode = match (operator, shape.lane) {
            | (B::Addition, Repr::Float { .. }) => Opcode::Fadd,
            | (B::Subtraction, Repr::Float { .. }) => Opcode::Fsub,
            | (B::Multiplication, Repr::Float { .. }) => Opcode::Fmul,
            | (B::Division, Repr::Float { .. }) => Opcode::Fdiv,
            | (B::Addition, _) => Opcode::Iadd,
            | (B::Subtraction, _) => Opcode::Isub,
            | (B::Multiplication, _) => Opcode::Imul,
            | (B::Division, _) if signed => Opcode::Sdiv,
            | (B::Division, _) => Opcode::Udiv,
            | (B::Modulo, _) if signed => Opcode::Srem,
            | (B::Modulo, _) => Opcode::Urem,
            | (B::BitwiseAnd, _) => Opcode::And,
            | (B::BitwiseOr, _) => Opcode::Or,
            | (B::BitwiseXor, _) => Opcode::Xor,
            | (B::LeftShift, _) => Opcode::Shl,
            | (B::RightShift, _) if signed => Opcode::Ashr,
            | (B::RightShift, _) => Opcode::Lshr,
            | _ =>
                return Err(LoweringError::missing(
                    "a vector operator has no lowering",
                    source,
                )),
        };
        Ok(self.binary_inst(opcode, InstFlags::empty(), x, y))
    }

    /// A unary operator on a vector: `+` copies it, `-` negates each lane
    /// and `~` complements each integer lane.
    /// C99: §6.5.3.3 paragraphs 1-4, pp. 78-79; PDF pp. 90-91.
    pub(super) fn vector_unary(
        &mut self,
        operator: UnaryOperator,
        operand: Item,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        let shape = self.vector_shape(ty, source)?;
        let from = vector_address(operand, source)?;
        if matches!(operator, UnaryOperator::Plus | UnaryOperator::Extension) {
            return Ok(Item {
                operand: Operand::Aggregate(from),
                ty,
            });
        }
        let object = self.vector_object(shape, source)?;
        for lane in 0..shape.count {
            let x = self.load_lane(from, shape, lane);
            let ty = self.draft.value_type(x);
            let value = match operator {
                | UnaryOperator::Minus if ty.is_float() => self.convert(Opcode::Fneg, ty, x),
                | UnaryOperator::Minus => {
                    let zero = self.iconst(ty, 0);
                    self.binary_inst(Opcode::Isub, InstFlags::empty(), zero, x)
                },
                | UnaryOperator::BitwiseNot => {
                    let ones = self.iconst(ty, -1);
                    self.binary_inst(Opcode::Xor, InstFlags::empty(), x, ones)
                },
                | _ =>
                    return Err(LoweringError::missing(
                        "a vector operator has no lowering",
                        source,
                    )),
            };
            self.store_lane(value, object, shape, lane);
        }
        Ok(Item {
            operand: Operand::Aggregate(object),
            ty,
        })
    }

    /// GCC's `++` and `--` on a vector: every lane one more or one less.
    /// GNU extension: GCC manual, "Vector Extensions".
    /// C99: §6.5.3.1 paragraph 2, p. 78; PDF p. 90.
    pub(super) fn vector_step(
        &mut self,
        item: Item,
        increment: bool,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        let shape = self.vector_shape(item.ty, source)?;
        let from = vector_address(item, source)?;
        let object = self.vector_object(shape, source)?;
        let ty = shape.lane_type(source)?;
        let (one, opcode) = if ty.is_float() {
            let bits = floats::from_integer(1, ty)
                .ok_or_else(|| LoweringError::missing("a floating type has no one", source))?;
            let opcode = if increment {
                Opcode::Fadd
            } else {
                Opcode::Fsub
            };
            (self.fconst(ty, bits), opcode)
        } else {
            let opcode = if increment {
                Opcode::Iadd
            } else {
                Opcode::Isub
            };
            (self.iconst(ty, 1), opcode)
        };
        for lane in 0..shape.count {
            let x = self.load_lane(from, shape, lane);
            let value = self.binary_inst(opcode, InstFlags::empty(), x, one);
            self.store_lane(value, object, shape, lane);
        }
        Ok(Item {
            operand: Operand::Aggregate(object),
            ty:      item.ty,
        })
    }

    /// A copy of a vector operand's value, which a postfix `++` keeps as
    /// its result while the store replaces the object.
    pub(super) fn vector_snapshot(
        &mut self,
        item: Item,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        let shape = self.vector_shape(item.ty, source)?;
        let from = vector_address(item, source)?;
        let object = self.vector_object(shape, source)?;
        let size = self.iconst(Type::I64, i128::from(shape.size));
        _ = self.draft.push(
            self.memory_op(InstData::MemoryRange {
                opcode: Opcode::Copy,
                flags:  InstFlags::empty(),
                align:  shape.align,
                args:   [object, from, size],
            }),
            None,
        );
        Ok(Item {
            operand: Operand::Aggregate(object),
            ty:      item.ty,
        })
    }

    /// A scalar operand of a vector operator, converted to the lane type
    /// and copied into every lane of a vector of type `to`.
    /// GNU extension: GCC manual, "Vector Extensions": "the scalar is
    /// transformed into a vector where all elements are the scalar".
    pub(super) fn vector_splat(
        &mut self,
        item: Item,
        to: TypeId,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        let shape = self.vector_shape(to, source)?;
        let lane = self.convert_value(item, shape.element, source)?;
        let value = self.value(lane, source)?;
        let object = self.vector_object(shape, source)?;
        for index in 0..shape.count {
            self.store_lane(value, object, shape, index);
        }
        Ok(Item {
            operand: Operand::Aggregate(object),
            ty:      to,
        })
    }

    /// A cast to or from a vector type: the same bits as a value of type
    /// `to`, which semantic analysis checked has the same size. A vector
    /// becomes a vector or a scalar of its size by reading its object as
    /// that type, and a scalar becomes a vector by storing it.
    /// GNU extension: GCC manual, "Vector Extensions": "It is possible to
    /// cast from one vector type to another, provided they are of the same
    /// size (in fact, you can also cast vectors to and from other datatypes
    /// of the same size)."
    pub(super) fn vector_cast(
        &mut self,
        item: Item,
        to: TypeId,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        let target = self.repr(to, source)?;
        if self.vector(item.ty).is_some() {
            let address = vector_address(item, source)?;
            return Ok(Item {
                operand: match target {
                    | Repr::Void => Operand::Void,
                    | Repr::Aggregate => Operand::Aggregate(address),
                    | representation => {
                        let ty = representation.value_type().ok_or_else(|| {
                            LoweringError::missing("a vector cast has no value type", source)
                        })?;
                        let align = self.place_alignment(address, Align::BYTE);
                        Operand::Value(self.inst(
                            InstData::Load {
                                ty,
                                flags: InstFlags::empty(),
                                align,
                                tag: None,
                                addr: address,
                            },
                            ty,
                        ))
                    },
                },
                ty:      to,
            });
        }
        let shape = self.vector_shape(to, source)?;
        let value = self.value(item, source)?;
        let object = self.vector_object(shape, source)?;
        _ = self.draft.push(
            self.memory_op(InstData::Store {
                ty:    self.draft.value_type(value),
                flags: InstFlags::empty(),
                align: shape.align,
                tag:   None,
                args:  [value, object],
            }),
            None,
        );
        Ok(Item {
            operand: Operand::Aggregate(object),
            ty:      to,
        })
    }

    /// Lane `index` of the vector `base`: a place inside the object of a
    /// vector lvalue, or the lane's value for a vector rvalue, of type
    /// `ty`. An index outside the vector is undefined, as for an array.
    /// GNU extension: GCC manual, "Vector Extensions": "Vectors can be
    /// subscripted as if the vector were an array with the same number of
    /// elements and base type."
    /// C99: §6.5.2.1 paragraph 2, p. 70; PDF p. 82.
    pub(super) fn vector_subscript(
        &mut self,
        base: Item,
        index: Item,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        let shape = self.vector_shape(base.ty, source)?;
        let address = vector_address(base, source)?;
        let guaranteed = self.place_alignment(address, shape.align);
        let signed = self.repr(index.ty, source)?.signed();
        let index = self.value(index, source)?;
        let lane = match self.draft.constant(index) {
            | Some(constant) => {
                let offset = constant.wrapping_mul(i128::from(shape.stride));
                let lane = self.iconst(Type::I64, offset);
                let lane = self.ptr_add(address, lane);
                self.retain_alignment(lane, at_offset(guaranteed, offset.unsigned_abs()));
                lane
            },
            | None => {
                let wide = self.extend(index, signed, Type::I64);
                let stride = self.iconst(Type::I64, i128::from(shape.stride));
                let offset = self.binary_inst(Opcode::Imul, InstFlags::empty(), wide, stride);
                let lane = self.ptr_add(address, offset);
                self.retain_alignment(lane, at_offset(guaranteed, u128::from(shape.stride)));
                lane
            },
        };
        let place = Place::Memory(lane);
        Ok(match base.operand {
            | Operand::Place(_) => Item {
                operand: Operand::Place(place),
                ty,
            },
            | _ => self.read(place, ty, ty.unqualified(), source)?,
        })
    }
}

// Lanes and objects

impl FunctionLowerer<'_, '_, '_, '_, '_> {
    /// The shape of the vector type `ty`, which an operator's operand or
    /// result has.
    pub(super) fn vector_shape(
        &self,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<Shape, LoweringError> {
        self.vector(ty)
            .ok_or_else(|| LoweringError::missing("a vector operand has no vector type", source))
    }

    /// A fresh object for a vector result of shape `shape`.
    pub(super) fn vector_object(
        &mut self,
        shape: Shape,
        source: SourceVectors,
    ) -> Result<Value, LoweringError> {
        let address = self.fresh_object(shape.size, shape.align, source)?;
        self.retain_alignment(address, shape.align);
        Ok(address)
    }

    /// Loads lane `index` of the vector at `base`.
    pub(super) fn load_lane(&mut self, base: Value, shape: Shape, index: u64) -> Value {
        let ty = shape.lane.value_type().unwrap_or(Type::I8);
        let offset = index * shape.stride;
        let align = at_offset(self.place_alignment(base, shape.align), u128::from(offset));
        let addr = self.offset_address(base, offset);
        let loaded = self.inst(
            InstData::Load {
                ty,
                flags: InstFlags::empty(),
                align,
                tag: None,
                addr,
            },
            ty,
        );
        self.normalize(loaded, shape.lane)
    }

    /// Stores `value` as lane `index` of the vector at `base`.
    pub(super) fn store_lane(&mut self, value: Value, base: Value, shape: Shape, index: u64) {
        let offset = index * shape.stride;
        let align = at_offset(self.place_alignment(base, shape.align), u128::from(offset));
        let addr = self.offset_address(base, offset);
        self.retain_alignment(addr, align);
        _ = self.draft.push(
            self.memory_op(InstData::Store {
                ty: self.draft.value_type(value),
                flags: InstFlags::empty(),
                align,
                tag: None,
                args: [value, addr],
            }),
            None,
        );
    }
}

const fn unsigned(cc: IntCC) -> IntCC {
    match cc {
        | IntCC::Slt => IntCC::Ult,
        | IntCC::Sgt => IntCC::Ugt,
        | IntCC::Sle => IntCC::Ule,
        | IntCC::Sge => IntCC::Uge,
        | other => other,
    }
}

// Calling conventions

/// The ABI leaf of a vector member of a structure or union, which Clang's
/// `X86_64ABIInfo::classify` gives the vector as a whole: one SSE register
/// (`f128`) for 16 bytes, an SSE eightbyte (`f64`) for 8, an INTEGER one of
/// its size for 1 to 4, and the x87 class for a lone `long double`. A
/// larger vector makes its record larger than two eightbytes, which no
/// convention passes in registers, so its leaf is never classified.
/// System V AMD64 psABI §3.2.3, "Classification", rule 1 (`__m64`,
/// `__m128` and `__m256`).
pub(super) fn member_leaf(types: &Types<'_>, ty: TypeId) -> Result<Type, LoweringErrorKind> {
    let shape = shape(types, ty).ok_or(LoweringErrorKind::MissingFact(
        "a vector member has no vector type",
    ))?;
    Ok(match shape.size {
        | 16 if shape.lane == (Repr::Float { ty: Type::F80 }) => Type::F80,
        | 16.. => Type::F128,
        | 8 => Type::F64,
        | size => int_type(8 * u32::try_from(size).unwrap_or(32)),
    })
}

/// The facts a vector parameter (`result` false) or result of type `ty` is
/// passed by under `convention`, as the compiler whose `rules` the
/// identity follows passes it: an [`AggregateFlags::VECTOR`] layout whose
/// scalars are the registers it travels in, or none for memory (System
/// V's stack and hidden result pointer, Microsoft x64's reference and
/// hidden result pointer, WebAssembly's `byval` and `sret`).
///
/// System V: both compilers pass 16 bytes in one SSE register, 8 in an SSE
/// eightbyte and 1, 2 or 4 in a general register (psABI §3.2.3 classifies
/// `__m64` as SSE and `__m128` as SSE and SSEUP; Clang's
/// `X86_64ABIInfo::classify`, "gcc passes the following as integer"). They
/// differ for one-lane vectors and wide results: GCC passes a lone `float`
/// or `double` in memory and Clang a lone `float` as an integer and a lone
/// `double` argument in memory but its result in XMM0; a lone `long double`
/// is x87 data to Clang (memory as an argument, ST0 as a result) and memory
/// to GCC; and without AVX, a 32-byte result comes back in XMM0 and XMM1
/// from Clang (LLVM legalizes `<8 x float>`) but through memory from GCC.
/// A 64-byte result, which Clang returns in XMM0-XMM3, takes GCC's memory.
///
/// Microsoft x64, as Clang passes GNU vectors there (MSVC has none): an
/// argument by reference, except a lone `float` or `double` in its XMM
/// register; a result of up to 16 bytes in XMM0 and of 32 bytes in XMM0
/// and XMM1.
///
/// WebAssembly: Clang passes a lone `float` or `double` as that value and
/// splits every other vector argument into its lanes, which bcc does not:
/// it passes the vector by reference and returns it through memory, as the
/// basic C ABI passes aggregates.
pub(super) fn abi_layout<'s>(
    types: &Types<'_>,
    ty: TypeId,
    convention: Convention,
    rules: RuleSet,
    result: bool,
    scratch: &'s Bump,
) -> Result<AggregateLayout<'s>, LoweringErrorKind> {
    let shape = shape(types, ty).ok_or(LoweringErrorKind::MissingFact(
        "a vector parameter has no vector type",
    ))?;
    let one = |ty: Type| -> &'s [AggregateScalar] {
        scratch.alloc_slice_copy(&[AggregateScalar { offset: 0, ty }])
    };
    let lone = |want: Type| shape.count == 1 && shape.lane == Repr::Float { ty: want };
    let gcc = rules == RuleSet::Gcc;
    let scalars: &'s [AggregateScalar] = match convention {
        | Convention::SystemV => match shape.size {
            | 4 if lone(Type::F32) && gcc => &[],
            | 8 if lone(Type::F64) && (gcc || !result) => &[],
            | 16 if lone(Type::F80) && gcc => &[],
            | 16 if lone(Type::F80) => one(Type::F80),
            | 1 | 2 | 4 => one(int_type(u32::try_from(8 * shape.size).unwrap_or(32))),
            | 8 => one(Type::F64),
            | 16 => one(Type::F128),
            | 32 if result && !gcc => scratch.alloc_slice_copy(&[
                AggregateScalar {
                    offset: 0,
                    ty:     Type::F128,
                },
                AggregateScalar {
                    offset: 16,
                    ty:     Type::F128,
                },
            ]),
            | _ => &[],
        },
        // An `i128` register is a whole XMM register here, as for Win64's
        // `__int128` result.
        | Convention::Win64 => match shape.size {
            | 4 if lone(Type::F32) => one(Type::F32),
            | 8 if lone(Type::F64) => one(Type::F64),
            | _ if !result => &[],
            | 1..=4 => one(Type::F32),
            | 8 => one(Type::F64),
            | 16 => one(Type::I128),
            | 32 => scratch.alloc_slice_copy(&[
                AggregateScalar {
                    offset: 0,
                    ty:     Type::I128,
                },
                AggregateScalar {
                    offset: 16,
                    ty:     Type::I128,
                },
            ]),
            | _ => &[],
        },
        | Convention::Wasm32 => match shape.size {
            | 4 if lone(Type::F32) => one(Type::F32),
            | 8 if lone(Type::F64) => one(Type::F64),
            | _ => &[],
        },
    };
    Ok(AggregateLayout {
        size: shape.size,
        align: shape.align,
        flags: AggregateFlags::VECTOR,
        scalars,
        first: None,
    })
}

// Builtins

/// A vector builtin lowering expands inline.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum VectorBuiltin {
    /// `__builtin_shufflevector(a, b, i...)`, or its two-operand form with
    /// a run-time mask.
    ShuffleVector,
    /// GCC's `__builtin_shuffle(a, mask)` and `__builtin_shuffle(a, b,
    /// mask)`.
    Shuffle,
    /// `__builtin_convertvector(v, T)`.
    ConvertVector,
    /// `__builtin_bit_cast(T, x)`.
    BitCast,
}

impl<'tu, 's> FunctionLowerer<'_, '_, 'tu, '_, 's> {
    /// Schedules a call of a vector builtin: its vector operands, then the
    /// builtin. A constant lane index of `__builtin_shufflevector` is read
    /// from its folded value and not evaluated.
    pub(super) fn schedule_vector_call(
        &mut self,
        e: &'tu Expression<'tu>,
    ) -> Result<bool, LoweringError> {
        let Some(builtin) = self.vector_builtin(e) else {
            // The elementwise and non-temporal vector builtins have no
            // lowering yet; refuse them by name rather than call them.
            if self.unvectorized_builtin(e) {
                return Err(LoweringError::unsupported(
                    super::Construct::Builtin,
                    e.source_vectors,
                ));
            }
            return Ok(false);
        };
        let arguments = self.vector_arguments(e)?;
        let evaluated = match builtin {
            | VectorBuiltin::ShuffleVector => 2,
            | _ => arguments.len(),
        };
        self.tasks.push(Task::FinishVectorBuiltin(e));
        for &argument in arguments[..evaluated.min(arguments.len())].iter().rev() {
            self.tasks.push(Task::Evaluate(argument));
        }
        Ok(true)
    }

    /// Schedules the keyword forms `__builtin_convertvector` and
    /// `__builtin_bit_cast`: their one value operand, then the builtin.
    pub(super) fn schedule_vector_keyword(
        &mut self,
        e: &'tu Expression<'tu>,
        b: &'tu Builtin<'tu>,
    ) -> bool {
        if !matches!(
            b.keyword,
            KeywordTokenType::BuiltinConvertVector | KeywordTokenType::BuiltinBitCast
        ) {
            return false;
        }
        let Some(operand) = b.operands.iter().find_map(|operand| match operand {
            | SyntaxOperand::Expression(expression) => Some(*expression),
            | _ => None,
        }) else {
            return false;
        };
        self.tasks.push(Task::FinishVectorBuiltin(e));
        self.tasks.push(Task::Evaluate(operand));
        true
    }

    /// Completes a vector builtin whose operands are on the stack.
    pub(super) fn finish_vector_builtin(
        &mut self,
        e: &'tu Expression<'tu>,
    ) -> Result<(), LoweringError> {
        let source = e.source_vectors;
        let ty = self.info(e)?.ty;
        let builtin = match e.kind {
            | ExpressionType::Builtin(b) if b.keyword == KeywordTokenType::BuiltinBitCast =>
                VectorBuiltin::BitCast,
            | ExpressionType::Builtin(_) => VectorBuiltin::ConvertVector,
            | _ => self
                .vector_builtin(e)
                .ok_or_else(|| LoweringError::missing("a vector builtin has no callee", source))?,
        };
        let result = match builtin {
            | VectorBuiltin::BitCast => {
                let item = self.pop();
                self.bit_cast(item, ty, source)?
            },
            | VectorBuiltin::ConvertVector => {
                let item = self.pop();
                self.convert_vector(item, ty, source)?
            },
            | VectorBuiltin::ShuffleVector => {
                let arguments = self.vector_arguments(e)?;
                let second = self.pop();
                let first = self.pop();
                if arguments.len() == 2 {
                    self.dynamic_shuffle(first, None, second, ty, source)?
                } else {
                    let mut indices = ArenaVec::new_in(self.scratch);
                    for &index in &arguments[2..] {
                        let value = self.info(index)?.integer.ok_or_else(|| {
                            LoweringError::missing("a shuffle index is not constant", source)
                        })?;
                        indices.push(value.value);
                    }
                    self.constant_shuffle(first, second, &indices, ty, source)?
                }
            },
            | VectorBuiltin::Shuffle => {
                let mask = self.pop();
                let second = (self.vector_arguments(e)?.len() == 3).then(|| self.pop());
                let first = self.pop();
                match second {
                    | Some(second) =>
                        self.dynamic_shuffle(first, Some(second), mask, ty, source)?,
                    | None => self.dynamic_shuffle(first, None, mask, ty, source)?,
                }
            },
        };
        self.operands.push(result);
        Ok(())
    }

    /// The vector builtin `e` calls, if it calls one.
    fn vector_builtin(&self, e: &'tu Expression<'tu>) -> Option<VectorBuiltin> {
        let call = self.unit.sema.call_facts(e)?;
        let Callee::Builtin {
            binding: Some(binding),
            ..
        } = call.callee
        else {
            return None;
        };
        match self.unit.sema.bindings[binding].origin.descriptor()?.id {
            | BuiltinId::Shufflevector => Some(VectorBuiltin::ShuffleVector),
            | BuiltinId::Shuffle => Some(VectorBuiltin::Shuffle),
            | _ => None,
        }
    }

    /// Whether `e` calls a vector builtin lowering does not expand.
    fn unvectorized_builtin(&self, e: &'tu Expression<'tu>) -> bool {
        let Some(call) = self.unit.sema.call_facts(e) else {
            return false;
        };
        let Callee::Builtin {
            binding: Some(binding),
            ..
        } = call.callee
        else {
            return false;
        };
        self.unit.sema.bindings[binding]
            .origin
            .descriptor()
            .is_some_and(|descriptor| {
                descriptor.operation == BuiltinOperation::Unsupported(UnsupportedReason::Vector)
            })
    }

    /// The argument expressions of the call `e`.
    fn vector_arguments(
        &self,
        e: &'tu Expression<'tu>,
    ) -> Result<ArenaVec<'s, &'tu Expression<'tu>>, LoweringError> {
        let source = e.source_vectors;
        let call = self
            .unit
            .sema
            .call_facts(e)
            .ok_or_else(|| LoweringError::missing("call facts are not retained", source))?;
        let mut arguments = ArenaVec::new_in(self.scratch);
        for argument in call.arguments {
            let expression = self
                .unit
                .sema
                .expressions
                .get(argument.expression.0)
                .ok_or_else(|| LoweringError::missing("a call argument is not retained", source))?
                .expression;
            arguments.push(expression);
        }
        Ok(arguments)
    }

    /// `__builtin_shufflevector` with constant indices: lane `k` of the
    /// result is lane `indices[k]` of `a` followed by `b`; an index of -1
    /// leaves the lane unspecified, which here is zero.
    /// Clang extension: Clang Language Extensions,
    /// "`__builtin_shufflevector`".
    fn constant_shuffle(
        &mut self,
        a: Item,
        b: Item,
        indices: &[i128],
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        let input = self.vector_shape(a.ty, source)?;
        let result = self.vector_shape(ty, source)?;
        let first = vector_address(a, source)?;
        let second = vector_address(b, source)?;
        let object = self.vector_object(result, source)?;
        let count = i128::from(input.count);
        for (lane, &index) in indices.iter().enumerate() {
            let value = match u64::try_from(index) {
                | Ok(index) if i128::from(index) < count => self.load_lane(first, input, index),
                | Ok(index) => self.load_lane(second, input, index - input.count),
                | Err(_) => {
                    let ty = result.lane_type(source)?;
                    if ty.is_float() {
                        self.fconst(ty, 0)
                    } else {
                        self.iconst(ty, 0)
                    }
                },
            };
            self.store_lane(value, object, result, lane as u64);
        }
        Ok(Item {
            operand: Operand::Aggregate(object),
            ty,
        })
    }

    /// A shuffle by a run-time `mask`: lane `k` of the result is lane
    /// `mask[k]` of `a`, or of `a` followed by `b`, each index taken modulo
    /// that lane count, as GCC's `__builtin_shuffle` and Clang's
    /// two-operand `__builtin_shufflevector` take it (the count is a power
    /// of two). The inputs are copied side by side first, so a lane is one
    /// indexed load.
    /// GNU extension: GCC manual, "Vector Extensions",
    /// `__builtin_shuffle`.
    fn dynamic_shuffle(
        &mut self,
        a: Item,
        b: Option<Item>,
        mask: Item,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        let input = self.vector_shape(a.ty, source)?;
        let selector = self.vector_shape(mask.ty, source)?;
        let result = self.vector_shape(ty, source)?;
        let inputs = 1 + u64::from(b.is_some());
        let lanes = input.count * inputs;
        let joined = Shape {
            count: lanes,
            size: input.stride * lanes,
            ..input
        };
        let table = self.vector_object(joined, source)?;
        for (part, item) in [Some(a), b].into_iter().flatten().enumerate() {
            let from = vector_address(item, source)?;
            for lane in 0..input.count {
                let value = self.load_lane(from, input, lane);
                self.store_lane(value, table, joined, part as u64 * input.count + lane);
            }
        }
        let indices = vector_address(mask, source)?;
        let object = self.vector_object(result, source)?;
        for lane in 0..result.count {
            let index = self.load_lane(indices, selector, lane);
            let ty = self.draft.value_type(index);
            let modulus = self.iconst(ty, i128::from(lanes) - 1);
            let index = self.binary_inst(Opcode::And, InstFlags::empty(), index, modulus);
            let index = self.extend(index, false, Type::I64);
            let stride = self.iconst(Type::I64, i128::from(input.stride));
            let offset = self.binary_inst(Opcode::Imul, InstFlags::empty(), index, stride);
            let address = self.ptr_add(table, offset);
            let align = at_offset(joined.align, u128::from(input.stride));
            self.retain_alignment(address, align);
            let ty = input.lane_type(source)?;
            let value = self.inst(
                InstData::Load {
                    ty,
                    flags: InstFlags::empty(),
                    align,
                    tag: None,
                    addr: address,
                },
                ty,
            );
            self.store_lane(value, object, result, lane);
        }
        Ok(Item {
            operand: Operand::Aggregate(object),
            ty,
        })
    }

    /// `__builtin_convertvector(v, T)`: each lane converted to `T`'s lane
    /// type as a C conversion would (§6.3.1).
    /// Clang extension: Clang Language Extensions,
    /// "`__builtin_convertvector`"; GCC manual, "Vector Extensions".
    /// C99: §6.3.1.3-§6.3.1.5, pp. 43-44; PDF pp. 55-56.
    fn convert_vector(
        &mut self,
        item: Item,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        let input = self.vector_shape(item.ty, source)?;
        let result = self.vector_shape(ty, source)?;
        let from = vector_address(item, source)?;
        let object = self.vector_object(result, source)?;
        for lane in 0..input.count {
            let value = self.load_lane(from, input, lane);
            let converted = self.convert_value(
                Item {
                    operand: Operand::Value(value),
                    ty:      input.element,
                },
                result.element,
                source,
            )?;
            let converted = self.value(converted, source)?;
            self.store_lane(converted, object, result, lane);
        }
        Ok(Item {
            operand: Operand::Aggregate(object),
            ty,
        })
    }

    /// `__builtin_bit_cast(T, x)`: the bytes of `x` read as a `T`, which
    /// has the same size: they are stored to a fresh object and read back.
    /// Clang extension: Clang Language Extensions, "`__builtin_bit_cast`".
    fn bit_cast(
        &mut self,
        item: Item,
        ty: TypeId,
        source: SourceVectors,
    ) -> Result<Item, LoweringError> {
        let layout = types::layout(&self.unit.sema.types, ty)
            .map_err(|kind| super::module_items::at(kind, source))?;
        let align = types::align(layout);
        let object = self.fresh_object(layout.size, align, source)?;
        self.retain_alignment(object, align);
        let place = Item {
            operand: Operand::Place(Place::Memory(object)),
            ty:      item.ty.unqualified(),
        };
        _ = self.write(place, item, source)?;
        self.read(Place::Memory(object), ty, ty, source)
    }
}
