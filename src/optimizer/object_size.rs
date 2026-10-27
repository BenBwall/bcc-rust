//! Object-size queries answered after inlining, as GCC's object-size pass
//! and LLVM's `llvm.objectsize` answer them. Read [`resolve_object_sizes`],
//! then [`walk`] and [`runtime_plan`].
//!
//! Semantic analysis answers `__builtin_object_size (p, mode)` and
//! `__builtin_dynamic_object_size` itself when the C expression names its
//! object; otherwise, in a function, lowering emits an `object_size` of the
//! pointer value. Fortify wrappers such as glibc's `memcpy` ask about their
//! parameter, whose object only expanding the `always_inline` call reveals.
//! Lowering also passes each pointer argument of an `always_inline` call
//! through a `subobject` that records the closest surrounding subobject the
//! argument names, which the IR's addresses do not record and modes 1 and 3
//! need. The hint closest to a query decides: an argument that is the
//! caller's own parameter offset by pointer arithmetic gets none, so its
//! caller's hint still answers, and any other argument whose subobject
//! semantic analysis cannot size gets one saying it is unknown, so that no
//! hint from further out answers across a member access that narrowed it.
//! A mode 1 or 3 query of anything but such a parameter is answered as
//! unknown in lowering.
//!
//! [`resolve_object_sizes`] runs after the always-inline expansion, for
//! every back end and at every optimization level. It walks each query's
//! pointer back through `ptr_add`, `subobject`, `select` and block
//! parameters to the objects it may point into, and answers from their
//! sizes; then it drops the `subobject` hints and folds the code that
//! tested the answers. A query it cannot answer gets the answer for an
//! unknown object: `(size_t) -1` for modes 0 and 1, and 0 for modes 2 and
//! 3. No answer exceeds the bytes the pointer has left in its object (or,
//! for modes 1 and 3, in its subobject), so a checked call never accepts a
//! write the object cannot hold.
//!
//! For `char a[4]` and a wrapper `w (char *p)` returning
//! `__builtin_object_size (p + 1, 1)`, the expanded call is
//!
//! ```text
//! v0 = stack_addr slot0             ; a, 4 bytes
//! v1 = iconst.i64 4
//! v2 = subobject v0, v1             ; the argument `a` names a 4-byte object
//! v3 = iconst.i64 1
//! v4 = ptr_add inbounds v2, v3
//! v5 = object_size.i64 v4, 1        ; becomes iconst.i64 3
//! ```
//!
//! GNU extension: GCC manual, "Object Size Checking".

use super::{
    MAX_ROUNDS,
    OptimizationReport,
    always_inline,
    dce,
    draft::{
        Constant,
        Draft,
    },
    fold,
    simplify_cfg,
};
use crate::{
    ir::{
        Block,
        Body,
        FuncId,
        Inst,
        InstData,
        InstFlags,
        IntCC,
        Linkage,
        Module,
        ObjectSizeQuery,
        Opcode,
        Type,
        Value,
        ValueDef,
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// The size a `subobject` hint gives when the closest subobject is not
/// known: the hints further out, which a member access may have narrowed,
/// no longer answer for the pointer.
const UNKNOWN_SUBOBJECT: u64 = u64::MAX;

/// How many values one query's walk visits before it gives up and answers
/// as for an unknown object; loops reach it through their block parameters.
const WALK_LIMIT: usize = 64;

/// Answers every `object_size` in `module` and removes every `subobject`.
///
/// First the calls of `always_inline` functions that ask (or call one that
/// asks) are expanded where the LLVM back end would otherwise leave them
/// to LLVM ([`always_inline::expand_object_size_wrappers`]); the other back
/// ends had them expanded already. Each function is then answered in
/// rounds: a query whose walk meets only constant offsets is answered, and
/// `fold` makes the offsets computed from answered queries constant for
/// the next round (a wrapper may offset its pointer by another query's
/// answer). The queries left are answered with runtime arithmetic when
/// dynamic, else as for an unknown object.
pub(crate) fn resolve_object_sizes(module: &mut Module<'_>, library: bool, scratch: &Bump) {
    if !module
        .functions()
        .any(|(_, function)| function.body.as_ref().is_some_and(has_size_facts))
    {
        return;
    }
    always_inline::expand_object_size_wrappers(module, scratch);
    let functions = scratch.alloc_slice_fill_iter(
        module
            .functions()
            .filter(|(_, function)| function.body.as_ref().is_some_and(has_size_facts))
            .map(|(func, _)| func),
    );
    let mut function_scratch = Bump::new();
    for &func in &*functions {
        resolve_function(module, func, library, &function_scratch);
        function_scratch.reset();
    }
}

/// Whether `body` has an `object_size` or a `subobject`.
pub(super) fn has_size_facts(body: &Body<'_>) -> bool {
    body.blocks().any(|block| {
        body.block_insts(block).iter().any(|&inst| {
            matches!(
                body.inst(inst).opcode(),
                Opcode::ObjectSize | Opcode::Subobject
            )
        })
    })
}

/// Whether `body` asks an `object_size`.
pub(super) fn has_queries(body: &Body<'_>) -> bool {
    body.blocks().any(|block| {
        body.block_insts(block)
            .iter()
            .any(|&inst| body.inst(inst).opcode() == Opcode::ObjectSize)
    })
}

/// Answers the queries of one function, drops its hints, and folds what
/// the answers decide. These rewrites are not counted against
/// `--opt-bisect-limit`, as the answers are part of the program's meaning.
fn resolve_function(module: &mut Module<'_>, func: FuncId, library: bool, scratch: &Bump) {
    let Some(body) = module.function_mut(func).body.take() else {
        return;
    };
    let signature = module.function_signature(func);
    let mut draft = Draft::lift(&body, scratch);
    let mut report = OptimizationReport::new(None);
    let objects = Objects { module, library };
    for _ in 0..MAX_ROUNDS {
        if !answer_queries(&mut draft, objects, false)
            || !fold::run(&mut draft, signature, &mut report)
        {
            break;
        }
    }
    _ = answer_queries(&mut draft, objects, true);
    remove_subobjects(&mut draft);
    for _ in 0..MAX_ROUNDS {
        let round = fold::run(&mut draft, signature, &mut report)
            | simplify_cfg::run(&mut draft, &mut report)
            | dce::run(&mut draft, &mut report);
        if !round {
            break;
        }
    }
    draft.sweep();
    draft.lower(module, func);
}

/// Answers the queries [`walk`] can answer with a constant; with `last`,
/// every other query too. Returns whether it answered any.
fn answer_queries(draft: &mut Draft<'_, '_>, objects: Objects<'_, '_>, last: bool) -> bool {
    let mut queries = ArenaVec::new_in(draft.scratch);
    for block in draft.alive_blocks() {
        for &inst in &draft.block(block).insts {
            if !draft.is_removed(inst)
                && draft.inst_constant(inst).is_none()
                && let InstData::ObjectSize { ty, query, ptr } = *draft.inst(inst)
            {
                queries.push((block, inst, ty, query, ptr));
            }
        }
    }
    let mut answered = false;
    for (block, inst, ty, query, ptr) in queries {
        let answer = match walk(draft, objects, ptr, query) {
            | Walk::Known(bytes) => u128::from(bytes),
            | _ if !last => continue,
            | Walk::Variable { branched: false } if query.dynamic() && ty == Type::I64 => {
                if let Some(plan) = runtime_plan(draft, objects, ptr, query) {
                    emit_runtime(draft, block, inst, query, &plan);
                    answered = true;
                    continue;
                }
                query.unknown(ty)
            },
            | Walk::Unknown | Walk::Variable { .. } => query.unknown(ty),
        };
        draft.set_constant(inst, Constant::Int(ty, answer.min(ty.mask())));
        answered = true;
    }
    answered
}

/// What [`walk`] found.
#[expect(
    variant_size_differences,
    reason = "A walk's result is returned once per query, never stored."
)]
enum Walk {
    /// Every object the pointer may point into, and the offset into it,
    /// is known: the combined answer.
    Known(u64),
    /// Some object is not known, or the walk gave up.
    Unknown,
    /// Some offset or size is a runtime value: a dynamic query of a single
    /// path (not `branched` through a `select` or a block parameter) can
    /// compute its answer ([`runtime_plan`]).
    Variable { branched: bool },
}

/// One pointer the walk still has to follow: `value`, which the queried
/// pointer lies `offset` bytes past, and the closest subobject a
/// `subobject` hint met on the way names (its size, and the queried
/// pointer's offset from the hinted one).
#[derive(Clone, Copy, PartialEq, Eq)]
struct Path {
    value:     Value,
    offset:    i128,
    subobject: Option<(u64, i128)>,
}

/// Follows `ptr` back to the objects it may point into, with constant
/// offsets, and combines the bytes left in each: the most for modes 0 and
/// 1, the fewest for 2 and 3, as GCC combines the arms of a conditional for
/// `__builtin_object_size`. A dynamic query whose alternatives differ is
/// unknown: GCC selects among them at run time, which bcc does not, and no
/// single constant is right for every one.
/// An explicit stack replaces recursion through `select` operands and the
/// arguments block parameters receive.
fn walk(
    draft: &Draft<'_, '_>,
    objects: Objects<'_, '_>,
    ptr: Value,
    query: ObjectSizeQuery,
) -> Walk {
    let mut stack = ArenaVec::new_in(draft.scratch);
    stack.push(Path {
        value:     ptr,
        offset:    0,
        subobject: None,
    });
    // The block parameter paths already followed: one reached again (a
    // loop that passes its pointer on unchanged) adds no object.
    let mut visited = ArenaVec::new_in(draft.scratch);
    let mut combined: Option<u64> = None;
    // A dynamic query answers for the object the pointer points into at run
    // time; the most or fewest of different objects' bytes would overstate
    // or understate it, so differing alternatives leave it unknown.
    let mut differ = false;
    let mut unknown = false;
    let mut variable = false;
    let mut branched = false;
    let mut steps = 0;
    while let Some(path) = stack.pop() {
        steps += 1;
        if steps > WALK_LIMIT {
            return Walk::Unknown;
        }
        let value = draft.resolve(path.value);
        let object = match draft.value_def(value) {
            | ValueDef::Param(block, index) if block != Draft::entry() => {
                branched = true;
                let path = Path { value, ..path };
                if visited.contains(&path) {
                    continue;
                }
                visited.push(path);
                let before = stack.len();
                for from in draft.alive_blocks() {
                    for edge in draft.block(from).term.edges() {
                        if edge.target == block
                            && let Some(&arg) = edge.args.get(index as usize)
                        {
                            stack.push(Path { value: arg, ..path });
                        }
                    }
                }
                if stack.len() != before {
                    continue;
                }
                Object::Unknown
            },
            | ValueDef::Param(..) => Object::Unknown,
            | ValueDef::Result(inst) if draft.inst_constant(inst).is_some() => Object::Unknown,
            | ValueDef::Result(inst) => match *draft.inst(inst) {
                | InstData::Binary {
                    opcode: Opcode::PtrAdd,
                    args: [base, delta],
                    ..
                } => {
                    match draft
                        .constant(delta)
                        .and_then(signed)
                        .and_then(|delta| path.offset.checked_add(delta))
                    {
                        | Some(offset) => stack.push(Path {
                            value: base,
                            offset,
                            ..path
                        }),
                        | None => variable = true,
                    }
                    continue;
                },
                | InstData::Binary {
                    opcode: Opcode::Subobject,
                    args: [base, size],
                    ..
                } => {
                    let subobject = path.subobject.or_else(|| match draft.constant(size) {
                        | Some(Constant::Int(_, bytes)) =>
                            u64::try_from(bytes).ok().map(|bytes| (bytes, path.offset)),
                        | _ => None,
                    });
                    stack.push(Path {
                        value: base,
                        subobject,
                        ..path
                    });
                    continue;
                },
                | InstData::Select {
                    args: [_, yes, no], ..
                } => {
                    branched = true;
                    stack.push(Path { value: yes, ..path });
                    stack.push(Path { value: no, ..path });
                    continue;
                },
                | _ => allocation(draft, objects, inst),
            },
        };
        let bytes = match object {
            | Object::Bytes(size) => left(query, size, path),
            | Object::Runtime(_) => {
                variable = true;
                continue;
            },
            | Object::Unknown => subobject_left(query, path),
        };
        match bytes {
            | Some(bytes) => {
                differ |= combined.is_some_and(|other| other != bytes);
                combined = Some(combined.map_or(bytes, |other| {
                    if query.minimum() {
                        other.min(bytes)
                    } else {
                        other.max(bytes)
                    }
                }));
            },
            | None => unknown = true,
        }
    }
    if variable {
        Walk::Variable { branched }
    } else if unknown || (differ && query.dynamic()) {
        Walk::Unknown
    } else {
        combined.map_or(Walk::Unknown, Walk::Known)
    }
}

/// The bytes a path that reached an object of `size` bytes has left, or
/// `None` if the query needs a subobject no hint named.
fn left(query: ObjectSizeQuery, size: u64, path: Path) -> Option<u64> {
    let whole = remaining(size, path.offset);
    if query.subobject() {
        subobject_left(query, path).map(|bytes| bytes.min(whole))
    } else {
        Some(whole)
    }
}

/// The bytes left in the closest subobject a hint on the path named, for
/// modes 1 and 3; the hint closest to the query names it, and one that
/// names none leaves the subobject unknown. A pointer before the subobject
/// leaves none of it (mode 3); mode 1 then asks about the enclosing object,
/// which is not known.
fn subobject_left(query: ObjectSizeQuery, path: Path) -> Option<u64> {
    let (size, offset) = path.subobject.filter(|_| query.subobject())?;
    if size == UNKNOWN_SUBOBJECT || (offset < 0 && !query.minimum()) {
        return None;
    }
    Some(remaining(size, offset))
}

/// The bytes from a signed `offset` to the end of a `size`-byte object; a
/// position before the object or past its end leaves none.
fn remaining(size: u64, offset: i128) -> u64 {
    u64::try_from(offset).map_or(0, |offset| size.saturating_sub(offset))
}

/// A constant `ptr_add` offset as a signed byte count.
fn signed(constant: Constant) -> Option<i128> {
    match constant {
        | Constant::Int(Type::I64, bits) => {
            let bits = u64::try_from(bits).ok()?;
            Some(i128::from(i64::from_ne_bytes(bits.to_ne_bytes())))
        },
        | _ => None,
    }
}

/// What the walks read besides the function: the module's globals and
/// declarations, and whether the C library's allocation functions have
/// their library meaning, as in a hosted implementation; a freestanding
/// one may define `malloc` as anything (C99 §5.1.2.1p1, p. 11; PDF p. 23).
#[derive(Clone, Copy)]
struct Objects<'m, 'ir> {
    module:  &'m Module<'ir>,
    library: bool,
}

/// What a pointer that is not derived from another one points to.
enum Object {
    /// An object of this many bytes.
    Bytes(u64),
    /// An object whose size is this `i64` value, known at run time.
    Runtime(Value),
    Unknown,
}

/// The object `inst` creates or names: a stack slot, a defined global no
/// other unit can replace, a dynamic allocation, or, in a hosted unit, the
/// result of a call of the C library's allocation functions, which GCC
/// knows as built-ins unless `-ffreestanding`.
/// Their identifiers are reserved with external linkage, so a declaration
/// of one names the library's function.
/// C99: §7.20.3, pp. 313-314; PDF pp. 325-326 (`malloc`, `calloc`,
/// `realloc`); §7.1.3p1, p. 166; PDF p. 178 (reserved identifiers).
/// C11: §7.22.3.1, p. 347; PDF p. 365 (`aligned_alloc`).
fn allocation(draft: &Draft<'_, '_>, objects: Objects<'_, '_>, inst: Inst) -> Object {
    let module = objects.module;
    let bytes = |size: Value| match draft.constant(size) {
        | Some(Constant::Int(_, bytes)) =>
            u64::try_from(bytes).map_or(Object::Unknown, Object::Bytes),
        | None if draft.value_type(size) == Type::I64 => Object::Runtime(draft.resolve(size)),
        | Some(Constant::Poison(_)) | None => Object::Unknown,
    };
    match *draft.inst(inst) {
        | InstData::StackAddr { slot } => Object::Bytes(u64::from(draft.stack_slot(slot).size)),
        | InstData::GlobalAddr { global } => {
            let global = module.global(global);
            if global.init.is_some()
                && global.attributes.alias.is_none()
                && global.linkage != Linkage::Weak
            {
                Object::Bytes(global.size)
            } else {
                Object::Unknown
            }
        },
        | InstData::DynAlloca { size, .. } => bytes(size),
        | InstData::Call { func, args } => {
            let function = module.function(func);
            if !objects.library || function.body.is_some() || function.linkage != Linkage::External
            {
                return Object::Unknown;
            }
            let args = draft.call_operands(inst, args);
            match (function.name, args) {
                | ("malloc", &[size]) | ("realloc" | "aligned_alloc", &[_, size]) => bytes(size),
                | ("calloc", &[count, size]) => match (draft.constant(count), draft.constant(size))
                {
                    | (Some(Constant::Int(_, count)), Some(Constant::Int(_, size))) => count
                        .checked_mul(size)
                        .and_then(|bytes| u64::try_from(bytes).ok())
                        .map_or(Object::Unknown, Object::Bytes),
                    | _ => Object::Unknown,
                },
                | _ => Object::Unknown,
            }
        },
        | _ => Object::Unknown,
    }
}

/// How a dynamic query of a single path computes its answer at run time:
/// the object's size, the constant and runtime parts of the queried
/// pointer's offset into it, and the hinted subobject with the queried
/// pointer's offset into that (its constant part, and how many of the
/// runtime `terms` lie after the hint).
struct RuntimePlan<'s> {
    size:      Option<Size>,
    constant:  i128,
    terms:     ArenaVec<'s, Value>,
    subobject: Option<(u64, i128, usize)>,
}

#[derive(Clone, Copy)]
enum Size {
    Bytes(u64),
    Runtime(Value),
}

/// Follows `ptr` back along its only path, as [`walk`] does, collecting
/// runtime offsets instead of giving up on them. `None` when the query
/// cannot be answered from what the path shows.
fn runtime_plan<'s>(
    draft: &Draft<'_, 's>,
    objects: Objects<'_, '_>,
    ptr: Value,
    query: ObjectSizeQuery,
) -> Option<RuntimePlan<'s>> {
    let mut plan = RuntimePlan {
        size:      None,
        constant:  0,
        terms:     ArenaVec::new_in(draft.scratch),
        subobject: None,
    };
    let mut value = ptr;
    for _ in 0..WALK_LIMIT {
        value = draft.resolve(value);
        let ValueDef::Result(inst) = draft.value_def(value) else {
            break;
        };
        if draft.inst_constant(inst).is_some() {
            break;
        }
        match *draft.inst(inst) {
            | InstData::Binary {
                opcode: Opcode::PtrAdd,
                args: [base, delta],
                ..
            } => {
                match draft.constant(delta).map(signed) {
                    | Some(Some(delta)) => plan.constant = plan.constant.checked_add(delta)?,
                    | Some(None) => return None,
                    | None => plan.terms.push(draft.resolve(delta)),
                }
                value = base;
            },
            | InstData::Binary {
                opcode: Opcode::Subobject,
                args: [base, size],
                ..
            } => {
                if plan.subobject.is_none()
                    && let Some(Constant::Int(_, bytes)) = draft.constant(size)
                {
                    plan.subobject =
                        Some((u64::try_from(bytes).ok()?, plan.constant, plan.terms.len()));
                }
                value = base;
            },
            | _ => {
                plan.size = match allocation(draft, objects, inst) {
                    | Object::Bytes(bytes) => Some(Size::Bytes(bytes)),
                    | Object::Runtime(size) => Some(Size::Runtime(size)),
                    | Object::Unknown => None,
                };
                break;
            },
        }
    }
    let i64_range = i128::from(i64::MIN)..=i128::from(i64::MAX);
    let usable = if query.subobject() {
        plan.subobject
            .is_some_and(|(size, ..)| size != UNKNOWN_SUBOBJECT)
    } else {
        plan.size.is_some()
    };
    (usable && i64_range.contains(&plan.constant)).then_some(plan)
}

/// Replaces the query `inst` in `block` by the arithmetic `plan` describes:
/// the bytes from the offset to the end of the object, none if the offset
/// lies past its end or (for the whole object, or mode 3) before its start,
/// and the unknown answer for a mode 1 offset before the subobject's start,
/// as [`left`] answers constant offsets.
fn emit_runtime(
    draft: &mut Draft<'_, '_>,
    block: Block,
    inst: Inst,
    query: ObjectSizeQuery,
    plan: &RuntimePlan<'_>,
) {
    let position = draft
        .block(block)
        .insts
        .iter()
        .position(|&other| other == inst)
        .expect("a query is in its block");
    let mut at = Emitter { block, position };
    let zero = at.constant(draft, 0);
    let whole = plan.size.map(|size| {
        let size = match size {
            | Size::Bytes(bytes) => at.constant(draft, i128::from(bytes)),
            | Size::Runtime(size) => size,
        };
        let offset = at.offset(draft, plan.constant, &plan.terms);
        at.left(draft, size, offset, zero)
    });
    let answer = match plan.subobject.filter(|_| query.subobject()) {
        | Some((size, before, count)) => {
            let size = at.constant(draft, i128::from(size));
            let offset = at.offset(draft, before, &plan.terms[..count]);
            let sub = at.left(draft, size, offset, zero);
            let least = match whole {
                | Some(whole) => {
                    let smaller = at.compare(draft, IntCC::Ult, sub, whole);
                    at.select(draft, smaller, sub, whole)
                },
                | None => sub,
            };
            if query.minimum() {
                least
            } else {
                // Before the subobject's start the enclosing object is
                // asked about, which is not known.
                let unknown = at.constant(draft, i128::from(u64::MAX));
                let before = at.compare(draft, IntCC::Slt, offset, zero);
                at.select(draft, before, unknown, least)
            }
        },
        | None => whole.expect("a whole-object plan has a size"),
    };
    let result = draft.inst_result(inst).expect("a query has a result");
    draft.replace(result, answer);
    draft.remove_inst(inst);
}

/// Inserts `i64` arithmetic before a query, in order.
struct Emitter {
    block:    Block,
    position: usize,
}

impl Emitter {
    fn emit(&mut self, draft: &mut Draft<'_, '_>, data: InstData, ty: Type) -> Value {
        let value = draft
            .add_inst(self.block, self.position, data, Some(ty))
            .expect("arithmetic has a result");
        self.position += 1;
        value
    }

    /// An `i64` constant; constants go to the entry block, which does not
    /// move the insertion point unless it is that block.
    fn constant(&mut self, draft: &mut Draft<'_, '_>, bits: i128) -> Value {
        let before = draft.block(self.block).insts.len();
        let value = draft.add_constant(Constant::Int(
            Type::I64,
            bits.cast_unsigned() & Type::I64.mask(),
        ));
        self.position += draft.block(self.block).insts.len() - before;
        value
    }

    /// `constant` plus every term.
    fn offset(&mut self, draft: &mut Draft<'_, '_>, constant: i128, terms: &[Value]) -> Value {
        let mut offset = self.constant(draft, constant);
        for &term in terms {
            offset = self.emit(
                draft,
                InstData::Binary {
                    opcode: Opcode::Iadd,
                    ty:     Type::I64,
                    flags:  InstFlags::empty(),
                    args:   [offset, term],
                },
                Type::I64,
            );
        }
        offset
    }

    fn compare(&mut self, draft: &mut Draft<'_, '_>, cond: IntCC, a: Value, b: Value) -> Value {
        self.emit(
            draft,
            InstData::IntCompare {
                cond,
                ty: Type::I64,
                args: [a, b],
            },
            Type::I1,
        )
    }

    fn select(&mut self, draft: &mut Draft<'_, '_>, cond: Value, a: Value, b: Value) -> Value {
        self.emit(
            draft,
            InstData::Select {
                ty:   Type::I64,
                args: [cond, a, b],
            },
            Type::I64,
        )
    }

    /// `size - offset` when `offset <= size` (unsigned, so a negative
    /// offset exceeds every size), else `outside`.
    fn left(
        &mut self,
        draft: &mut Draft<'_, '_>,
        size: Value,
        offset: Value,
        outside: Value,
    ) -> Value {
        let rest = self.emit(
            draft,
            InstData::Binary {
                opcode: Opcode::Isub,
                ty:     Type::I64,
                flags:  InstFlags::empty(),
                args:   [size, offset],
            },
            Type::I64,
        );
        let within = self.compare(draft, IntCC::Ule, offset, size);
        self.select(draft, within, rest, outside)
    }
}

/// Makes the uses of every `subobject` use its pointer, and removes it.
fn remove_subobjects(draft: &mut Draft<'_, '_>) {
    let mut hints = ArenaVec::new_in(draft.scratch);
    for block in draft.alive_blocks() {
        for &inst in &draft.block(block).insts {
            if !draft.is_removed(inst)
                && let InstData::Binary {
                    opcode: Opcode::Subobject,
                    args: [base, _],
                    ..
                } = *draft.inst(inst)
            {
                hints.push((inst, base));
            }
        }
    }
    for (inst, base) in hints {
        let result = draft.inst_result(inst).expect("a hint has a result");
        draft.replace(result, base);
        draft.remove_inst(inst);
    }
}
