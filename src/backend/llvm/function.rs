//! Functions: declarations, and bodies with their phis, allocas and
//! instructions.

use std::fmt;

use super::{
    Emitter,
    Intrinsics,
    syntax::{
        float,
        int,
        symbol,
        type_name,
    },
};
use crate::{
    ir::{
        Block,
        BlockCall,
        Body,
        Entity,
        FuncId,
        Inst,
        InstData,
        InstFlags,
        Linkage,
        Module,
        Opcode,
        Signature,
        Value,
        ValueDef,
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

impl Emitter<'_, '_, '_> {
    /// Prints a function: a `declare` line for one without a body, else its
    /// `define` with every block reachable from the entry block.
    ///
    /// A definition is `dso_local` or `internal`, as for globals. A
    /// declaration is always external, since LLVM has no internal
    /// declarations.
    pub(super) fn function(&mut self, func: FuncId, scratch: &Bump) -> fmt::Result {
        let module = self.module;
        let function = module.function(func);
        let signature = module.signature(function.signature);
        let result = signature.result.map_or("void", type_name);
        let name = symbol(function.name);
        let Some(body) = &function.body else {
            return writeln!(
                self.out,
                "declare {result} {name}{}",
                parameter_types(signature)
            );
        };
        let linkage = match function.linkage {
            | Linkage::External => "dso_local",
            | Linkage::Internal => "internal",
        };
        write!(self.out, "define {linkage} {result} {name}(")?;
        let entry = body.entry_block().ok_or(fmt::Error)?;
        let mut separator = "";
        for &param in body.block_params(entry) {
            write!(
                self.out,
                "{separator}{} %{param}",
                type_name(body.value_type(param))
            )?;
            separator = ", ";
        }
        if signature.variadic {
            write!(self.out, "{separator}...")?;
        }
        self.out.write_str(") {\n")?;
        let mut printer = BodyPrinter {
            out:        &mut *self.out,
            intrinsics: &mut self.intrinsics,
            values:     Values { module, body },
            edges:      IncomingEdges::compute(body, scratch),
        };
        for block in body.blocks() {
            if printer.edges.reachable[block.index()] {
                printer.block(block)?;
            }
        }
        self.out.write_str("}\n")
    }
}

/// `(i32, ptr, ...)`: a signature's parameter types for a declaration or an
/// indirect call.
fn parameter_types(signature: Signature<'_>) -> impl fmt::Display + '_ {
    fmt::from_fn(move |f| {
        f.write_str("(")?;
        let mut separator = "";
        for &ty in signature.params {
            write!(f, "{separator}{}", type_name(ty))?;
            separator = ", ";
        }
        if signature.variadic {
            write!(f, "{separator}...")?;
        }
        f.write_str(")")
    })
}

/// Prints the blocks of one body.
struct BodyPrinter<'a, 'm, 'ir, 's> {
    out:        &'a mut dyn fmt::Write,
    intrinsics: &'a mut Intrinsics,
    values:     Values<'m, 'ir>,
    edges:      IncomingEdges<'s>,
}

impl BodyPrinter<'_, '_, '_, '_> {
    /// Prints a block: its label, then the entry block's allocas or another
    /// block's phis, one per parameter, then its instructions. The entry
    /// block's parameters are the function's.
    fn block(&mut self, block: Block) -> fmt::Result {
        let body = self.values.body;
        writeln!(self.out, "{block}:")?;
        let entry = Some(block) == body.entry_block();
        if entry {
            for (slot, data) in body.stack_slots() {
                writeln!(
                    self.out,
                    "  %{slot} = alloca [{} x i8], align {}",
                    data.size,
                    data.align.bytes()
                )?;
            }
        }
        let params = if entry { &[] } else { body.block_params(block) };
        for (index, &param) in params.iter().enumerate() {
            write!(
                self.out,
                "  %{param} = phi {} ",
                type_name(body.value_type(param))
            )?;
            let mut separator = "";
            for edge in self.edges.incoming(block) {
                let arg = body.block_call(edge.call).1[index];
                write!(
                    self.out,
                    "{separator}[ {}, %{} ]",
                    self.values.operand(arg),
                    edge.pred
                )?;
                separator = ", ";
            }
            self.out.write_str("\n")?;
        }
        for &inst in body.block_insts(block) {
            self.inst(inst)?;
        }
        Ok(())
    }

    /// Prints one instruction on its own line, unless it is a constant or an
    /// address, which [`Values::operand`] spells at each use instead.
    fn inst(&mut self, inst: Inst) -> fmt::Result {
        let values = self.values;
        let body = values.body;
        let data = *body.inst(inst);
        if folds(&data) {
            return Ok(());
        }
        self.out.write_str("  ")?;
        if let Some(result) = body.inst_result(inst) {
            write!(self.out, "%{result} = ")?;
        }
        let opcode = data.opcode();
        match data {
            | InstData::Binary {
                ty,
                flags,
                args: [a, b],
                ..
            } => match opcode {
                | Opcode::PtrAdd => {
                    let inbounds = if flags.contains(InstFlags::INBOUNDS) {
                        "inbounds "
                    } else {
                        ""
                    };
                    write!(
                        self.out,
                        "getelementptr {inbounds}i8, {}, {}",
                        values.typed(a),
                        values.typed(b)
                    )
                },
                | Opcode::VaCopy => self.intrinsic(
                    Intrinsics::VA_COPY,
                    format_args!("@llvm.va_copy.p0({}, {})", values.typed(a), values.typed(b)),
                ),
                | _ => write!(
                    self.out,
                    "{}{} {} {}, {}",
                    binary_name(opcode),
                    binary_flags(flags),
                    type_name(ty),
                    values.operand(a),
                    values.operand(b)
                ),
            },
            | InstData::Unary { ty, arg, .. } => match opcode {
                | Opcode::Fneg | Opcode::Freeze =>
                    write!(self.out, "{opcode} {}", values.typed(arg)),
                | Opcode::VaArg =>
                    write!(self.out, "va_arg {}, {}", values.typed(arg), type_name(ty)),
                | Opcode::VaStart => self.intrinsic(
                    Intrinsics::VA_START,
                    format_args!("@llvm.va_start.p0({})", values.typed(arg)),
                ),
                | Opcode::VaEnd => self.intrinsic(
                    Intrinsics::VA_END,
                    format_args!("@llvm.va_end.p0({})", values.typed(arg)),
                ),
                | _ => write!(
                    self.out,
                    "{opcode} {} to {}",
                    values.typed(arg),
                    type_name(ty)
                ),
            },
            | InstData::IntCompare {
                cond,
                ty,
                args: [a, b],
            } => write!(
                self.out,
                "icmp {cond} {} {}, {}",
                type_name(ty),
                values.operand(a),
                values.operand(b)
            ),
            | InstData::FloatCompare {
                cond,
                ty,
                args: [a, b],
            } => write!(
                self.out,
                "fcmp {cond} {} {}, {}",
                type_name(ty),
                values.operand(a),
                values.operand(b)
            ),
            | InstData::Select {
                args: [cond, a, b], ..
            } => write!(
                self.out,
                "select {}, {}, {}",
                values.typed(cond),
                values.typed(a),
                values.typed(b)
            ),
            | InstData::Load {
                ty,
                flags,
                align,
                addr,
                ..
            } => write!(
                self.out,
                "load {}{}, {}, align {}",
                volatile(flags),
                type_name(ty),
                values.typed(addr),
                align.bytes()
            ),
            | InstData::Store {
                flags,
                align,
                args: [value, addr],
                ..
            } => write!(
                self.out,
                "store {}{}, {}, align {}",
                volatile(flags),
                values.typed(value),
                values.typed(addr),
                align.bytes()
            ),
            | InstData::MemoryRange {
                flags,
                align,
                args: [dst, src, size],
                ..
            } => {
                let (intrinsic, name) = match opcode {
                    | Opcode::Fill => (Intrinsics::MEMSET, "memset.p0.i64"),
                    | _ if flags.contains(InstFlags::MAY_OVERLAP) =>
                        (Intrinsics::MEMMOVE, "memmove.p0.p0.i64"),
                    | _ => (Intrinsics::MEMCPY, "memcpy.p0.p0.i64"),
                };
                let align = align.bytes();
                let src = fmt::from_fn(|f| {
                    if opcode == Opcode::Fill {
                        write!(f, "{}", values.typed(src))
                    } else {
                        write!(f, "ptr align {align} {}", values.operand(src))
                    }
                });
                self.intrinsic(
                    intrinsic,
                    format_args!(
                        "@llvm.{name}(ptr align {align} {}, {src}, {}, i1 {})",
                        values.operand(dst),
                        values.typed(size),
                        flags.contains(InstFlags::VOLATILE)
                    ),
                )
            },
            | InstData::Call { func, args } => {
                let function = values.module.function(func);
                let signature = values.module.signature(function.signature);
                self.call(
                    signature,
                    signature.variadic,
                    symbol(function.name),
                    body.value_list(args),
                )
            },
            | InstData::CallIndirect { sig, args } => {
                let signature = values.module.signature(sig);
                let (&callee, args) = body.value_list(args).split_first().ok_or(fmt::Error)?;
                self.call(signature, true, values.operand(callee), args)
            },
            | InstData::Jump { dest } => write!(self.out, "br label %{}", body.block_call(dest).0),
            | InstData::Brif {
                cond,
                dests: [then, otherwise],
            } => write!(
                self.out,
                "br {}, label %{}, label %{}",
                values.typed(cond),
                body.block_call(then).0,
                body.block_call(otherwise).0
            ),
            | InstData::Switch {
                value,
                default,
                cases,
            } => {
                let ty = body.value_type(value);
                writeln!(
                    self.out,
                    "switch {}, label %{} [",
                    values.typed(value),
                    body.block_call(default).0
                )?;
                for (bits, dest) in body.switch_cases(cases) {
                    writeln!(
                        self.out,
                        "    {} {}, label %{}",
                        type_name(ty),
                        int(bits, ty),
                        body.block_call(dest).0
                    )?;
                }
                self.out.write_str("  ]")
            },
            | InstData::Return { value } => match value {
                | Some(value) => write!(self.out, "ret {}", values.typed(value)),
                | None => self.out.write_str("ret void"),
            },
            | InstData::Unreachable => self.out.write_str("unreachable"),
            | InstData::Const { .. }
            | InstData::WideConst { .. }
            | InstData::Nullary { .. }
            | InstData::StackAddr { .. }
            | InstData::GlobalAddr { .. }
            | InstData::FuncAddr { .. } => unreachable!("constants and addresses fold"),
        }?;
        self.out.write_str("\n")
    }

    /// A call: the result type alone for a direct call to a fixed-arity
    /// function, else the whole function type, which LLVM needs for a
    /// variadic callee and which states an indirect callee's signature.
    fn call(
        &mut self,
        signature: Signature<'_>,
        function_type: bool,
        callee: impl fmt::Display,
        args: &[Value],
    ) -> fmt::Result {
        let values = self.values;
        let result = signature.result.map_or("void", type_name);
        write!(self.out, "call {result} ")?;
        if function_type {
            write!(self.out, "{} ", parameter_types(signature))?;
        }
        write!(self.out, "{callee}(")?;
        let mut separator = "";
        for &arg in args {
            write!(self.out, "{separator}{}", values.typed(arg))?;
            separator = ", ";
        }
        self.out.write_str(")")
    }

    /// A call of a `void` intrinsic, which the module then declares.
    fn intrinsic(&mut self, intrinsic: Intrinsics, call: fmt::Arguments<'_>) -> fmt::Result {
        *self.intrinsics |= intrinsic;
        write!(self.out, "call void {call}")
    }
}

/// Whether an instruction is a constant or an address, which LLVM spells
/// as an operand rather than computing with an instruction.
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

/// The LLVM name of an integer or floating binary operation.
fn binary_name(opcode: Opcode) -> &'static str {
    match opcode {
        | Opcode::Iadd => "add",
        | Opcode::Isub => "sub",
        | Opcode::Imul => "mul",
        | _ => opcode.name(),
    }
}

/// ` nuw nsw` and ` exact`, in LLVM's order, for the flags a binary
/// operation carries.
fn binary_flags(flags: InstFlags) -> impl fmt::Display {
    fmt::from_fn(move |f| {
        for (flag, name) in [
            (InstFlags::NUW, " nuw"),
            (InstFlags::NSW, " nsw"),
            (InstFlags::EXACT, " exact"),
        ] {
            if flags.contains(flag) {
                f.write_str(name)?;
            }
        }
        Ok(())
    })
}

fn volatile(flags: InstFlags) -> &'static str {
    if flags.contains(InstFlags::VOLATILE) {
        "volatile "
    } else {
        ""
    }
}

/// How the values of one body are spelled as operands.
#[derive(Clone, Copy)]
struct Values<'m, 'ir> {
    module: &'m Module<'ir>,
    body:   &'m Body<'ir>,
}

impl<'m> Values<'m, '_> {
    /// `%v3`, or the constant or address a folded instruction stands for:
    /// an integer, a hexadecimal float, `poison`, `null`, `%slot0`, or a
    /// symbol.
    fn operand(self, value: Value) -> impl fmt::Display + 'm {
        let module = self.module;
        let body = self.body;
        fmt::from_fn(move |f| {
            let ValueDef::Result(inst) = body.value_def(value) else {
                return write!(f, "%{value}");
            };
            let data = body.inst(inst);
            match *data {
                | InstData::Const { opcode, ty, .. } | InstData::WideConst { opcode, ty, .. } => {
                    let bits = body.const_bits(data).ok_or(fmt::Error)?;
                    if opcode == Opcode::Fconst {
                        write!(f, "{}", float(bits, ty))
                    } else {
                        write!(f, "{}", int(bits, ty))
                    }
                },
                | InstData::Nullary { opcode, .. } => f.write_str(if opcode == Opcode::Null {
                    "null"
                } else {
                    "poison"
                }),
                | InstData::StackAddr { slot } => write!(f, "%{slot}"),
                | InstData::GlobalAddr { global } =>
                    write!(f, "{}", symbol(module.global(global).name)),
                | InstData::FuncAddr { func } =>
                    write!(f, "{}", symbol(module.function(func).name)),
                | _ => write!(f, "%{value}"),
            }
        })
    }

    /// The operand with its type before it, as `i32 %v3`.
    fn typed(self, value: Value) -> impl fmt::Display + 'm {
        let ty = type_name(self.body.value_type(value));
        let operand = self.operand(value);
        fmt::from_fn(move |f| write!(f, "{ty} {operand}"))
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
