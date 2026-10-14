//! The support check: whether the native back end implements everything a
//! module uses. It runs before anything is emitted, so a module is either
//! compiled whole or refused with the first construct it lacks, never
//! compiled in part or wrongly.
//!
//! The back end (see `docs/design/native-backend.md`) compiles integer,
//! pointer, `i128`, `f32` and `f64` code, variadic definitions and dynamic
//! stack allocation. It refuses `f80` and `f128` values, `i128` bit counts,
//! byte swaps, float conversions and `switch`es, `i128` values where Win64
//! passes them only by reference, and stack objects aligned above 16 bytes.

use std::fmt;

use super::RUNTIME_SYMBOLS;
use crate::{
    ir::{
        Body,
        Function,
        InstData,
        Linkage,
        Module,
        Opcode,
        ParamAbi,
        Profile,
        Signature,
        Type,
    },
    target::Target,
};

/// The largest frame the back end lays out: offsets stay well inside the
/// signed 32-bit displacements of `%rsp`-relative operands.
pub(crate) const MAX_FRAME_BYTES: u64 = 1 << 30;

/// Checks the whole module; the first unsupported construct is the error.
pub(crate) fn check_module<'m>(
    module: &'m Module<'_>,
    target: Target,
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
    for (_, global) in module.globals() {
        if !plain_symbol(global.name) {
            return Err(refuse(Some(global.name), Construct::SymbolName));
        }
        if let Some(name) = runtime_symbol(global.name) {
            return Err(refuse(Some(global.name), Construct::RuntimeName(name)));
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
        for index in 0..body.value_count() {
            scalar(body.value_type(crate::ir::Entity::new(index)))?;
        }
        let mut frame = 0_u64;
        for (_, slot) in body.stack_slots() {
            if slot.align.bytes() > 16 {
                return Err(Construct::SlotAlignment(slot.align.bytes()));
            }
            frame += u64::from(slot.size).next_multiple_of(slot.align.bytes());
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
            | InstData::Unary { ty, arg, .. }
                if matches!(
                    opcode,
                    Opcode::Sitofp | Opcode::Uitofp | Opcode::Fptosi | Opcode::Fptoui
                ) && (ty == Type::I128 || wide(arg)) =>
                Err(Construct::WideConversion(opcode)),
            | InstData::Switch { value, .. } if wide(value) => Err(Construct::WideSwitch),
            | InstData::Unary { .. } if opcode == Opcode::VaArg => Err(Construct::Opcode(opcode)),
            | InstData::VaArgAggregate { .. } => Err(Construct::Opcode(opcode)),
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
    /// passing kinds.
    fn signature(&self, signature: Signature<'_>) -> Result<(), Construct> {
        for &ty in signature.params.iter().chain(signature.result.iter()) {
            scalar(ty)?;
            if ty == Type::I128 && self.win64 {
                return Err(Construct::WideByValue);
            }
        }
        for param in signature.aggregates {
            match param.abi {
                | ParamAbi::ByVal(layout) if layout.align.bytes() > 16 =>
                    return Err(Construct::SlotAlignment(layout.align.bytes())),
                | ParamAbi::Registers(registers) if registers.align.bytes() > 16 =>
                    return Err(Construct::SlotAlignment(registers.align.bytes())),
                | ParamAbi::Sret(_) | ParamAbi::ByVal(_) => {},
                // Win64's `__int128` result in `%xmm0`.
                | ParamAbi::Registers(registers)
                    if registers.first == Type::I128 && registers.second.is_none() => {},
                | ParamAbi::Registers(registers) =>
                    for ty in registers.types() {
                        if !matches!(
                            ty,
                            Type::I8
                                | Type::I16
                                | Type::I32
                                | Type::I64
                                | Type::Ptr
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
            scalar(ty)?;
            if ty == Type::I128 && self.win64 {
                return Err(Construct::WideByValue);
            }
        }
        Ok(())
    }
}

/// Whether the back end handles values of `ty`.
fn scalar(ty: Type) -> Result<(), Construct> {
    match ty {
        | Type::I1
        | Type::I8
        | Type::I16
        | Type::I32
        | Type::I64
        | Type::I128
        | Type::Ptr
        | Type::F32
        | Type::F64 => Ok(()),
        | Type::F80 | Type::F128 => Err(Construct::Type(ty)),
    }
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
    /// Values of this type (`f80` and `f128`).
    Type(Type),
    Opcode(Opcode),
    /// A conversion between `i128` and a float.
    WideConversion(Opcode),
    /// A `switch` on an `i128`.
    WideSwitch,
    /// A Win64 `i128` passed by value.
    WideByValue,
    /// A structure result returned in registers of this type.
    RegisterResult(Type),
    SlotAlignment(u64),
    FrameSize(u64),
    /// A symbol the assembler cannot spell.
    SymbolName,
    /// The module defines, or makes internal, a name the back end calls
    /// itself (`memmove`, `__main`, `___chkstk_ms`, `fmod`, `__divti3`).
    RuntimeName(&'static str),
    /// The module was not lowered for the ABI.
    NotAbiLowered,
}

impl fmt::Display for Unsupported<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not yet supported by the native back end: ")?;
        match self.construct {
            | Construct::Type(ty) => write!(f, "{ty} values")?,
            | Construct::Opcode(opcode) => write!(f, "the `{opcode}` instruction")?,
            | Construct::WideConversion(opcode) =>
                write!(f, "the `{opcode}` instruction between i128 and a float")?,
            | Construct::WideSwitch => write!(f, "a switch on an i128 value")?,
            | Construct::WideByValue => write!(f, "an i128 passed by value on Win64")?,
            | Construct::RegisterResult(ty) =>
                write!(f, "a structure result returned in an {ty} register")?,
            | Construct::SlotAlignment(align) =>
                write!(f, "a stack object aligned to {align} bytes")?,
            | Construct::FrameSize(bytes) => write!(f, "a {bytes}-byte stack frame")?,
            | Construct::SymbolName => write!(f, "a symbol name with quotes or controls")?,
            | Construct::RuntimeName(name) => write!(
                f,
                "a module-local definition of the runtime symbol `{name}`"
            )?,
            | Construct::NotAbiLowered => write!(f, "a module not lowered for the ABI")?,
        }
        if let Some(function) = self.function {
            write!(f, " (in @{function})")?;
        }
        Ok(())
    }
}
