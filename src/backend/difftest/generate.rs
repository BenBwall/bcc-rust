//! The random program generator: one seed gives one well-formed module.
//!
//! [`generate`] writes textual IR directly. Each function is built from
//! structured statements (integer and floating arithmetic, comparisons,
//! `select`, `freeze`, conversions, loads, stores and copies, direct and
//! indirect calls, output through `putchar` and `printf`, diamonds, counted
//! loops and switches), so every use is dominated by its
//! definition by construction: the generator keeps a scope of the values
//! that dominate the current point, and drops the values of a region when it
//! leaves one whose blocks do not dominate what follows.
//!
//! Termination is guaranteed: loops count to a bound of at most 7, the call
//! graph is acyclic apart from self-recursion guarded by a depth argument
//! that every recursive call decreases, and a cost estimate refuses calls
//! and loops that would make a run too long.
//!
//! The generator tracks a conservative signed range for every integer value,
//! and puts `nsw` and `nuw` on an operation when the ranges prove the flag
//! holds. A few flags are put on regardless, so that poison does happen;
//! the harness discards runs that turn poison into undefined behaviour.
//!
//! With `probe`, every `freeze` is preceded by a division whose divisor is
//! poison exactly when the frozen value is. The interpreter traps on it, so
//! a probe run tells whether the program ever freezes poison: such a program
//! may legitimately behave differently after optimization or under LLVM,
//! and the harness discards it. The probe consumes no randomness, so both
//! variants of a seed are the same program.

use std::fmt::Write as _;

/// The textual module for `seed`; with `probe`, instrumented as the module
/// documentation describes.
pub(super) fn generate(seed: u64, probe: bool) -> String {
    let mut rng = Rng::new(seed);
    let count = 1 + rng.index(5);
    let mut signatures: Vec<Signature> = Vec::new();
    let mut costs: Vec<u64> = Vec::new();
    let mut out = String::new();
    out.push_str(GLOBALS);
    out.push('\n');
    out.push_str("function @putchar(i32) -> i32 external\n");
    out.push_str("function @printf(ptr, ...) -> i32 external\n");
    for index in 0..=count {
        let main = index == count;
        let signature = if main {
            Signature {
                params:    Vec::new(),
                result:    Ty::I32,
                recursive: false,
            }
        } else {
            Signature::random(&mut rng)
        };
        let mut function = FunctionGen::new(
            &mut rng,
            probe,
            index,
            &signatures,
            &costs,
            &signature,
            main,
        );
        function.body(main);
        let cost = function.total_cost();
        out.push('\n');
        function.write(&mut out, &signature, main);
        signatures.push(signature);
        costs.push(cost);
    }
    out
}

/// The module's globals: `printf` formats, two zeroed data objects, and a
/// table of relocated pointers: the upper half of `@g1`, and `@f0`.
const GLOBALS: &str = "\
global @fmt_d internal constant size 4, align 1 = bytes \"25640a00\"
global @fmt_x internal constant size 5, align 1 = bytes \"5b25785d00\"
global @g0 internal size 16, align 8 = zero
global @g1 internal size 64, align 8 = zero
global @table internal constant size 16, align 8 = bytes \"00000000000000000000000000000000\" \
                       relocs [0: @g1 + 32, 8: @f0]
";

/// The data objects besides the stack slots, with their sizes: two
/// globals, and the upper half of `@g1` reached through `@table`. Copies
/// start at an object's beginning and move at most 16 bytes, so the two
/// halves of `@g1` never overlap.
const DATA_GLOBALS: [(&str, u64); 3] = [("@g0", 16), ("@g1", 64), ("@table", 32)];

/// The most a non-`main` function may cost, in estimated instructions
/// executed per call.
const FUNCTION_BUDGET: u64 = 4_000;

/// The most `main` may cost.
const MAIN_BUDGET: u64 = 20_000;

/// The deepest nesting of diamonds, loops and switches.
const MAX_DEPTH: u32 = 3;

/// The largest product of the trip counts of the enclosing loops.
const MAX_MULTIPLIER: u64 = 64;

/// The largest depth argument a call to a recursive function passes.
const MAX_RECURSION: i128 = 3;

/// The calls one activation of a recursive function makes of itself at most,
/// and the invocations a call to it costs at most: with two self-calls and
/// a depth of three, 1 + 2 + 4 + 8.
const MAX_SELF_CALLS: u32 = 2;
const RECURSION_FACTOR: u64 = 15;

/// One function's generator state.
struct FunctionGen<'g> {
    rng:         &'g mut Rng,
    probe:       bool,
    index:       usize,
    /// The signatures of the functions defined before this one, which it
    /// may call.
    signatures:  &'g [Signature],
    costs:       &'g [u64],
    recursive:   bool,
    result:      Ty,
    /// The depth parameter of a recursive function.
    depth_param: Option<Val>,
    own_params:  Vec<Ty>,
    next_value:  u32,
    blocks:      Vec<BlockText>,
    current:     usize,
    /// The values that dominate the current point.
    scope:       Vec<Val>,
    slots:       Vec<u64>,
    /// The product of the trip counts of the enclosing loops.
    multiplier:  u64,
    cost:        u64,
    budget:      u64,
    nesting:     u32,
    in_loop:     bool,
    self_calls:  u32,
}

impl FunctionGen<'_> {
    /// Generates the whole body: stack slots and their initialization,
    /// statements, and the return.
    fn body(&mut self, main: bool) {
        for _ in 0..self.rng.below(3) {
            let size = self.rng.pick(&[16, 32, 64]);
            self.slots.push(size);
        }
        for slot in 0..self.slots.len() {
            let address = self.def(
                Ty::Ptr,
                Range::FULL_PTR,
                format_args!("stack_addr slot{slot}"),
            );
            let byte = self.rng.int(256) - 128;
            let byte = self.constant_value(Ty::I8, byte);
            let size = self.constant_value(Ty::I64, i128::from(self.slots[slot]));
            self.line(format_args!(
                "fill v{}, v{}, v{}, align 8",
                address.id, byte.id, size.id
            ));
        }
        let statements = if main { 12 } else { 4 + self.rng.below(10) };
        self.statements(statements);
        if main {
            self.call_every_function();
            self.return_checksum();
        } else {
            let value = self.operand(self.result);
            self.line(format_args!("return v{}", value.id));
        }
    }

    /// `count` statements at the current point.
    fn statements(&mut self, count: u64) {
        for _ in 0..count {
            self.statement();
        }
    }

    /// One random statement.
    fn statement(&mut self) {
        let nested = self.nesting < MAX_DEPTH;
        loop {
            match self.rng.below(100) {
                | 0..30 => return self.arithmetic(),
                | 30..38 => return self.compare(),
                | 38..44 => return self.select(),
                | 44..47 => return self.freeze(),
                | 47..53 => return self.convert(),
                | 53..61 => return self.store(),
                | 61..69 => return self.load(),
                | 69..70 => return self.copy(),
                | 70..78 if self.index > 0 =>
                    if self.call() {
                        return;
                    },
                | 78..81 => return self.output(),
                | 81..87 if nested => return self.diamond(),
                | 87..92 if nested =>
                    if self.counted_loop() {
                        return;
                    },
                | 92..96 if nested => return self.switch(),
                | 96..100 if self.recursive && !self.in_loop && self.self_calls < MAX_SELF_CALLS =>
                    return self.self_call(),
                | _ => {},
            }
        }
    }

    /// An integer operation on two operands, with the flags their ranges
    /// prove, and sometimes a flag they do not.
    fn arithmetic(&mut self) {
        if self.rng.chance(10) {
            return self.exact_pair();
        }
        if self.rng.chance(12) {
            return self.float();
        }
        if self.rng.chance(8) {
            return self.boolean_arithmetic();
        }
        let ty = self.int_type();
        let a = self.operand(ty);
        let b = if self.rng.chance(8) {
            a
        } else {
            self.operand(ty)
        };
        let op = self.rng.pick(&BINARY);
        match op {
            | "sdiv" | "udiv" | "srem" | "urem" => self.division(op, a),
            | "shl" | "lshr" | "ashr" => self.shift(op, a, b),
            | _ => {
                let (range, nsw, nuw) = binary_range(op, ty, a.range, b.range);
                let flags = self.flags(op, nsw, nuw);
                let _ = self.def(
                    ty,
                    range,
                    format_args!("{op}.{ty}{flags} v{}, v{}", a.id, b.id),
                );
            },
        }
    }

    /// A floating operation: arithmetic, a comparison, or a conversion to or
    /// from an integer or the other floating type. Floating values reach
    /// the checksum only through comparisons and conversions to integers,
    /// so NaN payloads, which LLVM does not fix, are never observed.
    fn float(&mut self) {
        let ty = if self.rng.chance(50) {
            Ty::F32
        } else {
            Ty::F64
        };
        let range = Range::FULL_PTR;
        match self.rng.below(6) {
            | 0 => {
                let a = self.float_operand(ty);
                let b = self.float_operand(ty);
                let op = self.rng.pick(&["fadd", "fsub", "fmul", "fdiv", "frem"]);
                let _ = self.def(ty, range, format_args!("{op}.{ty} v{}, v{}", a.id, b.id));
            },
            | 1 => {
                let a = self.float_operand(ty);
                let _ = self.def(ty, range, format_args!("fneg.{ty} v{}", a.id));
            },
            | 2 => {
                let a = self.float_operand(ty);
                let b = if self.rng.chance(10) {
                    a
                } else {
                    self.float_operand(ty)
                };
                let condition = self.rng.pick(&FLOAT_CONDITIONS);
                let _ = self.def(
                    Ty::I1,
                    Range::new(0, 1),
                    format_args!("fcmp.{ty} {condition} v{}, v{}", a.id, b.id),
                );
            },
            | 3 => {
                let a = self.float_operand(ty);
                let to = self.int_type();
                let op = if self.rng.chance(50) {
                    "fptosi"
                } else {
                    "fptoui"
                };
                let _ = self.def(to, Range::full(to), format_args!("{op}.{to} v{}", a.id));
            },
            | 4 => {
                let (from, op) = if ty == Ty::F32 {
                    (Ty::F64, "fptrunc")
                } else {
                    (Ty::F32, "fpext")
                };
                let a = self.float_operand(from);
                let _ = self.def(ty, range, format_args!("{op}.{ty} v{}", a.id));
            },
            | _ => {
                let from = self.rng.pick(&[Ty::I1, Ty::I8, Ty::I32, Ty::I64]);
                let a = self.operand(from);
                let op = if self.rng.chance(50) {
                    "sitofp"
                } else {
                    "uitofp"
                };
                let _ = self.def(ty, range, format_args!("{op}.{ty} v{}", a.id));
            },
        }
    }

    /// A floating operand: a value in scope, or a constant.
    fn float_operand(&mut self, ty: Ty) -> Val {
        if self.rng.chance(60)
            && let Some(value) = self.pick(ty)
        {
            return value;
        }
        let value = self.rng.pick(&FLOAT_CONSTANTS);
        #[expect(
            clippy::cast_possible_truncation,
            reason = "Rounding to the nearest `f32` is the intent."
        )]
        let single = value as f32;
        let bits = if ty == Ty::F32 {
            format!("0x{:08X}", single.to_bits())
        } else {
            format!("0x{:016X}", value.to_bits())
        };
        self.def(ty, Range::FULL_PTR, format_args!("fconst.{ty} {bits}"))
    }

    /// A multiplication or left shift of a masked operand, undone by an
    /// `exact` division or right shift that the ranges prove exact.
    fn exact_pair(&mut self) {
        let ty = self.rng.pick(&[Ty::I8, Ty::I32, Ty::I64]);
        let value = self.operand(ty);
        let mask = self.constant_value(ty, 15);
        let small = self.def(
            ty,
            Range::new(0, 15),
            format_args!("and.{ty} v{}, v{}", value.id, mask.id),
        );
        let (scale, undo) = if self.rng.chance(50) {
            let factor = self.rng.pick(&[2, 3, 5, 7]);
            let op = if self.rng.chance(50) { "sdiv" } else { "udiv" };
            (("imul", factor), (op, factor))
        } else {
            let shift = 1 + self.rng.int(3);
            let op = if self.rng.chance(50) { "lshr" } else { "ashr" };
            (("shl", shift), (op, shift))
        };
        let operand = self.constant_value(ty, scale.1);
        let factor = if scale.0 == "shl" {
            1 << scale.1
        } else {
            scale.1
        };
        let flags = self.flags(scale.0, true, true);
        let scaled = self.def(
            ty,
            Range::new(0, 15 * factor),
            format_args!("{}.{ty}{flags} v{}, v{}", scale.0, small.id, operand.id),
        );
        let operand = self.constant_value(ty, undo.1);
        let _ = self.def(
            ty,
            Range::new(0, 15),
            format_args!("{}.{ty} exact v{}, v{}", undo.0, scaled.id, operand.id),
        );
    }

    /// `and`, `or`, `xor`, `iadd`, `isub` or `imul` of two conditions.
    fn boolean_arithmetic(&mut self) {
        let a = self.condition();
        let b = self.condition();
        let op = self.rng.pick(&["and", "or", "xor", "iadd", "isub", "imul"]);
        let flags = self.flags(op, false, false);
        let _ = self.def(
            Ty::I1,
            Range::new(0, 1),
            format_args!("{op}.i1{flags} v{}, v{}", a.id, b.id),
        );
    }

    /// A division or remainder by a divisor that cannot be zero or -1: a
    /// small constant, or an operand masked into 1..=63.
    fn division(&mut self, op: &str, a: Val) {
        let ty = a.ty;
        let divisor = if self.rng.chance(40) {
            let value = self.rng.pick(&[1, 2, 3, 7, 10, -2, -3, -7]);
            self.constant_value(ty, value)
        } else {
            let operand = self.operand(ty);
            let mask = self.constant_value(ty, 63);
            let masked = self.def(
                ty,
                Range::new(0, 63),
                format_args!("and.{ty} v{}, v{}", operand.id, mask.id),
            );
            let one = self.constant_value(ty, 1);
            self.def(
                ty,
                Range::new(1, 63),
                format_args!("or.{ty} v{}, v{}", masked.id, one.id),
            )
        };
        let full = Range::full(ty);
        let range = match op {
            | "sdiv" if divisor.range.lo > 0 => Range::new(a.range.lo.min(0), a.range.hi.max(0)),
            | "srem" => {
                let bound = divisor.range.lo.abs().max(divisor.range.hi.abs()) - 1;
                Range::new(-bound, bound)
            },
            | "urem" if divisor.range.lo > 0 => Range::new(0, divisor.range.hi - 1),
            | "udiv" if a.range.lo >= 0 && divisor.range.lo > 0 => Range::new(0, a.range.hi),
            | _ => full,
        };
        let exact = if matches!(op, "sdiv" | "udiv") && self.rng.chance(4) {
            " exact"
        } else {
            ""
        };
        let _ = self.def(
            ty,
            range,
            format_args!("{op}.{ty}{exact} v{}, v{}", a.id, divisor.id),
        );
    }

    /// A shift by a constant below the width, or by an operand masked below
    /// it, or now and then by an operand as it is.
    fn shift(&mut self, op: &str, a: Val, b: Val) {
        let ty = a.ty;
        let width = i128::from(ty.bits());
        let amount = match self.rng.below(10) {
            | 0..5 => {
                let amount = self.rng.int(width);
                self.constant_value(ty, amount)
            },
            | 5..9 => {
                let mask = self.constant_value(ty, width - 1);
                self.def(
                    ty,
                    Range::new(0, width - 1),
                    format_args!("and.{ty} v{}, v{}", b.id, mask.id),
                )
            },
            | _ => b,
        };
        let constant = amount
            .constant
            .filter(|_| amount.range.lo == amount.range.hi);
        let (range, nsw, nuw) = match (op, constant) {
            | ("shl", Some(k)) if (0..width).contains(&k) => {
                let factor = 1_i128 << k;
                let (lo, hi) = (a.range.lo * factor, a.range.hi * factor);
                let fits = ty.contains(lo) && ty.contains(hi);
                let range = if fits {
                    Range::new(lo, hi)
                } else {
                    Range::full(ty)
                };
                (range, fits, fits && a.range.lo >= 0)
            },
            | ("ashr", Some(k)) if (0..width).contains(&k) =>
                (Range::new(a.range.lo >> k, a.range.hi >> k), false, false),
            | ("lshr", Some(k)) if (0..width).contains(&k) => {
                let range = if a.range.lo >= 0 {
                    Range::new(a.range.lo >> k, a.range.hi >> k)
                } else if k > 0 {
                    Range::new(0, ty.umax() >> k)
                } else {
                    a.range
                };
                (range, false, false)
            },
            | _ => (Range::full(ty), false, false),
        };
        let flags = if op == "shl" {
            self.flags(op, nsw, nuw)
        } else if self.rng.chance(4) {
            " exact".to_string()
        } else {
            String::new()
        };
        let _ = self.def(
            ty,
            range,
            format_args!("{op}.{ty}{flags} v{}, v{}", a.id, amount.id),
        );
    }

    /// The flags of an `iadd`, `isub`, `imul` or `shl`: each proven one with
    /// probability 0.6, each unproven one with probability 0.03.
    fn flags(&mut self, op: &str, nsw: bool, nuw: bool) -> String {
        if !matches!(op, "iadd" | "isub" | "imul" | "shl") {
            return String::new();
        }
        let mut flags = String::new();
        for (name, proven) in [("nsw", nsw), ("nuw", nuw)] {
            let chance = if proven { 60 } else { 3 };
            if self.rng.chance(chance) {
                flags.push(' ');
                flags.push_str(name);
            }
        }
        flags
    }

    /// An `icmp`, and sometimes its result widened to an integer.
    fn compare(&mut self) {
        let condition = self.new_condition();
        if self.rng.chance(50) {
            let ty = self.int_type();
            let op = if self.rng.chance(50) { "zext" } else { "sext" };
            let range = if op == "zext" {
                Range::new(0, 1)
            } else {
                Range::new(-1, 0)
            };
            let _ = self.def(ty, range, format_args!("{op}.{ty} v{}", condition.id));
        }
    }

    fn select(&mut self) {
        let condition = self.condition();
        let ty = self.int_type();
        let a = self.operand(ty);
        let b = self.operand(ty);
        let _ = self.def(
            ty,
            a.range.union(b.range),
            format_args!("select.{ty} v{}, v{}, v{}", condition.id, a.id, b.id),
        );
    }

    /// A `freeze`, preceded in a probe run by the division that traps if
    /// the operand is poison.
    fn freeze(&mut self) {
        let ty = self.int_type();
        let value = self.operand(ty);
        if self.probe {
            // Pushed past `line`, so that the cost estimate, and with it the
            // rest of the program, is the same in both variants.
            let one = self.fresh();
            let either = self.fresh();
            let quotient = self.fresh();
            self.blocks[self.current].lines.extend([
                format!("v{one} = iconst.{ty} 1"),
                format!("v{either} = or.{ty} v{}, v{one}", value.id),
                format!("v{quotient} = udiv.{ty} v{one}, v{either}"),
            ]);
        }
        let _ = self.def(ty, value.range, format_args!("freeze.{ty} v{}", value.id));
    }

    /// A `zext`, `sext` or `trunc` between two integer types.
    fn convert(&mut self) {
        let from = self.rng.pick(&[Ty::I1, Ty::I8, Ty::I32, Ty::I64]);
        let to = if self.rng.chance(15) {
            Ty::I1
        } else {
            self.int_type()
        };
        if from == to {
            return;
        }
        let value = self.operand(from);
        let signed = self.rng.chance(50);
        let _ = self.convert_value(value, to, signed);
    }

    /// `value` converted to `to`: truncated if `to` is narrower, else
    /// extended with sign or zero bits.
    fn convert_value(&mut self, value: Val, to: Ty, signed: bool) -> Val {
        let from = value.ty;
        if from == to {
            return value;
        }
        if from.bits() > to.bits() {
            let range =
                if to != Ty::I1 && to.contains(value.range.lo) && to.contains(value.range.hi) {
                    value.range
                } else {
                    Range::full(to)
                };
            return self.def(to, range, format_args!("trunc.{to} v{}", value.id));
        }
        let (op, range) = match (from, signed) {
            | (Ty::I1, false) => ("zext", Range::new(0, 1)),
            | (Ty::I1, true) => ("sext", Range::new(-1, 0)),
            | (_, true) => ("sext", value.range),
            | (_, false) if value.range.lo >= 0 => ("zext", value.range),
            | (_, false) => ("zext", Range::new(0, from.umax())),
        };
        self.def(to, range, format_args!("{op}.{to} v{}", value.id))
    }

    fn store(&mut self) {
        let ty = self.rng.pick(&[Ty::I8, Ty::I32, Ty::I64]);
        let address = self.address(ty);
        let value = self.operand(ty);
        self.line(format_args!(
            "store.{ty} v{}, v{}, align {}",
            value.id,
            address.id,
            ty.bytes()
        ));
    }

    fn load(&mut self) {
        let ty = self.rng.pick(&[Ty::I8, Ty::I32, Ty::I64]);
        let address = self.address(ty);
        let _ = self.def(
            ty,
            Range::full(ty),
            format_args!("load.{ty} v{}, align {}", address.id, ty.bytes()),
        );
    }

    /// A `copy` of 8 or 16 bytes between two different objects.
    fn copy(&mut self) {
        let objects = self.slots.len() + DATA_GLOBALS.len();
        let first = self.rng.index(objects);
        let second = (first + 1 + self.rng.index(objects - 1)) % objects;
        let size = if self.rng.chance(50) { 8 } else { 16 };
        let destination = self.object_address(first);
        let source = self.object_address(second);
        let size = self.constant_value(Ty::I64, size);
        self.line(format_args!(
            "copy v{}, v{}, v{}, align 8",
            destination.id, source.id, size.id
        ));
    }

    /// The address of an aligned element of type `ty` in a stack slot or a
    /// data global: at a constant offset, or at an index masked into the
    /// object.
    fn address(&mut self, ty: Ty) -> Val {
        let objects = self.slots.len() + DATA_GLOBALS.len();
        let object = self.rng.index(objects);
        let size = self.object_size(object);
        let base = self.object_address(object);
        let elements = size / ty.bytes();
        let offset = if self.rng.chance(50) {
            let element = self.rng.below(elements);
            if element == 0 && self.rng.chance(50) {
                return base;
            }
            self.constant_value(Ty::I64, i128::from(element * ty.bytes()))
        } else {
            let index_ty = self.int_type();
            let index = self.operand(index_ty);
            let mask = self.constant_value(index_ty, i128::from(elements - 1));
            let masked = self.def(
                index_ty,
                Range::new(0, i128::from(elements - 1)),
                format_args!("and.{index_ty} v{}, v{}", index.id, mask.id),
            );
            let signed = self.rng.chance(50);
            let wide = self.convert_value(masked, Ty::I64, signed);
            let scale = self.constant_value(Ty::I64, i128::from(ty.bytes()));
            let flags = self.flags("imul", true, true);
            self.def(
                Ty::I64,
                Range::new(0, i128::from(size - ty.bytes())),
                format_args!("imul.i64{flags} v{}, v{}", wide.id, scale.id),
            )
        };
        self.def(
            Ty::Ptr,
            Range::FULL_PTR,
            format_args!("ptr_add inbounds v{}, v{}", base.id, offset.id),
        )
    }

    /// The address of stack slot `object`, or of a data global after the
    /// slots.
    fn object_address(&mut self, object: usize) -> Val {
        if object < self.slots.len() {
            self.def(
                Ty::Ptr,
                Range::FULL_PTR,
                format_args!("stack_addr slot{object}"),
            )
        } else {
            let (name, _) = DATA_GLOBALS[object - self.slots.len()];
            let address = self.def(Ty::Ptr, Range::FULL_PTR, format_args!("global_addr {name}"));
            if name == "@table" {
                return self.def(
                    Ty::Ptr,
                    Range::FULL_PTR,
                    format_args!("load.ptr v{}, align 8", address.id),
                );
            }
            address
        }
    }

    fn object_size(&self, object: usize) -> u64 {
        self.slots
            .get(object)
            .copied()
            .unwrap_or_else(|| DATA_GLOBALS[object - self.slots.len()].1)
    }

    /// A call of an earlier function, unless it would exceed the budget.
    fn call(&mut self) -> bool {
        let callee = self.rng.index(self.index);
        self.call_function(callee)
    }

    fn call_function(&mut self, callee: usize) -> bool {
        let cost = self.multiplier * self.costs[callee];
        if self.cost + cost > self.budget {
            return false;
        }
        self.cost += cost;
        let signature = &self.signatures[callee];
        let params = signature.params.clone();
        let result = signature.result;
        let recursive = signature.recursive;
        let mut args = Vec::new();
        for (position, &ty) in params.iter().enumerate() {
            let arg = if recursive && position == 0 {
                let depth = self.rng.int(MAX_RECURSION + 1);
                self.constant_value(Ty::I32, depth)
            } else {
                self.operand(ty)
            };
            args.push(format!("v{}", arg.id));
        }
        let args = args.join(", ");
        if self.rng.chance(25) {
            // An indirect call, through `@table` for `@f0`.
            let callee_address = if callee == 0 && self.rng.chance(50) {
                let table = self.def(Ty::Ptr, Range::FULL_PTR, format_args!("global_addr @table"));
                let offset = self.constant_value(Ty::I64, 8);
                let slot = self.def(
                    Ty::Ptr,
                    Range::FULL_PTR,
                    format_args!("ptr_add inbounds v{}, v{}", table.id, offset.id),
                );
                self.def(
                    Ty::Ptr,
                    Range::FULL_PTR,
                    format_args!("load.ptr v{}, align 8", slot.id),
                )
            } else {
                self.def(
                    Ty::Ptr,
                    Range::FULL_PTR,
                    format_args!("func_addr @f{callee}"),
                )
            };
            let types: Vec<&str> = params.iter().map(|ty| ty.name()).collect();
            let _ = self.def(
                result,
                Range::full(result),
                format_args!(
                    "call_indirect v{}({args}) : ({}) -> {result}",
                    callee_address.id,
                    types.join(", ")
                ),
            );
        } else {
            let _ = self.def(
                result,
                Range::full(result),
                format_args!("call @f{callee}({args})"),
            );
        }
        true
    }

    /// A call of `putchar` with a letter, or of `printf` with an `i32`.
    fn output(&mut self) {
        let value = self.operand(Ty::I32);
        if self.rng.chance(50) {
            let mask = self.constant_value(Ty::I32, 15);
            let low = self.def(
                Ty::I32,
                Range::new(0, 15),
                format_args!("and.i32 v{}, v{}", value.id, mask.id),
            );
            let base = self.constant_value(Ty::I32, 65);
            let letter = self.def(
                Ty::I32,
                Range::new(65, 80),
                format_args!("iadd.i32 v{}, v{}", low.id, base.id),
            );
            let _ = self.def(
                Ty::I32,
                Range::full(Ty::I32),
                format_args!("call @putchar(v{})", letter.id),
            );
        } else {
            let format = if self.rng.chance(50) {
                "@fmt_d"
            } else {
                "@fmt_x"
            };
            let format = self.def(
                Ty::Ptr,
                Range::FULL_PTR,
                format_args!("global_addr {format}"),
            );
            let _ = self.def(
                Ty::I32,
                Range::full(Ty::I32),
                format_args!("call @printf(v{}, v{})", format.id, value.id),
            );
        }
    }

    /// An if-then-else whose arms meet in a block with parameters. One arm
    /// may branch straight to the merge, and both may, with equal arguments.
    fn diamond(&mut self) {
        let condition = self.condition();
        let types = self.merge_types();
        let (merge, params) = self.new_block(&types);
        let shape = self.rng.below(10);
        if shape == 0 {
            let args = self.arguments(&types);
            let edge = block_call(merge, &args);
            self.line(format_args!("brif v{}, {edge}, {edge}", condition.id));
            self.enter_merge(merge, &params, &[args]);
            return;
        }
        let (then, _) = self.new_block(&[]);
        if shape <= 2 {
            let direct = self.arguments(&types);
            self.line(format_args!(
                "brif v{}, block{then}, {}",
                condition.id,
                block_call(merge, &direct)
            ));
            let args = self.arm(then, merge, &types);
            self.enter_merge(merge, &params, &[direct, args]);
            return;
        }
        let (otherwise, _) = self.new_block(&[]);
        self.line(format_args!(
            "brif v{}, block{then}, block{otherwise}",
            condition.id
        ));
        let then_args = self.arm(then, merge, &types);
        let else_args = self.arm(otherwise, merge, &types);
        self.enter_merge(merge, &params, &[then_args, else_args]);
    }

    /// Generates the region starting at block `start`, which ends by jumping
    /// to `merge`; its values leave the scope afterwards. Returns the
    /// arguments it passed.
    fn arm(&mut self, start: usize, merge: usize, types: &[Ty]) -> Vec<Val> {
        self.current = start;
        let mark = self.scope.len();
        self.nesting += 1;
        let count = self.rng.below(5);
        self.statements(count);
        let args = self.arguments(types);
        self.line(format_args!("jump {}", block_call(merge, &args)));
        self.nesting -= 1;
        self.scope.truncate(mark);
        args
    }

    /// Continues in `merge`, whose parameters take the union of the ranges
    /// of the arguments each edge passes.
    fn enter_merge(&mut self, merge: usize, params: &[u32], edges: &[Vec<Val>]) {
        self.current = merge;
        for (position, &id) in params.iter().enumerate() {
            let ty = edges[0][position].ty;
            let range = edges
                .iter()
                .map(|args| args[position].range)
                .reduce(Range::union)
                .unwrap_or_else(|| Range::full(ty));
            self.scope.push(Val {
                id,
                ty,
                range,
                constant: None,
            });
        }
    }

    /// A loop counting from zero to a bound of at most 7, carrying one or two
    /// accumulators: tested at the top, or, with a bound of at least 1, at
    /// the bottom. Refused if the enclosing loops already multiply too much.
    fn counted_loop(&mut self) -> bool {
        let bottom = self.rng.chance(40);
        let counter = if self.rng.chance(70) {
            Ty::I32
        } else {
            Ty::I64
        };
        let (bound, max_trips) = if self.rng.chance(60) {
            let trips = i128::from(u8::from(bottom)) + self.rng.int(7 - i128::from(bottom));
            (self.constant_value(counter, trips), trips)
        } else {
            let value = self.operand(counter);
            let mask = self.constant_value(counter, 7);
            let masked = self.def(
                counter,
                Range::new(0, 7),
                format_args!("and.{counter} v{}, v{}", value.id, mask.id),
            );
            if bottom {
                let one = self.constant_value(counter, 1);
                let bound = self.def(
                    counter,
                    Range::new(1, 7),
                    format_args!("or.{counter} v{}, v{}", masked.id, one.id),
                );
                (bound, 7)
            } else {
                (masked, 7)
            }
        };
        let trips = u64::try_from(max_trips.max(1)).expect("a trip count is positive");
        if self.multiplier * trips > MAX_MULTIPLIER {
            return false;
        }
        let mut types = vec![counter];
        types.extend((0..=self.rng.below(2)).map(|_| self.int_type()));
        let zero = self.constant_value(counter, 0);
        let mut inits = vec![zero];
        inits.extend(types[1..].iter().map(|&ty| self.operand(ty)));
        let (header, params) = self.new_block(&types);
        self.line(format_args!("jump {}", block_call(header, &inits)));
        self.current = header;
        let bound_range = bound.range;
        let index = Val {
            id:       params[0],
            ty:       counter,
            range:    Range::new(0, bound_range.hi),
            constant: None,
        };
        self.scope.push(index);
        let mut carried = vec![index];
        for (position, &id) in params.iter().enumerate().skip(1) {
            let ty = types[position];
            let value = Val {
                id,
                ty,
                range: Range::full(ty),
                constant: None,
            };
            self.scope.push(value);
            carried.push(value);
        }
        let outer = (self.multiplier, self.in_loop);
        self.multiplier *= trips;
        self.in_loop = true;
        self.nesting += 1;
        let compare = |rng: &mut Rng| rng.pick(&["slt", "ult", "ne"]);
        if bottom {
            let count = 1 + self.rng.below(5);
            self.statements(count);
            let next = self.increment(index, bound_range.hi);
            let condition = compare(self.rng);
            let test = self.def(
                Ty::I1,
                Range::new(0, 1),
                format_args!("icmp.{counter} {condition} v{}, v{}", next.id, bound.id),
            );
            let mut args = vec![next];
            args.extend(types[1..].iter().map(|&ty| self.carried(&carried, ty)));
            let (exit, _) = self.new_block(&[]);
            self.line(format_args!(
                "brif v{}, {}, block{exit}",
                test.id,
                block_call(header, &args)
            ));
            self.current = exit;
        } else {
            let condition = compare(self.rng);
            let test = self.def(
                Ty::I1,
                Range::new(0, 1),
                format_args!("icmp.{counter} {condition} v{}, v{}", index.id, bound.id),
            );
            let (body, _) = self.new_block(&[]);
            let (exit, _) = self.new_block(&[]);
            self.line(format_args!("brif v{}, block{body}, block{exit}", test.id));
            self.current = body;
            let mark = self.scope.len();
            let count = 1 + self.rng.below(5);
            self.statements(count);
            let next = self.increment(index, bound_range.hi);
            let mut args = vec![next];
            args.extend(types[1..].iter().map(|&ty| self.carried(&carried, ty)));
            self.line(format_args!("jump {}", block_call(header, &args)));
            self.scope.truncate(mark);
            self.current = exit;
        }
        self.nesting -= 1;
        (self.multiplier, self.in_loop) = outer;
        true
    }

    /// The next value of a loop accumulator of type `ty`: often one of the
    /// header's parameters, so that edges permute them.
    fn carried(&mut self, header: &[Val], ty: Ty) -> Val {
        let same: Vec<Val> = header
            .iter()
            .filter(|value| value.ty == ty)
            .copied()
            .collect();
        if !same.is_empty() && self.rng.chance(35) {
            return self.rng.pick(&same);
        }
        self.operand(ty)
    }

    /// The loop index plus one, which cannot overflow below the bound.
    fn increment(&mut self, index: Val, bound: i128) -> Val {
        let ty = index.ty;
        let one = self.constant_value(ty, 1);
        let flags = self.flags("iadd", true, true);
        self.def(
            ty,
            Range::new(1, bound.max(1)),
            format_args!("iadd.{ty}{flags} v{}, v{}", index.id, one.id),
        )
    }

    /// A `switch` on an operand, often masked into the case range, whose
    /// targets meet in a block with parameters. Sometimes the cases cover
    /// every masked value and the default is `unreachable`.
    fn switch(&mut self) {
        let ty = self.rng.pick(&[Ty::I8, Ty::I32, Ty::I64]);
        let covered = self.rng.chance(15);
        let value = self.operand(ty);
        let selector = if covered || self.rng.chance(60) {
            let mask = if covered { 3 } else { 7 };
            let mask_value = self.constant_value(ty, mask);
            self.def(
                ty,
                Range::new(0, mask),
                format_args!("and.{ty} v{}, v{}", value.id, mask_value.id),
            )
        } else {
            value
        };
        let mut cases: Vec<i128> = if covered {
            vec![0, 1, 2, 3]
        } else {
            let mut candidates = vec![0, 1, 2, 3, 4, 5, 6, 7, -1, 100, ty.min(), ty.max()];
            let count = 1 + self.rng.index(4);
            let mut chosen = Vec::new();
            for _ in 0..count {
                let pick = self.rng.index(candidates.len());
                chosen.push(candidates.swap_remove(pick));
            }
            chosen
        };
        if !covered {
            cases.sort_unstable();
        }
        let types = self.merge_types();
        let (merge, params) = self.new_block(&types);
        let mut direct: Option<Vec<Val>> = None;
        // Each target: a fresh block, the previous case's block, or the merge
        // block directly (at most once, so its edges pass equal arguments).
        let mut targets: Vec<String> = Vec::new();
        let mut fresh: Vec<usize> = Vec::new();
        let default = if covered {
            let (block, _) = self.new_block(&[]);
            self.blocks[block].lines.push("unreachable".to_string());
            format!("block{block}")
        } else if self.rng.chance(20) {
            let args = self.arguments(&types);
            direct = Some(args.clone());
            block_call(merge, &args)
        } else {
            let (block, _) = self.new_block(&[]);
            fresh.push(block);
            format!("block{block}")
        };
        for position in 0..cases.len() {
            let choice = self.rng.below(10);
            if choice < 2 && position > 0 {
                let previous = targets[position - 1].clone();
                targets.push(previous);
            } else if choice < 4 && direct.is_none() {
                let args = self.arguments(&types);
                targets.push(block_call(merge, &args));
                direct = Some(args);
            } else {
                let (block, _) = self.new_block(&[]);
                fresh.push(block);
                targets.push(format!("block{block}"));
            }
        }
        let list: Vec<String> = cases
            .iter()
            .zip(&targets)
            .map(|(case, target)| format!("{case}: {target}"))
            .collect();
        self.line(format_args!(
            "switch v{}, {default}, [{}]",
            selector.id,
            list.join(", ")
        ));
        let mut edges: Vec<Vec<Val>> = direct.into_iter().collect();
        for block in fresh {
            edges.push(self.arm(block, merge, &types));
        }
        self.enter_merge(merge, &params, &edges);
    }

    /// In a recursive function, a call of itself guarded by its depth
    /// parameter, which the call decreases.
    fn self_call(&mut self) {
        let Some(depth) = self.depth_param else {
            return;
        };
        self.self_calls += 1;
        let zero = self.constant_value(Ty::I32, 0);
        let positive = self.def(
            Ty::I1,
            Range::new(0, 1),
            format_args!("icmp.i32 sgt v{}, v{}", depth.id, zero.id),
        );
        let result = self.result;
        let fallback = self.operand(result);
        let (call, _) = self.new_block(&[]);
        let (merge, params) = self.new_block(&[result]);
        self.line(format_args!(
            "brif v{}, block{call}, {}",
            positive.id,
            block_call(merge, &[fallback])
        ));
        self.current = call;
        let mark = self.scope.len();
        let one = self.constant_value(Ty::I32, 1);
        let decreased = self.def(
            Ty::I32,
            Range::full(Ty::I32),
            format_args!("isub.i32 nsw v{}, v{}", depth.id, one.id),
        );
        let own_params = self.own_params.clone();
        let mut args = vec![format!("v{}", decreased.id)];
        for &ty in &own_params[1..] {
            let arg = self.operand(ty);
            args.push(format!("v{}", arg.id));
        }
        let index = self.index;
        let value = self.def(
            result,
            Range::full(result),
            format_args!("call @f{index}({})", args.join(", ")),
        );
        self.line(format_args!("jump {}", block_call(merge, &[value])));
        self.scope.truncate(mark);
        self.enter_merge(merge, &params, &[vec![fallback], vec![value]]);
    }

    /// In `main`: a call of every function, so that each one runs.
    fn call_every_function(&mut self) {
        for callee in 0..self.index {
            let _ = self.call_function(callee);
        }
    }

    /// In `main`: returns an `i32` that mixes up to eight values in scope.
    fn return_checksum(&mut self) {
        let mut sum = self.constant_value(Ty::I32, 0x1234_5678);
        let candidates: Vec<Val> = self
            .scope
            .iter()
            .rev()
            .filter(|value| value.ty.is_int())
            .take(8)
            .copied()
            .collect();
        for value in candidates {
            let wide = self.convert_value(value, Ty::I32, true);
            let op = if self.rng.chance(50) { "xor" } else { "iadd" };
            sum = self.def(
                Ty::I32,
                Range::full(Ty::I32),
                format_args!("{op}.i32 v{}, v{}", sum.id, wide.id),
            );
            let factor = self.constant_value(Ty::I32, 31);
            sum = self.def(
                Ty::I32,
                Range::full(Ty::I32),
                format_args!("imul.i32 v{}, v{}", sum.id, factor.id),
            );
        }
        self.line(format_args!("return v{}", sum.id));
    }

    /// A condition: one in scope, a new comparison, or a constant.
    fn condition(&mut self) -> Val {
        match self.rng.below(10) {
            | 0..5 => match self.pick(Ty::I1) {
                | Some(value) => value,
                | None => self.new_condition(),
            },
            | 5..9 => self.new_condition(),
            | _ => {
                let value = self.rng.int(2);
                self.constant_value(Ty::I1, value)
            },
        }
    }

    /// An `icmp` of two operands, or of one operand with itself.
    fn new_condition(&mut self) -> Val {
        let ty = self.int_type();
        let a = self.operand(ty);
        let b = if self.rng.chance(8) {
            a
        } else {
            self.operand(ty)
        };
        let condition = self.rng.pick(&CONDITIONS);
        self.def(
            Ty::I1,
            Range::new(0, 1),
            format_args!("icmp.{ty} {condition} v{}, v{}", a.id, b.id),
        )
    }

    /// The types of a merge block's parameters: one to three integers.
    fn merge_types(&mut self) -> Vec<Ty> {
        (0..=self.rng.below(3)).map(|_| self.int_type()).collect()
    }

    fn arguments(&mut self, types: &[Ty]) -> Vec<Val> {
        types.iter().map(|&ty| self.operand(ty)).collect()
    }

    /// An operand of type `ty`: usually a value in scope, else a constant or
    /// a conversion of another value.
    fn operand(&mut self, ty: Ty) -> Val {
        if self.rng.chance(70)
            && let Some(value) = self.pick(ty)
        {
            return value;
        }
        if ty != Ty::I1 && self.rng.chance(30) {
            let from = self.rng.pick(&[Ty::I8, Ty::I32, Ty::I64]);
            if from != ty
                && let Some(value) = self.pick(from)
            {
                let signed = self.rng.chance(50);
                return self.convert_value(value, ty, signed);
            }
        }
        if ty == Ty::I1 {
            return self.new_condition();
        }
        if self.rng.chance(1) {
            return self.def(ty, Range::full(ty), format_args!("poison.{ty}"));
        }
        self.constant(ty)
    }

    /// A value of type `ty` in scope, favouring recent ones.
    fn pick(&mut self, ty: Ty) -> Option<Val> {
        let candidates: Vec<Val> = self.scope.iter().filter(|v| v.ty == ty).copied().collect();
        if candidates.is_empty() {
            return None;
        }
        let window = if self.rng.chance(50) {
            candidates.len().min(6)
        } else {
            candidates.len()
        };
        let choice = candidates.len() - 1 - self.rng.index(window);
        Some(candidates[choice])
    }

    /// A new constant, often an edge case of its type.
    fn constant(&mut self, ty: Ty) -> Val {
        let value = match self.rng.below(12) {
            | 0 => 0,
            | 1 => 1,
            | 2 => -1,
            | 3 => ty.min(),
            | 4 => ty.max(),
            | 5 => ty.min() + 1,
            | 6 => ty.max() - 1,
            | 7 => 2,
            | 8 => i128::from(self.rng.next()) & ty.umax(),
            | _ => self.rng.int(41) - 20,
        };
        let value = ty.wrap(value);
        self.constant_value(ty, value)
    }

    /// `iconst` of `value`, which must fit `ty` as a signed number (or be 0
    /// or 1 for `i1`).
    fn constant_value(&mut self, ty: Ty, value: i128) -> Val {
        let value = ty.wrap(value);
        let mut constant = self.def(
            ty,
            Range::new(value, value),
            format_args!("iconst.{ty} {value}"),
        );
        constant.constant = Some(value);
        if let Some(last) = self.scope.last_mut() {
            *last = constant;
        }
        constant
    }

    fn int_type(&mut self) -> Ty {
        self.rng.pick(&[Ty::I8, Ty::I32, Ty::I32, Ty::I64])
    }

    /// Defines a new value of type `ty` by `text` and puts it in scope.
    fn def(&mut self, ty: Ty, range: Range, text: std::fmt::Arguments<'_>) -> Val {
        let id = self.fresh();
        self.line(format_args!("v{id} = {text}"));
        let value = Val {
            id,
            ty,
            range,
            constant: None,
        };
        self.scope.push(value);
        value
    }

    /// Appends an instruction to the current block.
    fn line(&mut self, text: std::fmt::Arguments<'_>) {
        self.cost += self.multiplier;
        self.blocks[self.current].lines.push(text.to_string());
    }

    fn fresh(&mut self) -> u32 {
        let id = self.next_value;
        self.next_value += 1;
        id
    }

    /// A new block with parameters of `types`, returning its number and the
    /// parameters' values.
    fn new_block(&mut self, types: &[Ty]) -> (usize, Vec<u32>) {
        let params: Vec<(u32, Ty)> = types.iter().map(|&ty| (self.fresh(), ty)).collect();
        let ids = params.iter().map(|&(id, _)| id).collect();
        self.blocks.push(BlockText {
            params,
            lines: Vec::new(),
        });
        (self.blocks.len() - 1, ids)
    }

    /// The estimated cost of one call of the function.
    fn total_cost(&self) -> u64 {
        if self.recursive {
            self.cost * RECURSION_FACTOR
        } else {
            self.cost
        }
    }

    /// Writes the function's text.
    fn write(&self, out: &mut String, signature: &Signature, main: bool) {
        let name = if main {
            "main".to_string()
        } else {
            format!("f{}", self.index)
        };
        let params: Vec<&str> = signature.params.iter().map(|ty| ty.name()).collect();
        let linkage = if main { "external" } else { "internal" };
        let _ = writeln!(
            out,
            "function @{name}({}) -> {} {linkage} {{",
            params.join(", "),
            signature.result
        );
        for (slot, size) in self.slots.iter().enumerate() {
            let _ = writeln!(out, "    slot{slot} = stack_slot {size}, align 8");
        }
        for (number, block) in self.blocks.iter().enumerate() {
            let _ = write!(out, "block{number}");
            if !block.params.is_empty() {
                let params: Vec<String> = block
                    .params
                    .iter()
                    .map(|(id, ty)| format!("v{id}: {ty}"))
                    .collect();
                let _ = write!(out, "({})", params.join(", "));
            }
            out.push_str(":\n");
            for line in &block.lines {
                let _ = writeln!(out, "    {line}");
            }
        }
        out.push_str("}\n");
    }
}

/// The integer operations [`FunctionGen::arithmetic`] picks from.
const BINARY: [&str; 13] = [
    "iadd", "isub", "imul", "sdiv", "udiv", "srem", "urem", "and", "or", "xor", "shl", "lshr",
    "ashr",
];

const FLOAT_CONDITIONS: [&str; 14] = [
    "oeq", "one", "olt", "ole", "ogt", "oge", "ord", "ueq", "une", "ult", "ule", "ugt", "uge",
    "uno",
];

/// The floating constants operands start from: signed zeros, fractions,
/// values at and beyond the integer limits, infinities and NaN.
const FLOAT_CONSTANTS: [f64; 14] = [
    0.0,
    -0.0,
    1.0,
    -1.5,
    0.1,
    2.5,
    -7.75,
    255.0,
    1e10,
    2_147_483_648.0,
    -9.3e18,
    f64::INFINITY,
    f64::NEG_INFINITY,
    f64::NAN,
];

const CONDITIONS: [&str; 10] = [
    "eq", "ne", "slt", "sle", "sgt", "sge", "ult", "ule", "ugt", "uge",
];

/// The range of `a op b` for `iadd`, `isub`, `imul`, `and`, `or` and `xor`,
/// and whether the ranges prove `nsw` and `nuw`.
fn binary_range(op: &str, ty: Ty, a: Range, b: Range) -> (Range, bool, bool) {
    let both_nonnegative = a.lo >= 0 && b.lo >= 0;
    let exact = |lo: i128, hi: i128| {
        let fits = ty.contains(lo) && ty.contains(hi);
        let range = if fits {
            Range::new(lo, hi)
        } else {
            Range::full(ty)
        };
        (range, fits)
    };
    match op {
        | "iadd" => {
            let (range, fits) = exact(a.lo + b.lo, a.hi + b.hi);
            (range, fits, fits && both_nonnegative)
        },
        | "isub" => {
            let (range, fits) = exact(a.lo - b.hi, a.hi - b.lo);
            (range, fits, both_nonnegative && a.lo >= b.hi)
        },
        | "imul" => {
            let corners = [a.lo * b.lo, a.lo * b.hi, a.hi * b.lo, a.hi * b.hi];
            let lo = corners.iter().copied().min().unwrap_or(0);
            let hi = corners.iter().copied().max().unwrap_or(0);
            let (range, fits) = exact(lo, hi);
            (range, fits, fits && both_nonnegative)
        },
        | "and" if a.lo >= 0 || b.lo >= 0 => {
            let hi = match (a.lo >= 0, b.lo >= 0) {
                | (true, true) => a.hi.min(b.hi),
                | (true, false) => a.hi,
                | _ => b.hi,
            };
            (Range::new(0, hi), false, false)
        },
        | "or" | "xor" if both_nonnegative => {
            let bits = 128 - a.hi.max(b.hi).leading_zeros();
            (Range::new(0, (1_i128 << bits) - 1), false, false)
        },
        | _ => (Range::full(ty), false, false),
    }
}

/// `block3` or `block3(v1, v2)`.
fn block_call(block: usize, args: &[Val]) -> String {
    if args.is_empty() {
        format!("block{block}")
    } else {
        let args: Vec<String> = args.iter().map(|value| format!("v{}", value.id)).collect();
        format!("block{block}({})", args.join(", "))
    }
}

/// A generated function's signature. A recursive function's first
/// parameter is its depth.
#[derive(Clone, Debug)]
struct Signature {
    params:    Vec<Ty>,
    result:    Ty,
    recursive: bool,
}

impl Signature {
    fn random(rng: &mut Rng) -> Self {
        let recursive = rng.chance(30);
        let mut params = Vec::new();
        if recursive {
            params.push(Ty::I32);
        }
        for _ in 0..=rng.below(3) {
            params.push(rng.pick(&[Ty::I8, Ty::I32, Ty::I64]));
        }
        let result = rng.pick(&[Ty::I8, Ty::I32, Ty::I64]);
        Self {
            params,
            result,
            recursive,
        }
    }
}

/// A block's parameters and instruction lines.
struct BlockText {
    params: Vec<(u32, Ty)>,
    lines:  Vec<String>,
}

/// A value in scope: its number, type and range, and its value if it is a
/// constant.
#[derive(Clone, Copy, Debug)]
struct Val {
    id:       u32,
    ty:       Ty,
    range:    Range,
    constant: Option<i128>,
}

/// A conservative range of a value's signed interpretation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Range {
    lo: i128,
    hi: i128,
}

impl Range {
    /// The range of a pointer, which is never used.
    const FULL_PTR: Self = Self { lo: 0, hi: 0 };

    const fn new(lo: i128, hi: i128) -> Self {
        Self { lo, hi }
    }

    fn full(ty: Ty) -> Self {
        Self::new(ty.min(), ty.max())
    }

    fn union(self, other: Self) -> Self {
        Self::new(self.lo.min(other.lo), self.hi.max(other.hi))
    }
}

/// The value types the generator uses.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ty {
    I1,
    I8,
    I32,
    I64,
    F32,
    F64,
    Ptr,
}

impl Ty {
    const fn name(self) -> &'static str {
        match self {
            | Self::I1 => "i1",
            | Self::I8 => "i8",
            | Self::I32 => "i32",
            | Self::I64 => "i64",
            | Self::F32 => "f32",
            | Self::F64 => "f64",
            | Self::Ptr => "ptr",
        }
    }

    const fn bits(self) -> u32 {
        match self {
            | Self::I1 => 1,
            | Self::I8 => 8,
            | Self::I32 | Self::F32 => 32,
            | Self::I64 | Self::F64 | Self::Ptr => 64,
        }
    }

    const fn is_int(self) -> bool {
        matches!(self, Self::I1 | Self::I8 | Self::I32 | Self::I64)
    }

    fn bytes(self) -> u64 {
        u64::from(self.bits()) / 8
    }

    /// The smallest value; `i1` counts as unsigned, 0 or 1.
    const fn min(self) -> i128 {
        match self {
            | Self::I1 => 0,
            | _ => -(1 << (self.bits() - 1)),
        }
    }

    const fn max(self) -> i128 {
        match self {
            | Self::I1 => 1,
            | _ => (1 << (self.bits() - 1)) - 1,
        }
    }

    const fn umax(self) -> i128 {
        (1 << self.bits()) - 1
    }

    const fn contains(self, value: i128) -> bool {
        self.min() <= value && value <= self.max()
    }

    /// `value` wrapped into the type's signed range (0 or 1 for `i1`).
    const fn wrap(self, value: i128) -> i128 {
        let bits = self.bits();
        let low = value & self.umax();
        if matches!(self, Self::I1) || low < (1 << (bits - 1)) {
            low
        } else {
            low - (1 << bits)
        }
    }
}

impl std::fmt::Display for Ty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// A small deterministic random number generator (`SplitMix64`), so that a
/// seed alone reproduces a program on every platform.
pub(super) struct Rng(u64);

impl Rng {
    pub(super) const fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub(super) fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number below `bound`, which must not be zero.
    pub(super) fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    /// An index below `len`, which must not be zero.
    pub(super) fn index(&mut self, len: usize) -> usize {
        let len = u64::try_from(len).expect("a length fits in 64 bits");
        usize::try_from(self.below(len)).expect("an index below a length fits")
    }

    /// A number in `0..bound`, which must be positive.
    pub(super) fn int(&mut self, bound: i128) -> i128 {
        i128::from(self.below(u64::try_from(bound).expect("a positive bound")))
    }

    /// One of `items`, which must not be empty.
    pub(super) fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.index(items.len())]
    }

    /// True with probability `percent` / 100.
    pub(super) fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

impl<'g> FunctionGen<'g> {
    fn new(
        rng: &'g mut Rng,
        probe: bool,
        index: usize,
        signatures: &'g [Signature],
        costs: &'g [u64],
        signature: &Signature,
        main: bool,
    ) -> Self {
        let mut function = Self {
            rng,
            probe,
            index,
            signatures,
            costs,
            recursive: signature.recursive,
            result: signature.result,
            depth_param: None,
            own_params: signature.params.clone(),
            next_value: 0,
            blocks: Vec::new(),
            current: 0,
            scope: Vec::new(),
            slots: Vec::new(),
            multiplier: 1,
            cost: 0,
            budget: if main { MAIN_BUDGET } else { FUNCTION_BUDGET },
            nesting: 0,
            in_loop: false,
            self_calls: 0,
        };
        let (_, params) = function.new_block(&signature.params);
        for (&id, &ty) in params.iter().zip(&signature.params) {
            function.scope.push(Val {
                id,
                ty,
                range: Range::full(ty),
                constant: None,
            });
        }
        if signature.recursive {
            function.depth_param = function.scope.first().copied();
        }
        function
    }
}
