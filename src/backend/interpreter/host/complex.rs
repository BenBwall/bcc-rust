//! Complex arithmetic helpers and component extraction for the default host.
//! Compiler helpers use the retained guest return area, even for a target
//! register result. Native C sees only real scalars and host output
//! temporaries. C99: §6.2.5p13, p. 34; PDF p. 46; §7.3.9.2p2, p. 179; PDF p.
//! 191; §7.3.9.5p2, p. 180; PDF p. 192.

use super::{
    Fault,
    HostCall,
    HostReturn,
    Memory,
    RuntimeValue,
    Type,
    UbKind,
    Unsupported,
};
use crate::{
    float_parsing::{
        complex_f32,
        complex_f64,
    },
    ir::Align,
    target::Target,
};

pub(super) fn provides(name: &str) -> bool {
    matches!(
        name,
        "__mulsc3"
            | "__divsc3"
            | "__muldc3"
            | "__divdc3"
            | "__mulxc3"
            | "__divxc3"
            | "crealf"
            | "creal"
            | "creall"
            | "cimagf"
            | "cimag"
            | "cimagl"
    )
}

pub(super) fn call(
    call: HostCall<'_>,
    memory: &mut Memory<'_>,
    target: Target,
    long_double: Type,
) -> Option<Result<HostReturn, Fault>> {
    provides(call.name).then(|| evaluate(call, memory, target, long_double))
}

fn evaluate(
    call: HostCall<'_>,
    memory: &mut Memory<'_>,
    target: Target,
    long_double: Type,
) -> Result<HostReturn, Fault> {
    let helper = call.name.starts_with("__");
    let precision = match call.name {
        | "__mulsc3" | "__divsc3" | "crealf" | "cimagf" => Type::F32,
        | "__muldc3" | "__divdc3" | "creal" | "cimag" => Type::F64,
        | _ => long_double,
    };
    if precision == Type::F80 {
        return Err(Fault::Unsupported(Unsupported::WideFloat));
    }
    if call.args.iter().any(|value| value.is_poison()) {
        return Err(UbKind::PoisonInHostCall.into());
    }
    let result = if helper {
        if call.result.is_some()
            || call.arg_types != [Type::Ptr, precision, precision, precision, precision]
            || call.args.len() != 5
        {
            return Err(UbKind::SignatureMismatch.into());
        }
        let pair = match (&call.args[1..], precision) {
            | (
                [
                    RuntimeValue::F32(a),
                    RuntimeValue::F32(b),
                    RuntimeValue::F32(c),
                    RuntimeValue::F32(d),
                ],
                Type::F32,
            ) => {
                let pair = complex_f32(
                    [*a, *b],
                    [*c, *d],
                    if call.name.starts_with("__div") { 3 } else { 2 },
                );
                [RuntimeValue::F32(pair[0]), RuntimeValue::F32(pair[1])]
            },
            | (
                [
                    RuntimeValue::F64(a),
                    RuntimeValue::F64(b),
                    RuntimeValue::F64(c),
                    RuntimeValue::F64(d),
                ],
                Type::F64,
            ) => {
                let pair = complex_f64(
                    [*a, *b],
                    [*c, *d],
                    if call.name.starts_with("__div") { 3 } else { 2 },
                );
                [RuntimeValue::F64(pair[0]), RuntimeValue::F64(pair[1])]
            },
            | _ => return Err(UbKind::SignatureMismatch.into()),
        };
        for (index, value) in pair.into_iter().enumerate() {
            let address = memory.ptr_add(
                call.args[0],
                RuntimeValue::Int(index as u128 * u128::from(precision.bytes())),
                false,
            );
            memory.store(precision, value, address, Align::BYTE)?;
        }
        None
    } else {
        if call.result != Some(precision) {
            return Err(UbKind::SignatureMismatch.into());
        }
        let imaginary = call.name.starts_with("cimag");
        let value = match (call.arg_types, call.args) {
            | ([Type::Ptr], [address]) => {
                let at = memory.ptr_add(
                    *address,
                    RuntimeValue::Int(if imaginary {
                        u128::from(precision.bytes())
                    } else {
                        0
                    }),
                    false,
                );
                memory.load(precision, at, Align::BYTE)?
            },
            | ([Type::F64, Type::F64], [a, b])
                if precision == Type::F64
                    && matches!(target, Target::LinuxGnu | Target::LinuxMusl) =>
                if imaginary {
                    *b
                } else {
                    *a
                },
            | ([Type::F64], [RuntimeValue::F64(bits)])
                if precision == Type::F32
                    && matches!(target, Target::LinuxGnu | Target::LinuxMusl) =>
            {
                let bits = bits.to_bits() >> if imaginary { 32 } else { 0 };
                RuntimeValue::F32(f32::from_bits(
                    u32::try_from(bits & u64::from(u32::MAX)).expect("masked f32 bits"),
                ))
            },
            | ([Type::I64], [RuntimeValue::Int(bits)])
                if precision == Type::F32
                    && matches!(target, Target::WindowsGnu | Target::WindowsMsvc) =>
                RuntimeValue::F32(f32::from_bits(
                    u32::try_from((bits >> if imaginary { 32 } else { 0 }) & u128::from(u32::MAX))
                        .expect("masked f32 bits"),
                )),
            | _ => return Err(UbKind::SignatureMismatch.into()),
        };
        if value.is_poison() {
            return Err(UbKind::PoisonInHostCall.into());
        }
        Some(value)
    };
    Ok(HostReturn::Value(result))
}
