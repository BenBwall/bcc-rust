//! libgcc's integer-power helpers, `__powisf2`, `__powidf2`, `__powixf2` and
//! `__powitf2`, which a GCC-identity `__builtin_powi` calls on Windows,
//! where lowering names the routine rather than emitting the IR's `powi`
//! intrinsic. They compute as the intrinsic does ([`arithmetic::powi`]),
//! which follows libgcc's loop.
//! GNU extension: GCC manual, "Other Builtins"; libgcc's `libgcc2.c`,
//! `__powidf2`.

use super::{
    Fault,
    HostCall,
    HostReturn,
    Type,
    UbKind,
};
use crate::backend::interpreter::arithmetic;

/// The base's type of the helper `name`, if it is one.
fn precision(name: &str) -> Option<Type> {
    Some(match name {
        | "__powisf2" => Type::F32,
        | "__powidf2" => Type::F64,
        | "__powixf2" => Type::F80,
        | "__powitf2" => Type::F128,
        | _ => return None,
    })
}

pub(super) fn provides(name: &str) -> bool {
    precision(name).is_some()
}

/// Runs the helper `call` names, or `None` for another name. A call whose
/// types are not the helper's `TYPE (TYPE, int)` is undefined.
pub(super) fn call(call: HostCall<'_>) -> Option<Result<HostReturn, Fault>> {
    let ty = precision(call.name)?;
    if call.result != Some(ty) || call.arg_types != [ty, Type::I32] || call.args.len() != 2 {
        return Some(Err(UbKind::SignatureMismatch.into()));
    }
    if call.args.iter().any(|value| value.is_poison()) {
        return Some(Err(UbKind::PoisonInHostCall.into()));
    }
    let value = arithmetic::powi(ty, call.args[0], call.args[1]);
    Some(Ok(HostReturn::Value(Some(value))))
}
