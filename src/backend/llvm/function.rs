//! Direct post-ABI instruction construction. Blocks retain layout order;
//! instructions are built in reverse postorder so dominating values exist.
//! Every labeled block exists, reachable or not, because `blockaddress`
//! names it; one the entry does not reach holds only `unreachable`.
use std::fmt::Write as _;

use llvm_sys::{
    LLVMIntPredicate,
    LLVMOpcode,
    LLVMRealPredicate,
};

use super::{
    EmitError,
    Emitter,
    api::{
        self,
        Session,
        Ty,
        Val,
    },
};
use crate::{
    ir::{
        Block,
        BlockCall,
        Body,
        ControlFlowGraph,
        DominatorTree,
        Entity,
        Extension,
        FloatCC,
        FuncId,
        Inst,
        InstData,
        InstFlags,
        IntCC,
        Module,
        Opcode,
        ParamAbi,
        ResultRegisters,
        Signature,
        Type,
        Value,
    },
    util::bump::{
        ArenaString,
        ArenaVec,
        Bump,
    },
};

impl<'a, 'id, 'm> Emitter<'a, 'id, 'm, '_> {
    /// Emits the body of `id`, if it has one, and returns the LLVM block of
    /// each of its labels, in label order, for relocations in globals.
    pub(super) fn function(&self, id: FuncId) -> Result<&'a [api::Block<'id>], EmitError<'m>> {
        let function = self.module.function(id);
        let Some(body) = &function.body else {
            return Ok(&[]);
        };
        let api = self.api;
        let signature = self.module.signature(function.signature);
        let entry = body.entry_block().ok_or(std::fmt::Error)?;
        let edges = IncomingEdges::compute(body, self.arena);
        let mut blocks = ArenaVec::new_in(self.arena);
        blocks.resize(body.block_count(), None);
        let mut labeled = ArenaVec::new_in(self.arena);
        labeled.resize(body.block_count(), false);
        for (_, block) in body.labels() {
            labeled[block.index()] = true;
        }
        for block in body.blocks() {
            if edges.reachable[block.index()] || labeled[block.index()] {
                blocks[block.index()] = Some(api.block(
                    self.functions[id.index()],
                    api.name(&named(block, self.arena)),
                ));
            }
        }
        for block in body.blocks() {
            if labeled[block.index()] && !edges.reachable[block.index()] {
                api.position(blocks[block.index()].ok_or(std::fmt::Error)?);
                _ = api.unreachable();
            }
        }
        let mut values = ArenaVec::new_in(self.arena);
        values.resize(body.value_count(), None);
        api.position(blocks[entry.index()].ok_or(std::fmt::Error)?);
        let mut slots = ArenaVec::new_in(self.arena);
        for (slot, data) in body.stack_slots() {
            let value = api.alloca(api.array(api.ty(Type::I8), u64::from(data.size)));
            api.alignment(value, data.align.bytes());
            api.set_name(value, &named(slot, self.arena));
            slots.push(value);
        }
        let mut index = 0;
        for (ir_index, &param) in body.block_params(entry).iter().enumerate() {
            let value = if let Some(ParamAbi::Registers(registers)) = signature.param_abi(ir_index)
            {
                let area = api.alloca(api.array(api.ty(Type::I8), registers.area_bytes()));
                api.alignment(area, registers.align.bytes().max(8));
                area
            } else {
                let param = api.param(self.functions[id.index()], index);
                index += 1;
                param
            };
            api.set_name(value, &named(param, self.arena));
            values[param.index()] = Some(value);
        }
        for block in body.blocks() {
            if block == entry || !edges.reachable[block.index()] {
                continue;
            }
            api.position(blocks[block.index()].ok_or(std::fmt::Error)?);
            for &param in body.block_params(block) {
                let value = api.phi(api.ty(body.value_type(param)));
                api.set_name(value, &named(param, self.arena));
                values[param.index()] = Some(value);
            }
        }
        let cfg = ControlFlowGraph::compute(body, self.arena);
        let dom = DominatorTree::compute(body, &cfg, self.arena);
        let mut builder = BodyBuilder {
            emitter: self,
            body,
            signature,
            function: self.functions[id.index()],
            blocks: &blocks,
            slots: &slots,
            values,
        };
        for &block in dom.reverse_post_order() {
            api.position(blocks[block.index()].ok_or(std::fmt::Error)?);
            for &inst in body.block_insts(block) {
                let value = builder.inst(inst)?;
                if let (Some(result), Some(value)) = (body.inst_result(inst), value) {
                    // LLVM may constant-fold builder operations. Constants and
                    // addresses have no SSA name; naming
                    // globals here would rename their symbols.
                    if !folds(self.module, body.inst(inst)) {
                        api.set_name(value, &named(result, self.arena));
                    }
                    builder.values[result.index()] = Some(value);
                }
            }
        }
        for block in body.blocks() {
            if block == entry || !edges.reachable[block.index()] {
                continue;
            }
            for (index, &param) in body.block_params(block).iter().enumerate() {
                let phi = builder.value(param)?;
                for edge in edges.incoming(block) {
                    let arg = builder.value(body.block_call(edge.call).1[index])?;
                    api.incoming(phi, arg, blocks[edge.pred.index()].ok_or(std::fmt::Error)?);
                }
            }
        }
        let mut labels = ArenaVec::with_capacity_in(body.label_count(), self.arena);
        for (_, block) in body.labels() {
            labels.push(blocks[block.index()].ok_or(std::fmt::Error)?);
        }
        Ok(self.arena.alloc_slice_copy(&labels))
    }
}

pub(super) fn signature_type<'id>(
    api: &Session<'id>,
    signature: Signature<'_>,
    arena: &Bump,
) -> Ty<'id> {
    let mut types = ArenaVec::with_capacity_in(signature.params.len(), arena);
    for (index, &ty) in signature.params[..signature.fixed_count()]
        .iter()
        .enumerate()
    {
        if passed(signature, index) {
            types.push(api.ty(ty));
        }
    }
    api.signature(result_type(api, signature), &types, signature.variadic)
}

fn result_type<'id>(api: &Session<'id>, signature: Signature<'_>) -> Ty<'id> {
    match signature.result_param() {
        | Some(ParamAbi::Registers(registers)) => registers_type(api, registers),
        | _ => signature.result.map_or_else(|| api.void(), |ty| api.ty(ty)),
    }
}

fn registers_type<'id>(api: &Session<'id>, registers: ResultRegisters) -> Ty<'id> {
    // The verifier enforces the shared return-area layout, so an unpacked
    // LLVM structure places the second part at ResultRegisters::SECOND_OFFSET.
    debug_assert!(registers.valid_layout(), "verified register return layout");
    match registers.second {
        | Some(second) => api.structure(&[api.ty(registers.first), api.ty(second)], false),
        | None if registers.first == Type::I128 => api.vector(api.ty(Type::I64), 2),
        | None => api.ty(registers.first),
    }
}

fn passed(signature: Signature<'_>, index: usize) -> bool {
    !matches!(signature.param_abi(index), Some(ParamAbi::Registers(_)))
}

pub(super) fn attributes<'id>(
    api: &Session<'id>,
    value: Val<'id>,
    signature: Signature<'_>,
    call: bool,
) {
    extension(api, value, 0, signature.result_extension, call);
    if signature.result_noundef {
        api.attribute(value, 0, "noundef", 0, None, call);
    }
    let mut llvm_index = 1;
    for index in 0..signature.params.len() {
        if !passed(signature, index) {
            continue;
        }
        match signature.param_abi(index) {
            | Some(ParamAbi::ByVal(layout) | ParamAbi::Sret(layout)) => {
                let name = if matches!(signature.param_abi(index), Some(ParamAbi::ByVal(_))) {
                    "byval"
                } else {
                    "sret"
                };
                api.attribute(
                    value,
                    llvm_index,
                    name,
                    0,
                    Some(api.array(api.ty(Type::I8), layout.size)),
                    call,
                );
                api.attribute(value, llvm_index, "align", layout.align.bytes(), None, call);
            },
            | _ => {},
        }
        extension(
            api,
            value,
            llvm_index,
            signature.param_extension(index),
            call,
        );
        if signature.param_noundef(index) {
            api.attribute(value, llvm_index, "noundef", 0, None, call);
        }
        llvm_index += 1;
    }
}

/// Marks a function or call `returns_twice`, which keeps LLVM from
/// reusing stack slots or keeping state where the second return of a
/// `setjmp` cannot find it. LLVM's function attribute index is `~0`.
/// C99: §7.13.2.1 paragraphs 3-4, p. 244; PDF p. 256.
pub(super) fn returns_twice<'id>(api: &Session<'id>, value: Val<'id>, call: bool) {
    api.attribute(value, u32::MAX, "returns_twice", 0, None, call);
}

/// Maps the register extension only; poison remains legal at calls and returns.
/// C99: §6.2.5 paragraphs 2-6, p. 33; PDF p. 45 (integer types);
/// the extension convention and poison semantics are additional IR contracts.
fn extension<'id>(
    api: &Session<'id>,
    value: Val<'id>,
    index: u32,
    extension: Option<Extension>,
    call: bool,
) {
    let name = match extension {
        | Some(Extension::Sign) => "signext",
        | Some(Extension::Zero) => "zeroext",
        | _ => return,
    };
    api.attribute(value, index, name, 0, None, call);
}

struct BodyBuilder<'a, 'id, 'm, 'ir> {
    emitter:   &'a Emitter<'a, 'id, 'm, 'ir>,
    body:      &'m Body<'ir>,
    signature: Signature<'ir>,
    function:  Val<'id>,
    blocks:    &'a [Option<api::Block<'id>>],
    slots:     &'a [Val<'id>],
    values:    ArenaVec<'a, Option<Val<'id>>>,
}

impl<'id, 'm> BodyBuilder<'_, 'id, 'm, '_> {
    fn value(&self, value: Value) -> Result<Val<'id>, EmitError<'m>> {
        if let Some(value) = self.values[value.index()] {
            return Ok(value);
        }
        // Constants need not have been visited, including constants in a block
        // that is unreachable but used on a reachable phi edge after
        // transforms.
        let crate::ir::ValueDef::Result(inst) = self.body.value_def(value) else {
            return Err(std::fmt::Error.into());
        };
        self.constant(self.body.inst(inst))
            .ok_or_else(|| std::fmt::Error.into())
    }

    fn constant(&self, data: &InstData) -> Option<Val<'id>> {
        let api = self.emitter.api;
        Some(match *data {
            | InstData::Const { ty, .. } | InstData::WideConst { ty, .. } =>
                api.bits(ty, self.body.const_bits(data)?),
            | InstData::Nullary {
                opcode: Opcode::Null,
                ty,
            } => api.null(api.ty(ty)),
            | InstData::Nullary {
                opcode: Opcode::Poison,
                ty,
            } => api.poison(api.ty(ty)),
            | InstData::StackAddr { slot } => self.slots[slot.index()],
            | InstData::GlobalAddr { global } => self.emitter.globals[global.index()],
            | InstData::FuncAddr { func } => self.emitter.functions[func.index()],
            | InstData::BlockAddr { label } => api.block_address(
                self.function,
                self.blocks[self.body.label_block(label)?.index()]?,
            ),
            | _ => return None,
        })
    }

    fn destination(&self, call: BlockCall) -> Result<api::Block<'id>, EmitError<'m>> {
        self.blocks[self.body.block_call(call).0.index()].ok_or_else(|| std::fmt::Error.into())
    }

    fn inst(&self, inst: Inst) -> Result<Option<Val<'id>>, EmitError<'m>> {
        let data = self.body.inst(inst);
        if folds(self.emitter.module, data) {
            return Ok(self.constant(data));
        }
        let api = self.emitter.api;
        let opcode = data.opcode();
        let value = match *data {
            | InstData::Binary {
                ty,
                flags,
                args: [a, b],
                ..
            } => {
                let (a, b) = (self.value(a)?, self.value(b)?);
                match opcode {
                    | _ if opcode.is_overflow() => {
                        let name = if opcode == Opcode::UaddOverflow {
                            "llvm.uadd.with.overflow"
                        } else {
                            "llvm.umul.with.overflow"
                        };
                        let pair = self.intrinsic(name, &[a, b], &[api.ty(ty)]);
                        api.extract_value(pair, 1)
                    },
                    | Opcode::PtrAdd => api.gep(a, b, flags.contains(InstFlags::INBOUNDS)),
                    | Opcode::VaCopy =>
                        self.intrinsic("llvm.va_copy", &[a, b], &[api.ty(Type::Ptr)]),
                    | _ => {
                        let value = api.binary(binary_opcode(opcode), a, b);
                        api.flags(value, flags);
                        value
                    },
                }
            },
            | InstData::DynAlloca { size, align, .. } => {
                let value = api.array_alloca(api.ty(Type::I8), self.value(size)?);
                api.alignment(value, align.bytes());
                value
            },
            | InstData::Unary { ty, arg, .. } => {
                let arg = self.value(arg)?;
                match opcode {
                    | Opcode::Clz | Opcode::ClzDefined | Opcode::Ctz | Opcode::CtzDefined => {
                        let name = if matches!(opcode, Opcode::Clz | Opcode::ClzDefined) {
                            "llvm.ctlz"
                        } else {
                            "llvm.cttz"
                        };
                        self.intrinsic(
                            name,
                            &[
                                arg,
                                api.bits(
                                    Type::I1,
                                    u128::from(matches!(opcode, Opcode::Clz | Opcode::Ctz)),
                                ),
                            ],
                            &[api.ty(ty)],
                        )
                    },
                    | Opcode::Popcount => self.intrinsic("llvm.ctpop", &[arg], &[api.ty(ty)]),
                    | Opcode::Bswap => self.intrinsic("llvm.bswap", &[arg], &[api.ty(ty)]),
                    | Opcode::StackRestore =>
                        self.intrinsic("llvm.stackrestore", &[arg], &[api.ty(Type::Ptr)]),
                    | Opcode::Fneg => api.fneg(arg),
                    | Opcode::Freeze => api.freeze(arg),
                    | Opcode::VaArg => api.va_arg(arg, api.ty(ty)),
                    | Opcode::VaStart =>
                        self.intrinsic("llvm.va_start", &[arg], &[api.ty(Type::Ptr)]),
                    | Opcode::VaEnd => self.intrinsic("llvm.va_end", &[arg], &[api.ty(Type::Ptr)]),
                    | _ => api.cast(cast_opcode(opcode), arg, api.ty(ty)),
                }
            },
            | InstData::IntCompare {
                cond, args: [a, b], ..
            } => api.icmp(int_condition(cond), self.value(a)?, self.value(b)?),
            | InstData::FloatCompare {
                cond, args: [a, b], ..
            } => api.fcmp(float_condition(cond), self.value(a)?, self.value(b)?),
            | InstData::Select {
                args: [cond, a, b], ..
            } => api.select(self.value(cond)?, self.value(a)?, self.value(b)?),
            | InstData::Load {
                ty,
                flags,
                align,
                addr,
                ..
            } => {
                let value = api.load(api.ty(ty), self.value(addr)?);
                api.alignment(value, align.bytes());
                api.volatile(value, flags.contains(InstFlags::VOLATILE));
                value
            },
            | InstData::Store {
                flags,
                align,
                args: [value, addr],
                ..
            } => {
                let value = api.store(self.value(value)?, self.value(addr)?);
                api.alignment(value, align.bytes());
                api.volatile(value, flags.contains(InstFlags::VOLATILE));
                value
            },
            | InstData::MemoryRange {
                flags,
                align,
                args: [dst, src, size],
                ..
            } => {
                let fill = opcode == Opcode::Fill;
                let name = if fill {
                    "llvm.memset"
                } else if flags.contains(InstFlags::MAY_OVERLAP) {
                    "llvm.memmove"
                } else {
                    "llvm.memcpy"
                };
                let ptr = api.ty(Type::Ptr);
                let overloads = if fill {
                    &[ptr, api.ty(Type::I64)][..]
                } else {
                    &[ptr, ptr, api.ty(Type::I64)][..]
                };
                let call = self.intrinsic(
                    name,
                    &[
                        self.value(dst)?,
                        self.value(src)?,
                        self.value(size)?,
                        api.bits(Type::I1, u128::from(flags.contains(InstFlags::VOLATILE))),
                    ],
                    overloads,
                );
                api.attribute(call, 1, "align", align.bytes(), None, true);
                if !fill {
                    api.attribute(call, 2, "align", align.bytes(), None, true);
                }
                call
            },
            | InstData::Call { func, args } => {
                let call = self.call(
                    inst,
                    self.emitter.functions[func.index()],
                    self.emitter.module.function_signature(func),
                    self.body.value_list(args),
                )?;
                if self.emitter.module.returns_twice(func) {
                    returns_twice(api, call, true);
                }
                call
            },
            | InstData::CallIndirect { sig, args } => {
                let (&callee, args) = self
                    .body
                    .value_list(args)
                    .split_first()
                    .ok_or(std::fmt::Error)?;
                let call = self.call(
                    inst,
                    self.value(callee)?,
                    self.emitter.module.signature(sig),
                    args,
                )?;
                if self.emitter.indirect_returns_twice {
                    returns_twice(api, call, true);
                }
                call
            },
            | InstData::Jump { dest } => api.jump(self.destination(dest)?),
            | InstData::Brif {
                cond,
                dests: [then, other],
            } => api.branch(
                self.value(cond)?,
                self.destination(then)?,
                self.destination(other)?,
            ),
            | InstData::Switch {
                value,
                default,
                cases,
            } => {
                let cases = self.body.switch_cases(cases);
                let switch = api.switch(
                    self.value(value)?,
                    self.destination(default)?,
                    u32::try_from(cases.len()).map_err(|_| std::fmt::Error)?,
                );
                for (bits, dest) in cases {
                    api.add_case(
                        switch,
                        api.bits(self.body.value_type(value), bits),
                        self.destination(dest)?,
                    );
                }
                switch
            },
            | InstData::Return { value: Some(value) } => api.ret(self.value(value)?),
            | InstData::Return { value: None } => {
                if let Some(ParamAbi::Registers(registers)) = self.signature.result_param() {
                    let entry = self.body.entry_block().ok_or(std::fmt::Error)?;
                    let area = *self
                        .body
                        .block_params(entry)
                        .first()
                        .ok_or(std::fmt::Error)?;
                    let value = self.register_return(registers, self.value(area)?);
                    api.set_name(value, &return_name(inst, self.emitter.arena));
                    api.ret(value)
                } else {
                    api.ret_void()
                }
            },
            | InstData::Nullary {
                opcode: Opcode::StackSave,
                ..
            } => self.intrinsic("llvm.stacksave", &[], &[api.ty(Type::Ptr)]),
            | InstData::Nullary {
                opcode: Opcode::FrameAddress,
                ..
            } => {
                let level = api.bits(Type::I32, 0);
                self.intrinsic("llvm.frameaddress", &[level], &[api.ty(Type::Ptr)])
            },
            | InstData::Nullary {
                opcode: Opcode::ReturnAddress,
                ..
            } => {
                let level = api.bits(Type::I32, 0);
                self.intrinsic("llvm.returnaddress", &[level], &[api.ty(Type::Ptr)])
            },
            | InstData::Nullary {
                opcode: Opcode::Trap,
                ..
            } => self.intrinsic("llvm.trap", &[], &[]),
            | InstData::IndirectBr { addr, dests } => {
                let dests = self.body.block_list(dests);
                let branch = api.indirect_br(
                    self.value(addr)?,
                    u32::try_from(dests.len()).map_err(|_| std::fmt::Error)?,
                );
                for dest in dests {
                    api.add_destination(branch, self.destination(dest)?);
                }
                branch
            },
            | InstData::Unreachable => api.unreachable(),
            // The evaluating thread's copy, as Clang computes it; a bare use
            // of the global would let LLVM reuse one thread's address in
            // another.
            | InstData::GlobalAddr { global } => self.intrinsic(
                "llvm.threadlocal.address",
                &[self.emitter.globals[global.index()]],
                &[api.ty(Type::Ptr)],
            ),
            | _ => return Err(std::fmt::Error.into()),
        };
        Ok(Some(value))
    }

    /// Packs the same bytes `interpreter::return_from` copies independently.
    /// A scalar load would spread one poison byte over its defined neighbors.
    /// Byte-vector loads retain lane poison, and vector freeze freezes each
    /// byte separately before the bitcast combines them into a register.
    /// C99: §6.2.6.1 paragraph 4, p. 37; PDF p. 49 (object representation);
    /// register packing and poison refinement are additional IR contracts.
    fn register_return(&self, registers: ResultRegisters, area: Val<'id>) -> Val<'id> {
        let api = self.emitter.api;
        let result_ty = registers_type(api, registers);
        let mut result = api.poison(result_ty);
        for ((offset, ty), index) in registers.parts().zip(0_u32..) {
            let addr = if offset == 0 {
                area
            } else {
                api.gep(area, api.bits(Type::I64, u128::from(offset)), true)
            };
            let bytes = api.load(api.vector(api.ty(Type::I8), ty.bytes()), addr);
            // The private return area is at least eight-byte aligned; the
            // optional second part starts at byte eight.
            let align = registers.align.bytes().max(8);
            api.alignment(
                bytes,
                if offset == 0 {
                    align
                } else {
                    align.min(offset)
                },
            );
            let frozen = if self.signature.result_noundef {
                bytes
            } else {
                api.freeze(bytes)
            };
            let part_ty = if registers.second.is_some() {
                api.ty(ty)
            } else {
                result_ty
            };
            let part = api.cast(LLVMOpcode::LLVMBitCast, frozen, part_ty);
            result = if registers.second.is_some() {
                api.insert_value(result, part, index)
            } else {
                part
            };
        }
        result
    }

    fn intrinsic(&self, name: &str, args: &[Val<'id>], overloads: &[Ty<'id>]) -> Val<'id> {
        let api = self.emitter.api;
        let (function, ty) = api.intrinsic(name, overloads);
        api.call(ty, function, args)
    }

    fn call(
        &self,
        inst: Inst,
        function: Val<'id>,
        signature: Signature<'_>,
        args: &[Value],
    ) -> Result<Val<'id>, EmitError<'m>> {
        let api = self.emitter.api;
        let mut arguments = ArenaVec::with_capacity_in(args.len(), self.emitter.arena);
        for (index, &arg) in args.iter().enumerate() {
            if passed(signature, index) {
                arguments.push(self.value(arg)?);
            }
        }
        let call = api.call(
            signature_type(api, signature, self.emitter.arena),
            function,
            &arguments,
        );
        attributes(api, call, signature, true);
        if signature.variadic_noundef {
            let fixed = signature
                .params
                .iter()
                .enumerate()
                .filter(|&(index, _)| passed(signature, index))
                .count();
            for index in 0..args.len().saturating_sub(signature.params.len()) {
                api.attribute(
                    call,
                    u32::try_from(fixed + index + 1).map_err(|_| std::fmt::Error)?,
                    "noundef",
                    0,
                    None,
                    true,
                );
            }
        }
        if let Some(ParamAbi::Registers(registers)) = signature.result_param() {
            api.set_name(call, &return_name(inst, self.emitter.arena));
            if let Some(&area) = args.first() {
                let store = api.store(call, self.value(area)?);
                api.alignment(store, registers.align.bytes());
            }
        }
        Ok(call)
    }
}

fn named(value: impl std::fmt::Display, arena: &Bump) -> ArenaString<'_> {
    let mut text = ArenaString::new_in(arena);
    write!(text, "{value}").expect("arena formatting succeeds");
    text
}
fn return_name(inst: Inst, arena: &Bump) -> ArenaString<'_> {
    let mut text = named(inst, arena);
    text.push_str(".ret");
    text
}
/// Whether an instruction becomes an LLVM constant rather than an
/// instruction. A thread-local global's address is computed per thread.
fn folds(module: &Module<'_>, data: &InstData) -> bool {
    match *data {
        | InstData::GlobalAddr { global } => !module.global(global).thread_local,
        | _ => matches!(
            data,
            InstData::Const { .. }
                | InstData::WideConst { .. }
                | InstData::Nullary {
                    opcode: Opcode::Poison | Opcode::Null,
                    ..
                }
                | InstData::StackAddr { .. }
                | InstData::FuncAddr { .. }
                | InstData::BlockAddr { .. }
        ),
    }
}

fn binary_opcode(value: Opcode) -> LLVMOpcode {
    match value {
        | Opcode::Iadd => LLVMOpcode::LLVMAdd,
        | Opcode::Isub => LLVMOpcode::LLVMSub,
        | Opcode::Imul => LLVMOpcode::LLVMMul,
        | Opcode::Sdiv => LLVMOpcode::LLVMSDiv,
        | Opcode::Udiv => LLVMOpcode::LLVMUDiv,
        | Opcode::Srem => LLVMOpcode::LLVMSRem,
        | Opcode::Urem => LLVMOpcode::LLVMURem,
        | Opcode::Shl => LLVMOpcode::LLVMShl,
        | Opcode::Lshr => LLVMOpcode::LLVMLShr,
        | Opcode::Ashr => LLVMOpcode::LLVMAShr,
        | Opcode::And => LLVMOpcode::LLVMAnd,
        | Opcode::Or => LLVMOpcode::LLVMOr,
        | Opcode::Xor => LLVMOpcode::LLVMXor,
        | Opcode::Fadd => LLVMOpcode::LLVMFAdd,
        | Opcode::Fsub => LLVMOpcode::LLVMFSub,
        | Opcode::Fmul => LLVMOpcode::LLVMFMul,
        | Opcode::Fdiv => LLVMOpcode::LLVMFDiv,
        | Opcode::Frem => LLVMOpcode::LLVMFRem,
        | _ => unreachable!("verified instruction category"),
    }
}

fn cast_opcode(value: Opcode) -> LLVMOpcode {
    match value {
        | Opcode::Trunc => LLVMOpcode::LLVMTrunc,
        | Opcode::Zext => LLVMOpcode::LLVMZExt,
        | Opcode::Sext => LLVMOpcode::LLVMSExt,
        | Opcode::Fptrunc => LLVMOpcode::LLVMFPTrunc,
        | Opcode::Fpext => LLVMOpcode::LLVMFPExt,
        | Opcode::Fptoui => LLVMOpcode::LLVMFPToUI,
        | Opcode::Fptosi => LLVMOpcode::LLVMFPToSI,
        | Opcode::Uitofp => LLVMOpcode::LLVMUIToFP,
        | Opcode::Sitofp => LLVMOpcode::LLVMSIToFP,
        | Opcode::Ptrtoint => LLVMOpcode::LLVMPtrToInt,
        | Opcode::Inttoptr => LLVMOpcode::LLVMIntToPtr,
        | Opcode::Bitcast => LLVMOpcode::LLVMBitCast,
        | _ => unreachable!("verified instruction category"),
    }
}

fn int_condition(value: IntCC) -> LLVMIntPredicate {
    match value {
        | IntCC::Eq => LLVMIntPredicate::LLVMIntEQ,
        | IntCC::Ne => LLVMIntPredicate::LLVMIntNE,
        | IntCC::Slt => LLVMIntPredicate::LLVMIntSLT,
        | IntCC::Sle => LLVMIntPredicate::LLVMIntSLE,
        | IntCC::Sgt => LLVMIntPredicate::LLVMIntSGT,
        | IntCC::Sge => LLVMIntPredicate::LLVMIntSGE,
        | IntCC::Ult => LLVMIntPredicate::LLVMIntULT,
        | IntCC::Ule => LLVMIntPredicate::LLVMIntULE,
        | IntCC::Ugt => LLVMIntPredicate::LLVMIntUGT,
        | IntCC::Uge => LLVMIntPredicate::LLVMIntUGE,
    }
}

fn float_condition(value: FloatCC) -> LLVMRealPredicate {
    match value {
        | FloatCC::Oeq => LLVMRealPredicate::LLVMRealOEQ,
        | FloatCC::One => LLVMRealPredicate::LLVMRealONE,
        | FloatCC::Olt => LLVMRealPredicate::LLVMRealOLT,
        | FloatCC::Ole => LLVMRealPredicate::LLVMRealOLE,
        | FloatCC::Ogt => LLVMRealPredicate::LLVMRealOGT,
        | FloatCC::Oge => LLVMRealPredicate::LLVMRealOGE,
        | FloatCC::Ord => LLVMRealPredicate::LLVMRealORD,
        | FloatCC::Ueq => LLVMRealPredicate::LLVMRealUEQ,
        | FloatCC::Une => LLVMRealPredicate::LLVMRealUNE,
        | FloatCC::Ult => LLVMRealPredicate::LLVMRealULT,
        | FloatCC::Ule => LLVMRealPredicate::LLVMRealULE,
        | FloatCC::Ugt => LLVMRealPredicate::LLVMRealUGT,
        | FloatCC::Uge => LLVMRealPredicate::LLVMRealUGE,
        | FloatCC::Uno => LLVMRealPredicate::LLVMRealUNO,
    }
}

/// The edges into each block from blocks reachable from the entry block,
/// grouped by target in the order the terminators list them.
struct IncomingEdges<'s> {
    reachable: ArenaVec<'s, bool>,
    /// Every edge leaving a reachable block, in layout and terminator order.
    edges:     ArenaVec<'s, Edge>,
    /// Indices into `edges`, grouped by target block.
    order:     ArenaVec<'s, u32>,
    /// Where each block's group starts in `order`; one more entry than there
    /// are blocks.
    starts:    ArenaVec<'s, u32>,
}

/// One edge: the block whose terminator branches, and the branch.
#[derive(Clone, Copy)]
struct Edge {
    pred:   Block,
    target: Block,
    call:   BlockCall,
}

impl<'s> IncomingEdges<'s> {
    /// Marks the reachable blocks with an explicit stack, then groups their
    /// outgoing edges by target with a counting sort, which keeps each
    /// group in layout and terminator order.
    fn compute(body: &Body<'_>, scratch: &'s Bump) -> Self {
        let count = body.block_count();
        let mut reachable = ArenaVec::with_capacity_in(count, scratch);
        reachable.resize(count, false);
        let mut stack = ArenaVec::new_in(scratch);
        if let Some(entry) = body.entry_block() {
            reachable[entry.index()] = true;
            stack.push(entry);
        }
        while let Some(block) = stack.pop() {
            let Some(terminator) = body.terminator(block) else {
                continue;
            };
            body.visit_destinations(terminator, |call| {
                let target = body.block_call(call).0;
                if !reachable[target.index()] {
                    reachable[target.index()] = true;
                    stack.push(target);
                }
            });
        }
        let mut edges = ArenaVec::new_in(scratch);
        for block in body.blocks() {
            if let Some(terminator) = body.terminator(block)
                && reachable[block.index()]
            {
                body.visit_destinations(terminator, |call| {
                    edges.push(Edge {
                        pred: block,
                        target: body.block_call(call).0,
                        call,
                    });
                });
            }
        }
        let mut starts = ArenaVec::with_capacity_in(count + 1, scratch);
        starts.resize(count + 1, 0_u32);
        for edge in &edges {
            starts[edge.target.index() + 1] += 1;
        }
        for index in 0..count {
            starts[index + 1] += starts[index];
        }
        let mut next = ArenaVec::with_capacity_in(count, scratch);
        next.extend_from_slice(&starts[..count]);
        let mut order = ArenaVec::with_capacity_in(edges.len(), scratch);
        order.resize(edges.len(), 0_u32);
        for (index, edge) in edges.iter().enumerate() {
            let slot = &mut next[edge.target.index()];
            order[*slot as usize] = u32::try_from(index).expect("too many edges");
            *slot += 1;
        }
        Self {
            reachable,
            edges,
            order,
            starts,
        }
    }

    /// The edges into `block`.
    fn incoming(&self, block: Block) -> impl Iterator<Item = Edge> + '_ {
        let start = self.starts[block.index()] as usize;
        let end = self.starts[block.index() + 1] as usize;
        self.order[start..end]
            .iter()
            .map(|&index| self.edges[index as usize])
    }
}
