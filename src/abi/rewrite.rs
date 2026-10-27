//! Rebuilding a function body for the plans of its own signature and of the
//! calls it makes.
//!
//! The new body keeps the old numbering: every old value, block and stack
//! slot is reserved or created first, with the same number, so an
//! instruction that ABI lowering leaves alone is copied unchanged, even
//! when its operands are defined later in layout order. New values, blocks
//! and slots follow the old ones.
//!
//! - The entry block receives the lowered parameters. An aggregate that arrives
//!   in registers is stored into a fresh slot, whose address defines the old
//!   pointer parameter; an ignored one gets an empty slot.
//! - A call passes each aggregate in registers as the scalars loaded from its
//!   copy, drops ignored ones, and makes sure that a `byval` copy and a
//!   register result's return area are as aligned and as large as the target
//!   needs: a slot the call already passes is enlarged in place, anything else
//!   is copied through a temporary.
//! - A `va_arg` becomes the target's walk over its `va_list`, then a load.

use super::{
    AbiErrorKind,
    plan::{
        ArgPlan,
        Convention,
        Parts,
        ResultPlan,
        SignaturePlan,
        stack_slot_size,
    },
    sysv::{
        self,
        va_list,
    },
    win64,
};
use crate::{
    ir::{
        AggregateLayout,
        Align,
        Block,
        Body,
        Entity,
        FuncId,
        FunctionBuilder,
        InstData,
        InstFlags,
        IntCC,
        MemFlags,
        Module,
        Opcode,
        SigId,
        StackSlotData,
        Type,
        Value,
        ValueDef,
    },
    target::Target,
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// Checks that the target's `va_arg` expansion reads `ty`.
pub(super) fn check_va_arg(target: Target, ty: Type) -> Result<(), AbiErrorKind> {
    match Convention::of(target) {
        | Convention::SystemV => sysv::va_arg_class(ty)
            .map(|_| ())
            .ok_or(AbiErrorKind::VaArgType(ty)),
        | Convention::Win64 => win64::check_va_arg(ty),
    }
}

/// Rebuilds the body of `func`, if it has one. `plans` holds the plan of
/// every pre-ABI signature, and `old_signatures` each function's pre-ABI
/// signature, which its callers were planned for.
pub(super) fn rewrite_body(
    module: &mut Module<'_>,
    func: FuncId,
    target: Target,
    plans: &[SignaturePlan<'_>],
    lowered_signatures: &[SigId],
    old_signatures: &[SigId],
    scratch: &Bump,
) {
    let Some(old) = module.function_mut(func).body.take() else {
        return;
    };
    let plan = plans[old_signatures[func.index()].index()];
    let slots = slot_needs(&old, plans, old_signatures, scratch);
    let mut rewriter = Rewriter {
        builder: FunctionBuilder::new(module, func),
        old: &old,
        plans,
        lowered_signatures,
        old_signatures,
        convention: Convention::of(target),
        scratch,
        wide_result: None,
    };
    rewriter.run(plan, &slots);
    rewriter.builder.finish();
    if cfg!(debug_assertions) {
        let errors = crate::ir::verify_function(module, func, crate::ir::Profile::PostAbi, scratch);
        assert!(
            errors.is_empty(),
            "ABI lowering produced invalid IR:\n{}\n{}",
            std::fmt::from_fn(|f| errors.iter().try_for_each(|error| writeln!(f, "{error}"))),
            module.display_function(func),
        );
    }
}

/// The state of rebuilding one body.
struct Rewriter<'a, 'm, 'ir, 's> {
    builder:            FunctionBuilder<'m, 'ir>,
    old:                &'a Body<'ir>,
    plans:              &'a [SignaturePlan<'s>],
    lowered_signatures: &'a [SigId],
    old_signatures:     &'a [SigId],
    convention:         Convention,
    scratch:            &'a Bump,
    /// A scalar i128 result's post-ABI register return area.
    wide_result:        Option<Value>,
}

/// What the entry block does for one parameter before the old code runs.
#[derive(Clone, Copy)]
enum Prologue<'s> {
    /// Load the scalar from Win64's indirect argument into the old SSA value.
    LoadI128 { param: Value, pointer: Value },
    /// Store the parts that arrived in registers into a fresh slot whose
    /// address is the old parameter.
    Rebuild {
        param:  Value,
        layout: AggregateLayout<'s>,
        parts:  Parts,
        first:  usize,
    },
    /// Give an aggregate that is not passed an empty slot.
    Empty {
        param:  Value,
        layout: AggregateLayout<'s>,
    },
}

impl<'a> Rewriter<'a, '_, '_, '_> {
    /// Rebuilds the body: the same values, slots and blocks first, then the
    /// entry block's parameters and prologue, then every instruction.
    fn run(&mut self, plan: SignaturePlan<'_>, slots: &[StackSlotData]) {
        let old = self.old;
        for _ in 0..old.value_count() {
            _ = self.builder.reserve_value();
        }
        for &data in slots {
            _ = self.builder.create_stack_slot(data.size, data.align);
        }
        for _ in old.blocks() {
            _ = self.builder.create_block();
        }
        // Blocks keep their numbers, and expansions split a block only after
        // its start, so each label still names its block.
        for (_, block) in old.labels() {
            _ = self.builder.create_label(block);
        }
        for block in old.blocks().skip(1) {
            for &param in old.block_params(block) {
                self.builder
                    .define_block_param(block, param, old.value_type(param));
            }
        }
        let Some(entry) = old.entry_block() else {
            return;
        };
        self.builder.switch_to_block(entry);
        let prologue = self.entry_params(plan, old.block_params(entry));
        for &item in &prologue {
            self.prologue(item);
        }
        for block in old.blocks() {
            self.builder.switch_to_block(block);
            for &inst in old.block_insts(block) {
                self.inst(inst);
            }
        }
    }

    /// Defines the entry block's lowered parameters and returns what the
    /// prologue must do for the aggregates among the old ones.
    fn entry_params<'p>(
        &mut self,
        plan: SignaturePlan<'p>,
        params: &[Value],
    ) -> ArenaVec<'a, Prologue<'p>> {
        let entry = Block::new(0);
        let mut prologue = ArenaVec::new_in(self.scratch);
        let first = plan.first_arg();
        match plan.result {
            | ResultPlan::Scalar => {},
            | ResultPlan::VectorI128 => {
                self.wide_result = Some(self.builder.append_block_param(entry, Type::Ptr));
            },
            | ResultPlan::Memory(_) | ResultPlan::Registers { .. } =>
                self.builder.define_block_param(entry, params[0], Type::Ptr),
            | ResultPlan::Ignore(layout) => prologue.push(Prologue::Empty {
                param: params[0],
                layout,
            }),
        }
        for (&param, &arg) in params[first..].iter().zip(plan.args) {
            match arg {
                | ArgPlan::Keep | ArgPlan::Stack { .. } =>
                    self.builder
                        .define_block_param(entry, param, self.old.value_type(param)),
                | ArgPlan::IndirectI128 => {
                    let pointer = self.builder.append_block_param(entry, Type::Ptr);
                    prologue.push(Prologue::LoadI128 { param, pointer });
                },
                | ArgPlan::Registers { layout, parts } => {
                    let first = self.builder.body().value_count();
                    for &ty in parts.types() {
                        _ = self.builder.append_block_param(entry, ty);
                    }
                    prologue.push(Prologue::Rebuild {
                        param,
                        layout,
                        parts,
                        first,
                    });
                },
                | ArgPlan::Ignore(layout) => prologue.push(Prologue::Empty { param, layout }),
            }
        }
        prologue
    }

    /// Rebuilds one aggregate parameter in a fresh slot. The slot covers
    /// every register's bytes, which may run past the aggregate.
    fn prologue(&mut self, item: Prologue<'_>) {
        match item {
            | Prologue::LoadI128 { param, pointer } => {
                _ = self.builder.insert_with_result(
                    InstData::Load {
                        ty:    Type::I128,
                        flags: InstFlags::empty(),
                        align: natural_align(Type::I128),
                        tag:   None,
                        addr:  pointer,
                    },
                    Some(param),
                );
            },
            | Prologue::Rebuild {
                param,
                layout,
                parts,
                first,
            } => {
                let size = layout.size.max(parts.bytes());
                let align = parts
                    .types()
                    .iter()
                    .fold(max_align(layout.align, 8), |align, &ty| {
                        max_align(align, natural_align(ty).bytes())
                    });
                let address = self.fresh_slot(param, size, align);
                for (index, &ty) in parts.types().iter().enumerate() {
                    let value = Value::new(first + index);
                    let at = self.offset(address, 8 * index as u64);
                    self.builder
                        .store(value, at, MemFlags::aligned(natural_align(ty)));
                }
            },
            | Prologue::Empty { param, layout } => {
                _ = self.fresh_slot(param, layout.size.max(1), layout.align);
            },
        }
    }

    /// A fresh slot whose address defines `param`.
    fn fresh_slot(&mut self, param: Value, size: u64, align: Align) -> Value {
        let slot = self.builder.create_stack_slot(slot_size(size), align);
        _ = self
            .builder
            .insert_with_result(InstData::StackAddr { slot }, Some(param));
        param
    }

    /// Copies one instruction, rewriting calls and `va_arg`.
    fn inst(&mut self, inst: crate::ir::Inst) {
        let old = self.old;
        let data = *old.inst(inst);
        let result = old.inst_result(inst);
        match data {
            | InstData::VaArgAggregate { sig, list } => {
                let layout = self
                    .builder
                    .signature(sig)
                    .param_abi(0)
                    .and_then(crate::ir::ParamAbi::layout)
                    .expect("verified aggregate descriptor");
                let address = self.aggregate_va_arg(layout, list);
                let zero = self.builder.iconst(Type::I64, 0);
                _ = self.builder.insert_with_result(
                    InstData::Binary {
                        opcode: Opcode::PtrAdd,
                        ty:     Type::Ptr,
                        flags:  InstFlags::empty(),
                        args:   [address, zero],
                    },
                    result,
                );
            },
            | InstData::Return { value: Some(value) } if self.wide_result.is_some() => {
                self.builder.store(
                    value,
                    self.wide_result.expect("the wide result has an area"),
                    MemFlags::aligned(natural_align(Type::I128)),
                );
                _ = self.builder.insert(InstData::Return { value: None });
            },
            | InstData::Atomic { args, .. } => {
                let values = self.builder.value_list(old.value_list(args));
                let mut data = data;
                if let InstData::Atomic { args, .. } = &mut data {
                    *args = values;
                }
                _ = self.builder.insert_with_result(data, result);
            },
            | InstData::InlineAsm { asm, args } => {
                let args = self.builder.value_list(old.value_list(args));
                _ = self.builder.insert(InstData::InlineAsm { asm, args });
            },
            | InstData::Call { func, args } => {
                let sig = self.old_signatures[func.index()];
                self.call(
                    sig,
                    Callee::Direct(func),
                    old.value_list(args),
                    result,
                    old.call_diagnostic(inst),
                );
            },
            | InstData::CallIndirect { sig, args } => {
                let values = old.value_list(args);
                self.call(
                    sig,
                    Callee::Indirect(values[0]),
                    &values[1..],
                    result,
                    old.call_diagnostic(inst),
                );
            },
            | InstData::Unary {
                opcode: Opcode::VaArg,
                ty,
                arg,
            } => {
                let address = match self.convention {
                    | Convention::SystemV => self.sysv_va_arg(ty, arg),
                    | Convention::Win64 => self.win64_va_arg(arg),
                };
                _ = self.builder.insert_with_result(
                    InstData::Load {
                        ty,
                        flags: InstFlags::empty(),
                        align: at_most(natural_align(ty), 8),
                        tag: None,
                        addr: address,
                    },
                    result,
                );
            },
            | InstData::WideConst {
                opcode,
                ty,
                constant,
            } => {
                let constant = self.builder.wide_constant(old.constant(constant));
                _ = self.builder.insert_with_result(
                    InstData::WideConst {
                        opcode,
                        ty,
                        constant,
                    },
                    result,
                );
            },
            | InstData::Jump { dest } => {
                let (block, args) = old.block_call(dest);
                self.builder.jump(block, args);
            },
            | InstData::Brif {
                cond,
                dests: [then, otherwise],
            } => self
                .builder
                .brif(cond, old.block_call(then), old.block_call(otherwise)),
            | InstData::Switch {
                value,
                default,
                cases,
            } => {
                let (block, args) = old.block_call(default);
                let default = self.builder.block_call(block, args);
                let mut list =
                    ArenaVec::with_capacity_in(old.switch_cases(cases).len(), self.scratch);
                for (bits, dest) in old.switch_cases(cases) {
                    let constant = self.builder.wide_constant(bits);
                    let (block, args) = old.block_call(dest);
                    list.push((constant, self.builder.block_call(block, args)));
                }
                let cases = self.builder.case_list(&list);
                _ = self.builder.insert(InstData::Switch {
                    value,
                    default,
                    cases,
                });
            },
            | InstData::IndirectBr { addr, dests } => {
                let mut list =
                    ArenaVec::with_capacity_in(old.block_list(dests).len(), self.scratch);
                for dest in old.block_list(dests) {
                    list.push(old.block_call(dest));
                }
                self.builder.indirect_br(addr, &list);
            },
            | _ => _ = self.builder.insert_with_result(data, result),
        }
    }

    /// Copies a call to a function of the pre-ABI signature `sig`, passing
    /// its arguments as the plan says.
    fn call(
        &mut self,
        sig: SigId,
        callee: Callee,
        args: &[Value],
        result: Option<Value>,
        diagnostic: Option<u32>,
    ) {
        let plan = self.plans[sig.index()];
        let mut lowered = ArenaVec::with_capacity_in(args.len() + 2, self.scratch);
        if let Callee::Indirect(pointer) = callee {
            lowered.push(pointer);
        }
        // A return area to copy into the caller's object after the call.
        let mut copy_back = None;
        let mut wide_result = None;
        match plan.result {
            | ResultPlan::Scalar | ResultPlan::Ignore(_) => {},
            | ResultPlan::VectorI128 => {
                let area = self.temporary(16, natural_align(Type::I128));
                lowered.push(area);
                wide_result = Some(area);
            },
            | ResultPlan::Memory(_) => lowered.push(args[0]),
            | ResultPlan::Registers { layout, registers } =>
                if self.is_slot_address(args[0]) {
                    lowered.push(args[0]);
                } else {
                    let size = layout.size.max(registers.bytes());
                    let area = self.temporary(size, max_align(layout.align, 8));
                    lowered.push(area);
                    copy_back = Some((area, args[0], layout));
                },
        }
        let first = plan.first_arg();
        for (&value, &arg) in args[first..].iter().zip(plan.args) {
            match arg {
                | ArgPlan::Keep => lowered.push(value),
                | ArgPlan::IndirectI128 => lowered.push(self.indirect_i128(value)),
                | ArgPlan::Registers { layout, parts } =>
                    for (index, &ty) in parts.types().iter().enumerate() {
                        let part = self.pack(value, layout, index, ty);
                        lowered.push(part);
                    },
                | ArgPlan::Stack { layout, align } =>
                    if self.is_slot_address(value) {
                        lowered.push(value);
                    } else {
                        let copy = self.temporary(layout.size, align);
                        self.copy(copy, value, layout.size, layout.align);
                        lowered.push(copy);
                    },
                | ArgPlan::Ignore(_) => {},
            }
        }
        for &value in &args[first + plan.args.len()..] {
            let value = if self.convention == Convention::Win64
                && self.old.value_type(value) == Type::I128
            {
                self.indirect_i128(value)
            } else {
                value
            };
            lowered.push(value);
        }
        let mut lowered_sig = self.lowered_signatures[sig.index()];
        let original = self.builder.signature(sig);
        let projected = self.builder.signature(lowered_sig);
        if original.variadic_noundef
            && original.variadic_fixed.is_some()
            && args.len() > original.params.len()
        {
            // A textual call can append scalars beyond its described tail.
            // Preserve their inherited promises when projection materializes
            // the known tail's attributes, without marking aggregate parts.
            let mut params = ArenaVec::new_in(self.scratch);
            params.extend_from_slice(projected.params);
            let mut noundef = ArenaVec::new_in(self.scratch);
            noundef.extend_from_slice(projected.noundef);
            for &value in &args[original.params.len()..] {
                noundef.push(u32::try_from(params.len()).expect("IR parameter index fits u32"));
                let ty = self.old.value_type(value);
                params.push(
                    if self.convention == Convention::Win64 && ty == Type::I128 {
                        Type::Ptr
                    } else {
                        ty
                    },
                );
            }
            lowered_sig = self.builder.intern(crate::ir::Signature {
                params: &params,
                noundef: &noundef,
                variadic_fixed: Some(
                    u32::try_from(projected.fixed_count()).expect("planned count"),
                ),
                ..projected
            });
        }
        let data = match callee {
            | Callee::Direct(func) => InstData::Call {
                func,
                args: self.builder.value_list(&lowered),
            },
            | Callee::Indirect(_) => InstData::CallIndirect {
                sig:  lowered_sig,
                args: self.builder.value_list(&lowered),
            },
        };
        let call = self
            .builder
            .insert_with_result(data, if wide_result.is_some() { None } else { result });
        if let Some(id) = diagnostic {
            self.builder.set_call_diagnostic(call, id);
        }
        if let (Some(area), Some(result)) = (wide_result, result) {
            _ = self.builder.insert_with_result(
                InstData::Load {
                    ty:    Type::I128,
                    flags: InstFlags::empty(),
                    align: natural_align(Type::I128),
                    tag:   None,
                    addr:  area,
                },
                Some(result),
            );
        }
        if let Some((area, object, layout)) = copy_back {
            self.copy(object, area, layout.size, layout.align);
        }
    }

    /// Stores a Win64 scalar argument in a fresh 16-aligned caller copy.
    /// C99: §6.5.2.2 paragraph 4, p. 71; PDF p. 83; `__int128` is an extension.
    fn indirect_i128(&mut self, value: Value) -> Value {
        let copy = self.temporary(16, natural_align(Type::I128));
        self.builder
            .store(value, copy, MemFlags::aligned(natural_align(Type::I128)));
        copy
    }

    /// The register part `index` of type `ty` of the aggregate at `base`:
    /// the lone scalar that fills it, loaded as is, or else its scalars
    /// loaded one by one, frozen so that one member never written does not
    /// make the others poison, and combined at their offsets. Overlapping
    /// union members are loaded a byte at a time, since which of them was
    /// written last is not known. Padding bytes are zero. SSE/SSEUP parts
    /// cover all sixteen bytes of a single vector register.
    /// C99: §6.2.6.1 paragraph 4, p. 37; PDF p. 49 (object representation);
    /// `__float128` is an extension, classified by the System V psABI.
    fn pack(&mut self, base: Value, layout: AggregateLayout<'_>, index: usize, ty: Type) -> Value {
        let start = 8 * index as u64;
        let eightbyte = self.offset(base, start);
        let bytes = u64::from(ty.bytes()).max(8);
        let end = start + bytes;
        let word = if bytes == 16 { Type::I128 } else { Type::I64 };
        let mut pieces = ArenaVec::new_in(self.scratch);
        for scalar in layout.scalars {
            let (from, to) = (
                u64::from(scalar.offset),
                u64::from(scalar.offset) + u64::from(scalar.ty.bytes()),
            );
            if to > start && from < end {
                let clipped = from < start || to > end;
                pieces.push((
                    from.max(start) - start,
                    to.min(end) - start,
                    scalar.ty,
                    clipped,
                ));
            }
        }
        if let &[(0, _, scalar, false)] = pieces.as_slice()
            && scalar == ty
        {
            return self.builder.load(
                ty,
                eightbyte,
                MemFlags::aligned(align_at(layout.align, start, u64::from(ty.bytes()))),
            );
        }
        pieces.sort_unstable_by_key(|&(from, to, ..)| (from, to));
        // Merge overlapping pieces into runs; a run of one piece is loaded
        // whole and any other a byte at a time.
        let mut runs: ArenaVec<'_, (u64, u64, bool)> = ArenaVec::new_in(self.scratch);
        for &(from, to, ..) in &pieces {
            match runs.last_mut() {
                | Some(run) if from < run.1 => {
                    run.1 = run.1.max(to);
                    run.2 = false;
                },
                | _ => runs.push((from, to, true)),
            }
        }
        let mut combined = None;
        for (from, to, single) in runs {
            let length = to - from;
            let (step, count) = if single && length.is_power_of_two() {
                (length, 1)
            } else {
                (1, length)
            };
            for chunk in 0..count {
                let offset = from + chunk * step;
                let at = self.offset(eightbyte, offset);
                let chunk_type = int_type(step);
                let align = align_at(layout.align, start + offset, step);
                let loaded = self.builder.load(chunk_type, at, MemFlags::aligned(align));
                let frozen = if layout.flags.contains(crate::ir::AggregateFlags::COMPLEX) {
                    loaded
                } else {
                    self.builder.unary(Opcode::Freeze, loaded)
                };
                let wide = if chunk_type == word {
                    frozen
                } else {
                    self.builder.convert(Opcode::Zext, word, frozen)
                };
                let shifted = if offset == 0 {
                    wide
                } else {
                    let amount = self.builder.iconst(word, i128::from(8 * offset));
                    self.builder
                        .binary(Opcode::Shl, InstFlags::empty(), wide, amount)
                };
                combined = Some(match combined {
                    | None => shifted,
                    | Some(sum) =>
                        self.builder
                            .binary(Opcode::Or, InstFlags::empty(), sum, shifted),
                });
            }
        }
        let combined = combined.unwrap_or_else(|| self.builder.iconst(word, 0));
        match ty {
            | Type::I64 | Type::I128 => combined,
            | Type::F128 => self.builder.convert(Opcode::Bitcast, Type::F128, combined),
            | Type::F64 => self.builder.convert(Opcode::Bitcast, Type::F64, combined),
            | Type::F32 => {
                let bits = self.builder.convert(Opcode::Trunc, Type::I32, combined);
                self.builder.convert(Opcode::Bitcast, Type::F32, bits)
            },
            | _ => self.builder.convert(Opcode::Trunc, ty, combined),
        }
    }

    /// Whether `value` is a stack slot's address, which [`slot_needs`] has
    /// enlarged and aligned for the calls that pass it.
    fn is_slot_address(&self, value: Value) -> bool {
        slot_of(self.old, value).is_some()
    }

    /// The address of a fresh slot for the call being rewritten.
    fn temporary(&mut self, size: u64, align: Align) -> Value {
        let slot = self.builder.create_stack_slot(slot_size(size), align);
        self.builder.stack_addr(slot)
    }

    fn copy(&mut self, to: Value, from: Value, size: u64, align: Align) {
        let size = self.builder.iconst(Type::I64, i128::from(size));
        self.builder
            .copy(InstFlags::empty(), [to, from, size], align);
    }

    /// `base` plus `offset` bytes, within the object `base` addresses.
    fn offset(&mut self, base: Value, offset: u64) -> Value {
        if offset == 0 {
            return base;
        }
        let offset = self.builder.iconst(Type::I64, i128::from(offset));
        self.builder.ptr_add(InstFlags::INBOUNDS, base, offset)
    }

    /// The address of the next argument of type `ty` in the System V
    /// `va_list` at `list`, which this advances: from the register save area
    /// while the registers the type needs remain, else from the overflow
    /// area on the stack.
    /// psABI §3.5.7, "The `va_arg` Macro", steps 1-11.
    fn sysv_va_arg(&mut self, ty: Type, list: Value) -> Value {
        let class = sysv::va_arg_class(ty).expect("lower_abi checked each va_arg type");
        let (field, limit, step) = if class.gp > 0 {
            (
                va_list::GP_OFFSET,
                va_list::GP_AREA_SIZE - 8 * u64::from(class.gp),
                8 * u64::from(class.gp),
            )
        } else if class.sse {
            (va_list::FP_OFFSET, va_list::SAVE_AREA_SIZE - 16, 16)
        } else {
            return self.overflow_arg(list, class);
        };
        let field = self.offset(list, field);
        let offset = self.builder.load(
            Type::I32,
            field,
            MemFlags::aligned(natural_align(Type::I32)),
        );
        let limit = self.builder.iconst(Type::I32, i128::from(limit));
        let fits = self.builder.icmp(IntCC::Ule, offset, limit);
        let in_registers = self.builder.create_block();
        let on_stack = self.builder.create_block();
        let join = self.builder.create_block();
        let address = self.builder.append_block_param(join, Type::Ptr);
        self.builder
            .brif(fits, (in_registers, &[]), (on_stack, &[]));
        self.builder.switch_to_block(in_registers);
        let save_field = self.offset(list, va_list::REG_SAVE_AREA);
        let save = self.builder.load(
            Type::Ptr,
            save_field,
            MemFlags::aligned(natural_align(Type::Ptr)),
        );
        let wide = self.builder.convert(Opcode::Zext, Type::I64, offset);
        let in_save = self.builder.ptr_add(InstFlags::empty(), save, wide);
        let step = self.builder.iconst(Type::I32, i128::from(step));
        let next = self
            .builder
            .binary(Opcode::Iadd, InstFlags::empty(), offset, step);
        self.builder
            .store(next, field, MemFlags::aligned(natural_align(Type::I32)));
        self.builder.jump(join, &[in_save]);
        self.builder.switch_to_block(on_stack);
        let on_stack = self.overflow_arg(list, class);
        self.builder.jump(join, &[on_stack]);
        self.builder.switch_to_block(join);
        address
    }

    /// The address of the next argument in the System V overflow area,
    /// aligned for the type, which this advances past it.
    /// psABI §3.5.7, "The `va_arg` Macro", steps 7-11.
    fn overflow_arg(&mut self, list: Value, class: sysv::VaArgClass) -> Value {
        let field = self.offset(list, va_list::OVERFLOW_ARG_AREA);
        let pointer_align = MemFlags::aligned(natural_align(Type::Ptr));
        let mut area = self.builder.load(Type::Ptr, field, pointer_align);
        if class.overflow_align > 8 {
            // Round the address up: area + ((-address) mod align).
            let address = self.builder.convert(Opcode::Ptrtoint, Type::I64, area);
            let zero = self.builder.iconst(Type::I64, 0);
            let negated = self
                .builder
                .binary(Opcode::Isub, InstFlags::empty(), zero, address);
            let mask = self
                .builder
                .iconst(Type::I64, i128::from(class.overflow_align - 1));
            let padding = self
                .builder
                .binary(Opcode::And, InstFlags::empty(), negated, mask);
            area = self.builder.ptr_add(InstFlags::empty(), area, padding);
        }
        let size = self
            .builder
            .iconst(Type::I64, i128::from(class.overflow_size));
        let next = self.builder.ptr_add(InstFlags::empty(), area, size);
        self.builder.store(next, field, pointer_align);
        area
    }

    /// The address of the next argument in the Win64 `va_list` at `list`, a
    /// pointer to its 8-byte slot, which this advances.
    /// Microsoft x64, "Varargs".
    fn win64_va_arg(&mut self, list: Value) -> Value {
        let pointer_align = MemFlags::aligned(natural_align(Type::Ptr));
        let slot = self.builder.load(Type::Ptr, list, pointer_align);
        let size = self.builder.iconst(Type::I64, 8);
        let next = self.builder.ptr_add(InstFlags::empty(), slot, size);
        self.builder.store(next, list, pointer_align);
        slot
    }

    /// Reconstructs the object, since SSE save slots are sixteen bytes apart
    /// and mixed classes occupy independent register areas.
    /// C99: §7.15.1.1, pp. 249-250; PDF pp. 261-262; psABI §3.5.7.
    fn aggregate_va_arg(&mut self, layout: AggregateLayout<'_>, list: Value) -> Value {
        let object = self.temporary(layout.size.max(16), layout.align);
        if self.convention == Convention::Win64 {
            let slot = self.win64_va_arg(list);
            let address = match win64::argument(layout) {
                | ArgPlan::Registers { .. } => slot,
                | _ =>
                    self.builder
                        .load(Type::Ptr, slot, MemFlags::aligned(natural_align(Type::Ptr))),
            };
            self.copy(object, address, layout.size, at_most(layout.align, 8));
            return object;
        }
        let stack_class = sysv::VaArgClass {
            gp:             0,
            sse:            false,
            overflow_size:  layout.size.next_multiple_of(8),
            overflow_align: layout.align.bytes().max(8),
        };
        // Variadic arguments follow the psABI in every dialect.
        let arg = sysv::argument(
            layout,
            &mut sysv::ArgumentRegisters::default(),
            sysv::Dialect::Gcc,
        )
        .expect("aggregate descriptor preflight");
        let ArgPlan::Registers { parts, .. } = arg else {
            if !matches!(arg, ArgPlan::Ignore(_)) {
                let address = self.overflow_arg(list, stack_class);
                self.copy(object, address, layout.size, layout.align);
            }
            return object;
        };
        let gp = parts.types().iter().filter(|t| !t.is_float()).count() as u64;
        let sse = parts.types().iter().filter(|t| t.is_float()).count() as u64;
        let gp_field = self.offset(list, va_list::GP_OFFSET);
        let fp_field = self.offset(list, va_list::FP_OFFSET);
        let mem32 = MemFlags::aligned(natural_align(Type::I32));
        let gp_offset = self.builder.load(Type::I32, gp_field, mem32);
        let fp_offset = self.builder.load(Type::I32, fp_field, mem32);
        let gp_limit = self
            .builder
            .iconst(Type::I32, i128::from(va_list::GP_AREA_SIZE - 8 * gp));
        let fp_limit = self
            .builder
            .iconst(Type::I32, i128::from(va_list::SAVE_AREA_SIZE - 16 * sse));
        let gp_fits = self.builder.icmp(IntCC::Ule, gp_offset, gp_limit);
        let fp_fits = self.builder.icmp(IntCC::Ule, fp_offset, fp_limit);
        let fits = if gp == 0 {
            fp_fits
        } else if sse == 0 {
            gp_fits
        } else {
            self.builder
                .binary(Opcode::And, InstFlags::empty(), gp_fits, fp_fits)
        };
        let registers = self.builder.create_block();
        let stack = self.builder.create_block();
        let join = self.builder.create_block();
        self.builder.brif(fits, (registers, &[]), (stack, &[]));
        self.builder.switch_to_block(registers);
        let save_field = self.offset(list, va_list::REG_SAVE_AREA);
        let save = self.builder.load(
            Type::Ptr,
            save_field,
            MemFlags::aligned(natural_align(Type::Ptr)),
        );
        let mut gp_cursor = gp_offset;
        let mut fp_cursor = fp_offset;
        for (index, &ty) in parts.types().iter().enumerate() {
            let cursor = if ty.is_float() { fp_cursor } else { gp_cursor };
            let offset = self.builder.convert(Opcode::Zext, Type::I64, cursor);
            let address = self.builder.ptr_add(InstFlags::empty(), save, offset);
            let value = self.builder.load(
                ty,
                address,
                MemFlags::aligned(at_most(natural_align(ty), 8)),
            );
            let destination = self.offset(object, 8 * index as u64);
            self.builder.store(
                value,
                destination,
                MemFlags::aligned(at_most(layout.align, 8)),
            );
            let step = self
                .builder
                .iconst(Type::I32, if ty.is_float() { 16 } else { 8 });
            let next = self
                .builder
                .binary(Opcode::Iadd, InstFlags::empty(), cursor, step);
            if ty.is_float() {
                fp_cursor = next;
            } else {
                gp_cursor = next;
            }
        }
        if gp > 0 {
            self.builder.store(gp_cursor, gp_field, mem32);
        }
        if sse > 0 {
            self.builder.store(fp_cursor, fp_field, mem32);
        }
        self.builder.jump(join, &[]);
        self.builder.switch_to_block(stack);
        let address = self.overflow_arg(list, stack_class);
        self.copy(object, address, layout.size, layout.align);
        self.builder.jump(join, &[]);
        self.builder.switch_to_block(join);
        object
    }
}

/// Whom a call calls.
#[derive(Clone, Copy)]
enum Callee {
    Direct(FuncId),
    /// Through a pointer, the first value of the old argument list.
    Indirect(Value),
}

/// Each old stack slot, enlarged and aligned for the calls that pass its
/// address: as a `byval` copy, which must have the stack slot's alignment,
/// or as a register result's return area, which must cover the registers.
fn slot_needs<'s>(
    old: &Body<'_>,
    plans: &[SignaturePlan<'_>],
    old_signatures: &[SigId],
    scratch: &'s Bump,
) -> ArenaVec<'s, StackSlotData> {
    let mut slots = ArenaVec::with_capacity_in(old.stack_slot_count(), scratch);
    slots.extend(old.stack_slots().map(|(_, data)| data));
    for block in old.blocks() {
        for &inst in old.block_insts(block) {
            let (sig, args) = match *old.inst(inst) {
                | InstData::Call { func, args } =>
                    (old_signatures[func.index()], old.value_list(args)),
                | InstData::CallIndirect { sig, args } => (sig, &old.value_list(args)[1..]),
                | _ => continue,
            };
            let plan = plans[sig.index()];
            let mut need = |value: Value, size: u64, align: Align| {
                if let Some(slot) = slot_of(old, value) {
                    let data = &mut slots[slot];
                    data.size = data.size.max(slot_size(size));
                    data.align = max_align(data.align, align.bytes());
                }
            };
            if let ResultPlan::Registers { registers, .. } = plan.result {
                need(args[0], registers.bytes(), registers.align);
            }
            for (&value, &arg) in args[plan.first_arg()..].iter().zip(plan.args) {
                if let ArgPlan::Stack { layout, align } = arg {
                    need(value, layout.size, align);
                }
            }
        }
    }
    slots
}

/// The stack slot whose address `value` is, if `stack_addr` defines it.
fn slot_of(body: &Body<'_>, value: Value) -> Option<usize> {
    match body.value_def(value) {
        | ValueDef::Result(inst) => match *body.inst(inst) {
            | InstData::StackAddr { slot } => Some(slot.index()),
            | _ => None,
        },
        | ValueDef::Param(..) => None,
    }
}

/// A slot size already checked by read-only ABI planning. Register and
/// scalar temporaries are at most sixteen bytes; aggregate layouts fit `u32`.
fn slot_size(size: u64) -> u32 {
    stack_slot_size(size).expect("ABI planning checked aggregate stack-slot sizes")
}

/// The larger of `align` and `bytes`.
fn max_align(align: Align, bytes: u64) -> Align {
    if align.bytes() >= bytes {
        align
    } else {
        Align::from_bytes(bytes).unwrap_or(align)
    }
}

/// The alignment of a scalar of type `ty`, as it is stored: its size
/// rounded up to a power of two, at most 16.
fn natural_align(ty: Type) -> Align {
    let bytes = u64::from(ty.bytes()).next_power_of_two().min(16);
    Align::from_bytes(bytes).unwrap_or(Align::BYTE)
}

/// The alignment of `size` bytes at `offset` in an object aligned to
/// `align`.
fn align_at(align: Align, offset: u64, size: u64) -> Align {
    let mut bytes = align.bytes();
    while !offset.is_multiple_of(bytes) {
        bytes /= 2;
    }
    Align::from_bytes(bytes.min(size.next_power_of_two())).unwrap_or(Align::BYTE)
}

/// The integer type of `bytes` bytes, a power of two up to sixteen.
fn int_type(bytes: u64) -> Type {
    match bytes {
        | 1 => Type::I8,
        | 2 => Type::I16,
        | 4 => Type::I32,
        | 16 => Type::I128,
        | _ => Type::I64,
    }
}

/// `align`, at most `bytes`.
fn at_most(align: Align, bytes: u64) -> Align {
    if align.bytes() <= bytes {
        align
    } else {
        Align::from_bytes(bytes).unwrap_or(align)
    }
}
