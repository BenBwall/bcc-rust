//! Direct post-ABI instruction construction. Blocks retain layout order;
//! instructions are built in reverse postorder so dominating values exist.
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

impl<'m> Emitter<'_, 'm, '_> {
    pub(super) fn function(&self, id: FuncId) -> Result<(), EmitError<'m>> {
        let function = self.module.function(id);
        let Some(body) = &function.body else {
            return Ok(());
        };
        let api = self.api;
        let signature = self.module.signature(function.signature);
        let entry = body.entry_block().ok_or(std::fmt::Error)?;
        let edges = IncomingEdges::compute(body, self.arena);
        let mut blocks = ArenaVec::new_in(self.arena);
        blocks.resize(body.block_count(), None);
        for block in body.blocks() {
            if edges.reachable[block.index()] {
                blocks[block.index()] = Some(api.block(
                    self.functions[id.index()],
                    api.name(&named(block, self.arena)),
                ));
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
                let area = api.alloca(api.array(api.ty(Type::I8), 16));
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
                    if !folds(body.inst(inst)) {
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
        Ok(())
    }
}

pub(super) fn signature_type<'a>(
    api: &'a Session<'a>,
    signature: Signature<'_>,
    arena: &'a Bump,
) -> Ty<'a> {
    let mut types = ArenaVec::new_in(arena);
    for (index, &ty) in signature.params.iter().enumerate() {
        if passed(signature, index) {
            types.push(api.ty(ty));
        }
    }
    api.signature(result_type(api, signature), &types, signature.variadic)
}

fn result_type<'a>(api: &'a Session<'a>, signature: Signature<'_>) -> Ty<'a> {
    match signature.result_param() {
        | Some(ParamAbi::Registers(registers)) => registers_type(api, registers),
        | _ => signature.result.map_or_else(|| api.void(), |ty| api.ty(ty)),
    }
}

fn registers_type<'a>(api: &'a Session<'a>, registers: ResultRegisters) -> Ty<'a> {
    match registers.second {
        | Some(second) => api.structure(&[api.ty(registers.first), api.ty(second)], false),
        | None if registers.first == Type::I128 => api.vector(api.ty(Type::I64), 2),
        | None => api.ty(registers.first),
    }
}

fn passed(signature: Signature<'_>, index: usize) -> bool {
    !matches!(signature.param_abi(index), Some(ParamAbi::Registers(_)))
}

pub(super) fn attributes(api: &Session<'_>, value: Val<'_>, signature: Signature<'_>, call: bool) {
    extension(api, value, 0, signature.result_extension, call);
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
        llvm_index += 1;
    }
}

fn extension(
    api: &Session<'_>,
    value: Val<'_>,
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
    if index != 0 {
        api.attribute(value, index, "noundef", 0, None, call);
    }
}

struct BodyBuilder<'a, 'm, 'ir> {
    emitter:   &'a Emitter<'a, 'm, 'ir>,
    body:      &'m Body<'ir>,
    signature: Signature<'ir>,
    blocks:    &'a [Option<api::Block<'a>>],
    slots:     &'a [Val<'a>],
    values:    ArenaVec<'a, Option<Val<'a>>>,
}

impl<'a, 'm> BodyBuilder<'a, 'm, '_> {
    fn value(&self, value: Value) -> Result<Val<'a>, EmitError<'m>> {
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

    fn constant(&self, data: &InstData) -> Option<Val<'a>> {
        let api = self.emitter.api;
        Some(match *data {
            | InstData::Const { ty, .. } | InstData::WideConst { ty, .. } =>
                api.bits(ty, self.body.const_bits(data)?),
            | InstData::Nullary {
                opcode: Opcode::Null,
                ty,
            } => api.null(api.ty(ty)),
            | InstData::Nullary { ty, .. } => api.poison(api.ty(ty)),
            | InstData::StackAddr { slot } => self.slots[slot.index()],
            | InstData::GlobalAddr { global } => self.emitter.globals[global.index()],
            | InstData::FuncAddr { func } => self.emitter.functions[func.index()],
            | _ => return None,
        })
    }

    fn destination(&self, call: BlockCall) -> Result<api::Block<'a>, EmitError<'m>> {
        self.blocks[self.body.block_call(call).0.index()].ok_or_else(|| std::fmt::Error.into())
    }

    fn inst(&self, inst: Inst) -> Result<Option<Val<'a>>, EmitError<'m>> {
        let data = self.body.inst(inst);
        if folds(data) {
            return Ok(self.constant(data));
        }
        let api = self.emitter.api;
        let opcode = data.opcode();
        let value = match *data {
            | InstData::Binary {
                flags,
                args: [a, b],
                ..
            } => {
                let (a, b) = (self.value(a)?, self.value(b)?);
                match opcode {
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
            | InstData::Unary { ty, arg, .. } => {
                let arg = self.value(arg)?;
                match opcode {
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
            | InstData::Call { func, args } => self.call(
                inst,
                self.emitter.functions[func.index()],
                self.emitter.module.function_signature(func),
                self.body.value_list(args),
            )?,
            | InstData::CallIndirect { sig, args } => {
                let (&callee, args) = self
                    .body
                    .value_list(args)
                    .split_first()
                    .ok_or(std::fmt::Error)?;
                self.call(
                    inst,
                    self.value(callee)?,
                    self.emitter.module.signature(sig),
                    args,
                )?
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
                    let value = api.load(registers_type(api, registers), self.value(area)?);
                    api.alignment(value, registers.align.bytes().max(8));
                    api.set_name(value, &return_name(inst, self.emitter.arena));
                    api.ret(value)
                } else {
                    api.ret_void()
                }
            },
            | InstData::Unreachable => api.unreachable(),
            | _ => return Err(std::fmt::Error.into()),
        };
        Ok(Some(value))
    }

    fn intrinsic(&self, name: &str, args: &[Val<'_>], overloads: &[Ty<'_>]) -> Val<'a> {
        let api = self.emitter.api;
        let (function, ty) = api.intrinsic(name, overloads);
        api.call(ty, function, args)
    }

    fn call(
        &self,
        inst: Inst,
        function: Val<'_>,
        signature: Signature<'_>,
        args: &[Value],
    ) -> Result<Val<'a>, EmitError<'m>> {
        let api = self.emitter.api;
        let mut arguments = ArenaVec::new_in(self.emitter.arena);
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
fn folds(data: &InstData) -> bool {
    matches!(
        data,
        InstData::Const { .. }
            | InstData::WideConst { .. }
            | InstData::Nullary { .. }
            | InstData::StackAddr { .. }
            | InstData::GlobalAddr { .. }
            | InstData::FuncAddr { .. }
    )
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
