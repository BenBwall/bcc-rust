//! The support check: whether the native back end implements everything a
//! module uses. It runs before anything is emitted, so a module is either
//! compiled whole or refused with the first construct it lacks, never
//! compiled in part or wrongly.
//!
//! Stage 1 (see `docs/design/native-backend.md`) compiles integer and
//! pointer code. It refuses floating-point and `i128` values, variadic
//! definitions (`va_start`), dynamic stack allocation, aggregate returns in
//! non-integer registers, and stack slots aligned above 16 bytes.

use std::fmt;

use super::RUNTIME_SYMBOLS;
use crate::ir::{
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
};

/// The largest frame the back end lays out: offsets stay well inside the
/// signed 32-bit displacements of `%rsp`-relative operands.
pub(crate) const MAX_FRAME_BYTES: u64 = 1 << 30;

/// Checks the whole module; the first unsupported construct is the error.
pub(crate) fn check_module<'m>(module: &'m Module<'_>) -> Result<(), Unsupported<'m>> {
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
    for (_, function) in module.functions() {
        let Some(body) = &function.body else { continue };
        check_function(module, function, body)
            .map_err(|construct| refuse(Some(function.name), construct))?;
    }
    Ok(())
}

/// Checks one defined function: its signature, its values, its stack
/// slots, and every instruction and call signature.
fn check_function(
    module: &Module<'_>,
    function: &Function<'_>,
    body: &Body<'_>,
) -> Result<(), Construct> {
    signature(module.signature(function.signature))?;
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
            instruction(module, body, body.inst(inst))?;
        }
    }
    Ok(())
}

/// Checks one instruction's opcode and, for calls, the callee's signature
/// and the types of extra variadic arguments.
fn instruction(module: &Module<'_>, body: &Body<'_>, data: &InstData) -> Result<(), Construct> {
    let opcode = data.opcode();
    match *data {
        | InstData::Binary { .. } if opcode.is_float_binary() || matches!(opcode, Opcode::VaCopy) =>
            Err(Construct::Opcode(opcode)),
        | InstData::Binary { ty: Type::I1, .. }
            if matches!(
                opcode,
                Opcode::Sdiv | Opcode::Udiv | Opcode::Srem | Opcode::Urem
            ) =>
            Err(Construct::Opcode(opcode)),
        | InstData::Unary { ty: Type::I1, .. } if opcode.is_bit_unary() =>
            Err(Construct::Opcode(opcode)),
        | InstData::Unary { .. }
            if matches!(
                opcode,
                Opcode::Fneg
                    | Opcode::Fpext
                    | Opcode::Fptrunc
                    | Opcode::Fptosi
                    | Opcode::Fptoui
                    | Opcode::Sitofp
                    | Opcode::Uitofp
                    | Opcode::VaStart
                    | Opcode::VaEnd
                    | Opcode::VaArg
                    | Opcode::StackRestore
            ) =>
            Err(Construct::Opcode(opcode)),
        | InstData::DynAlloca { .. }
        | InstData::VaArgAggregate { .. }
        | InstData::FloatCompare { .. }
        | InstData::WideConst { .. }
        | InstData::Nullary {
            opcode: Opcode::StackSave,
            ..
        } => Err(Construct::Opcode(opcode)),
        | InstData::Load { ty: Type::I1, .. } | InstData::Store { ty: Type::I1, .. } =>
            Err(Construct::Type(Type::I1)),
        | InstData::Call { func, args } => {
            let signature = module.function_signature(func);
            self::signature(signature)?;
            variadic_arguments(body, signature, body.value_list(args))
        },
        | InstData::CallIndirect { sig, args } => {
            let signature = module.signature(sig);
            self::signature(signature)?;
            let values = body.value_list(args);
            variadic_arguments(body, signature, values.get(1..).unwrap_or_default())
        },
        | _ => Ok(()),
    }
}

/// Checks a signature's parameter and result types and its aggregate
/// passing kinds.
fn signature(signature: Signature<'_>) -> Result<(), Construct> {
    for &ty in signature.params {
        scalar(ty)?;
    }
    if let Some(ty) = signature.result {
        scalar(ty)?;
    }
    for param in signature.aggregates {
        match param.abi {
            | ParamAbi::Sret(_) | ParamAbi::ByVal(_) => {},
            | ParamAbi::Registers(registers) =>
                for ty in registers.types() {
                    if !matches!(ty, Type::I8 | Type::I16 | Type::I32 | Type::I64 | Type::Ptr) {
                        return Err(Construct::RegisterResult(ty));
                    }
                },
            | ParamAbi::Aggregate(_) | ParamAbi::Result(_) => return Err(Construct::NotAbiLowered),
        }
    }
    Ok(())
}

/// Checks the arguments a call passes beyond its signature's parameters.
fn variadic_arguments(
    body: &Body<'_>,
    signature: Signature<'_>,
    args: &[crate::ir::Value],
) -> Result<(), Construct> {
    for &arg in args.get(signature.params.len()..).unwrap_or_default() {
        scalar(body.value_type(arg))?;
    }
    Ok(())
}

/// Whether the back end handles values of `ty`.
fn scalar(ty: Type) -> Result<(), Construct> {
    match ty {
        | Type::I1 | Type::I8 | Type::I16 | Type::I32 | Type::I64 | Type::Ptr => Ok(()),
        | Type::I128 | Type::F32 | Type::F64 | Type::F80 | Type::F128 => Err(Construct::Type(ty)),
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
    /// Values of this type (floats and `i128` in stage 1).
    Type(Type),
    Opcode(Opcode),
    /// A structure result returned in registers of this type.
    RegisterResult(Type),
    SlotAlignment(u64),
    FrameSize(u64),
    /// A symbol the assembler cannot spell.
    SymbolName,
    /// The module defines, or makes internal, a name the back end calls
    /// itself (`memcpy`, `__main`, `___chkstk_ms`).
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
