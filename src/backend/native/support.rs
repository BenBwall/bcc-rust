//! The support check: whether the native back end implements everything a
//! module uses. It runs before anything is emitted, so a module is either
//! compiled whole or refused with the first construct it lacks, never
//! compiled in part or wrongly.
//!
//! The back end (see `docs/design/native-backend.md`) compiles integer,
//! pointer, `i128`, `f32`, `f64`, `f80` and `f128` code, variadic
//! definitions and dynamic stack allocation. It refuses `i128` bit counts,
//! byte swaps and `switch`es, `i128` values where Win64 passes them only by
//! reference, structure results in register combinations other than the
//! ones System V and Win64 define, stack arguments aligned above 16 bytes,
//! stack argument areas whose offsets would leave a 32-bit displacement,
//! thread-local globals in named sections, floating and `i128` atomic
//! minimum and maximum, and atomic accesses aligned below their size.

use std::fmt;

use super::RUNTIME_SYMBOLS;
use crate::{
    ir::{
        AtomicOp,
        Body,
        Function,
        GlobalId,
        GlobalInit,
        InstData,
        Linkage,
        Module,
        Opcode,
        ParamAbi,
        Profile,
        RelocationTarget,
        Signature,
        Symbol,
        Type,
    },
    target::Target,
    util::bump::{
        ArenaSet,
        Bump,
    },
};

/// The largest frame the back end lays out: offsets stay well inside the
/// signed 32-bit displacements of `%rsp`-relative operands.
pub(crate) const MAX_FRAME_BYTES: u64 = 1 << 30;

/// Checks the whole module against its incoming references.
pub(super) fn check_module<'m>(
    module: &'m Module<'_>,
    target: Target,
    references: &GlobalReferences<'_>,
) -> Result<(), Unsupported<'m>> {
    let refuse = |function: Option<&'m str>, construct| Unsupported {
        function,
        construct,
    };
    if module.profile() != Profile::PostAbi {
        return Err(refuse(None, Construct::NotAbiLowered));
    }
    for (_, function) in module.functions() {
        if !plain_symbol(function.name) {
            return Err(refuse(Some(function.name), Construct::SymbolName));
        }
        if function.linkage == Linkage::Internal
            && let Some(name) = runtime_symbol(function.name)
        {
            return Err(refuse(Some(function.name), Construct::RuntimeName(name)));
        }
    }
    for (id, global) in module.globals() {
        if !plain_symbol(global.name) {
            return Err(refuse(Some(global.name), Construct::SymbolName));
        }
        if let Some(name) = runtime_symbol(global.name) {
            return Err(refuse(Some(global.name), Construct::RuntimeName(name)));
        }
        // A thread-local definition belongs in the thread-local data that
        // the TLS offsets count from, not in a named section.
        if global.thread_local
            && global.attributes.section.is_some()
            && !references.unused_internal_global(module, id)
        {
            return Err(refuse(Some(global.name), Construct::ThreadLocalSection));
        }
    }
    let checker = Checker {
        module,
        win64: matches!(target, Target::WindowsGnu | Target::WindowsMsvc),
    };
    for (_, function) in module.functions() {
        let Some(body) = &function.body else { continue };
        checker
            .function(function, body)
            .map_err(|construct| refuse(Some(function.name), construct))?;
    }
    Ok(())
}

/// Incoming global references, collected once for support and emission.
/// C99: §6.9p5, p. 140; PDF p. 152. Optional retention follows Clang.
pub(super) struct GlobalReferences<'a> {
    observed:          ArenaSet<'a, GlobalId>,
    #[cfg(test)]
    pub(super) visits: usize,
}

impl<'a> GlobalReferences<'a> {
    pub(super) fn collect(module: &Module<'_>, arena: &'a Bump) -> Self {
        let mut result = Self {
            observed:            ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, arena),
            #[cfg(test)]
            visits:              0,
        };
        for (_, global) in module.globals() {
            #[cfg(test)]
            {
                result.visits += 1;
            }
            if let Some(Symbol::Global(id)) = global.attributes.alias {
                _ = result.observed.insert(id);
            }
            if let Some(GlobalInit::Bytes { relocations, .. }) = global.init {
                for relocation in relocations {
                    #[cfg(test)]
                    {
                        result.visits += 1;
                    }
                    if let RelocationTarget::Symbol(Symbol::Global(id)) = relocation.target {
                        _ = result.observed.insert(id);
                    }
                }
            }
        }
        for (_, function) in module.functions() {
            if let Some(Symbol::Global(id)) = function.attributes.alias {
                _ = result.observed.insert(id);
            }
            if let Some(body) = &function.body {
                for block in body.blocks() {
                    for &inst in body.block_insts(block) {
                        #[cfg(test)]
                        {
                            result.visits += 1;
                        }
                        if let InstData::GlobalAddr { global } = body.inst(inst) {
                            _ = result.observed.insert(*global);
                        }
                    }
                }
            }
        }
        result
    }

    /// An address is unobserved only when neither code, data, aliases nor
    /// linker retention requests require it. Membership is constant-time.
    pub(super) fn unused_internal_global(&self, module: &Module<'_>, id: GlobalId) -> bool {
        let global = module.global(id);
        global.linkage == Linkage::Internal
            && !global.attributes.used
            && !self.observed.contains(&id)
    }
}

/// The per-function checks and what they need of the module and target.
struct Checker<'c, 'ir> {
    module: &'c Module<'ir>,
    /// Win64 passes `__int128` arguments and results only by reference or
    /// in `%xmm0` (`registers(i128)`), never as plain `i128` positions.
    win64:  bool,
}

impl Checker<'_, '_> {
    /// Checks one defined function: its signature, its values, its stack
    /// slots, and every instruction and call signature.
    fn function(&self, function: &Function<'_>, body: &Body<'_>) -> Result<(), Construct> {
        self.signature(self.module.signature(function.signature))?;
        let mut frame = 0_u64;
        for (_, slot) in body.stack_slots() {
            // An over-aligned object takes `align - 16` spare bytes.
            let align = slot.align.bytes();
            let spare = align.saturating_sub(16);
            frame = frame
                .saturating_add((u64::from(slot.size) + spare).next_multiple_of(align.min(16)));
        }
        if frame > MAX_FRAME_BYTES {
            return Err(Construct::FrameSize(frame));
        }
        for block in body.blocks() {
            for &inst in body.block_insts(block) {
                self.instruction(body, body.inst(inst))?;
            }
        }
        Ok(())
    }

    /// Checks one instruction's opcode and types and, for calls, the
    /// callee's signature and the types of extra variadic arguments.
    fn instruction(&self, body: &Body<'_>, data: &InstData) -> Result<(), Construct> {
        let opcode = data.opcode();
        let wide = |value| body.value_type(value) == Type::I128;
        match *data {
            | InstData::Binary { ty: Type::I1, .. }
                if matches!(
                    opcode,
                    Opcode::Sdiv | Opcode::Udiv | Opcode::Srem | Opcode::Urem
                ) =>
                Err(Construct::Opcode(opcode)),
            | InstData::Unary {
                ty: Type::I1 | Type::I128,
                ..
            } if opcode.is_bit_unary() => Err(Construct::Opcode(opcode)),
            | InstData::Switch { value, .. } if wide(value) => Err(Construct::WideSwitch),
            | InstData::Unary { .. } if opcode == Opcode::VaArg => Err(Construct::Opcode(opcode)),
            | InstData::Atomic { op, ty, align, .. } => atomic(op, ty, align.bytes()),
            | InstData::VaArgAggregate { .. }
            | InstData::Nullary {
                opcode: Opcode::ReadTimestamp,
                ..
            } => Err(Construct::Opcode(opcode)),
            | InstData::Load { ty: Type::I1, .. } | InstData::Store { ty: Type::I1, .. } =>
                Err(Construct::Type(Type::I1)),
            | InstData::Call { func, args } => {
                let signature = self.module.function_signature(func);
                self.signature(signature)?;
                self.variadic_arguments(body, signature, body.value_list(args))
            },
            | InstData::CallIndirect { sig, args } => {
                let signature = self.module.signature(sig);
                self.signature(signature)?;
                let values = body.value_list(args);
                self.variadic_arguments(body, signature, values.get(1..).unwrap_or_default())
            },
            | _ => Ok(()),
        }
    }

    /// Checks a signature's parameter and result types and its aggregate
    /// passing kinds. The structures it copies to the stack (`byval`) must
    /// leave the argument area addressable with the 32-bit displacements of
    /// the copies, as [`MAX_FRAME_BYTES`] keeps a frame.
    fn signature(&self, signature: Signature<'_>) -> Result<(), Construct> {
        for &ty in signature.params.iter().chain(signature.result.iter()) {
            if ty == Type::I128 && self.win64 {
                return Err(Construct::WideByValue);
            }
        }
        let mut area = 8 * signature.params.len() as u64;
        for param in signature.aggregates {
            if let ParamAbi::ByVal(layout) = param.abi {
                area = area.saturating_add(layout.size.next_multiple_of(8));
            }
        }
        if area > MAX_FRAME_BYTES {
            return Err(Construct::ArgumentArea(area));
        }
        for param in signature.aggregates {
            match param.abi {
                | ParamAbi::ByVal(layout) if layout.align.bytes() > 16 =>
                    return Err(Construct::SlotAlignment(layout.align.bytes())),
                | ParamAbi::Registers(registers) if registers.align.bytes() > 16 =>
                    return Err(Construct::SlotAlignment(registers.align.bytes())),
                | ParamAbi::Sret(_) | ParamAbi::ByVal(_) | ParamAbi::Indirect(_) => {},
                // Win64's `__int128` result and System V's `__float128` one
                // in `%xmm0`, and System V's `long double` in `%st0`, or a
                // complex one in `%st0` and `%st1`.
                | ParamAbi::Registers(registers)
                    if matches!(registers.first, Type::I128 | Type::F128 | Type::F80)
                        && registers.second.is_none() => {},
                | ParamAbi::Registers(registers)
                    if registers.first == Type::F80 && registers.second == Some(Type::F80) => {},
                | ParamAbi::Registers(registers) =>
                    for ty in registers.types() {
                        if !matches!(
                            ty,
                            Type::I8
                                | Type::I16
                                | Type::I32
                                | Type::I64
                                | Type::Ptr
                                | Type::F16
                                | Type::F32
                                | Type::F64
                        ) {
                            return Err(Construct::RegisterResult(ty));
                        }
                    },
                | ParamAbi::Aggregate(_) | ParamAbi::Result(_) =>
                    return Err(Construct::NotAbiLowered),
            }
        }
        Ok(())
    }

    /// Checks the arguments a call passes beyond its signature's parameters.
    fn variadic_arguments(
        &self,
        body: &Body<'_>,
        signature: Signature<'_>,
        args: &[crate::ir::Value],
    ) -> Result<(), Construct> {
        for &arg in args.get(signature.params.len()..).unwrap_or_default() {
            let ty = body.value_type(arg);
            if ty == Type::I128 && self.win64 {
                return Err(Construct::WideByValue);
            }
        }
        Ok(())
    }
}

/// Checks an atomic operation: the selector covers every one but floating
/// minimum and maximum, which need IEEE `minNum`/`maxNum` NaN handling, and
/// `i128` minimum and maximum, which libatomic has no routine for; and only
/// objects aligned to their size, since a locked access that straddles a
/// cache line is slow or traps (Intel SDM Volume 3A, "Software Controlled
/// Bus Locking") and a plain one is not atomic. C11: §7.17.7, pp. 282-285;
/// PDF pp. 300-303. Minimum and maximum are GNU extensions.
fn atomic(op: AtomicOp, ty: Type, align: u64) -> Result<(), Construct> {
    let unsupported = matches!(op, AtomicOp::Fmin | AtomicOp::Fmax)
        || (ty == Type::I128
            && matches!(
                op,
                AtomicOp::Min | AtomicOp::Max | AtomicOp::Umin | AtomicOp::Umax
            ));
    if unsupported {
        return Err(Construct::Atomic(op, ty));
    }
    let size = u64::from(ty.bits() / 8);
    if !op.is_fence() && !op.is_bytes() && align < size {
        return Err(Construct::UnderAlignedAtomic { size, align });
    }
    Ok(())
}

/// The runtime symbol spelled `name`, if it is one.
fn runtime_symbol(name: &str) -> Option<&'static str> {
    RUNTIME_SYMBOLS
        .iter()
        .copied()
        .find(|&symbol| symbol == name)
}

/// Whether the assembler can take `name` quoted: no quote, backslash or
/// control character. The IR already forbids NUL.
fn plain_symbol(name: &str) -> bool {
    !name.is_empty()
        && !name
            .chars()
            .any(|c| c == '"' || c == '\\' || c.is_control())
}

/// A construct the native back end does not compile, and where.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Unsupported<'m> {
    pub(crate) function:  Option<&'m str>,
    pub(crate) construct: Construct,
}

/// What is not supported.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Construct {
    /// Memory accesses of this type (`i1`).
    Type(Type),
    Opcode(Opcode),
    /// A `switch` on an `i128`.
    WideSwitch,
    /// A Win64 `i128` passed by value.
    WideByValue,
    /// A structure result returned in registers of this type.
    RegisterResult(Type),
    /// A stack argument or return area aligned above 16 bytes, which the
    /// caller's 16-byte-aligned stack cannot place without realigning it.
    SlotAlignment(u64),
    FrameSize(u64),
    /// Stack arguments, chiefly `byval` structures, of this many bytes.
    ArgumentArea(u64),
    /// A symbol the assembler cannot spell.
    SymbolName,
    /// The module defines, or makes internal, a name the back end calls
    /// itself (`memmove`, `__main`, `___chkstk_ms`, `fmod`, `__divti3`).
    RuntimeName(&'static str),
    /// The module was not lowered for the ABI.
    NotAbiLowered,
    /// A thread-local global placed in a named section.
    ThreadLocalSection,
    /// Inline assembly the back end cannot place, named by the reason.
    /// GNU extension: GCC manual, "Extended Asm".
    InlineAsm(&'static str),
    /// An atomic operation the back end has no sequence for.
    Atomic(AtomicOp, Type),
    /// An atomic access of `size` bytes aligned to fewer.
    UnderAlignedAtomic {
        size:  u64,
        align: u64,
    },
}

impl fmt::Display for Unsupported<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not yet supported by the native back end: ")?;
        match self.construct {
            | Construct::Type(ty) => write!(f, "{ty} values")?,
            | Construct::Opcode(opcode) => write!(f, "the `{opcode}` instruction")?,
            | Construct::InlineAsm(reason) => write!(f, "inline assembly with {reason}")?,
            | Construct::WideSwitch => write!(f, "a switch on an i128 value")?,
            | Construct::WideByValue => write!(f, "an i128 passed by value on Win64")?,
            | Construct::RegisterResult(ty) =>
                write!(f, "a structure result returned in an {ty} register")?,
            | Construct::SlotAlignment(align) =>
                write!(f, "a stack argument aligned to {align} bytes")?,
            | Construct::FrameSize(bytes) => write!(f, "a {bytes}-byte stack frame")?,
            | Construct::ArgumentArea(bytes) => write!(f, "{bytes} bytes of stack arguments")?,
            | Construct::SymbolName => write!(f, "a symbol name with quotes or controls")?,
            | Construct::RuntimeName(name) => write!(
                f,
                "a module-local definition of the runtime symbol `{name}`"
            )?,
            | Construct::NotAbiLowered => write!(f, "a module not lowered for the ABI")?,
            | Construct::ThreadLocalSection =>
                write!(f, "a thread-local global in a named section")?,
            | Construct::Atomic(op, ty) => write!(f, "an atomic `{op}` of {ty} values")?,
            | Construct::UnderAlignedAtomic { size, align } => write!(
                f,
                "an atomic access of {size} bytes aligned to only {align}"
            )?,
        }
        if let Some(function) = self.function {
            write!(f, " (in @{function})")?;
        }
        Ok(())
    }
}
