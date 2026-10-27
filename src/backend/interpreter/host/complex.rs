//! Complex arithmetic helpers, component extraction and the `<complex.h>`
//! basics (`conj`, `cproj`, `cabs`, `carg`) for the default host.
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
    if let Some(operation) = basic(call.name, long_double) {
        return Some(evaluate_basic(call, memory, operation));
    }
    provides(call.name).then(|| evaluate(call, memory, target, long_double))
}

/// A `<complex.h>` function of one complex argument.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Basic {
    Conj,
    Proj,
    Abs,
    Arg,
}

/// The operation and component type of a basic complex function.
pub(super) fn basic_provides(name: &str) -> bool {
    basic(name, Type::F80).is_some()
}

fn basic(name: &str, long_double: Type) -> Option<(Basic, Type)> {
    let (stem, precision) = match name.strip_suffix('f') {
        | Some(stem) => (stem, Type::F32),
        | _ => match name.strip_suffix('l') {
            | Some(stem) => (stem, long_double),
            | None => (name, Type::F64),
        },
    };
    let operation = match stem {
        | "conj" => Basic::Conj,
        | "cproj" => Basic::Proj,
        | "cabs" => Basic::Abs,
        | "carg" => Basic::Arg,
        | _ => return None,
    };
    Some((operation, precision))
}

/// `conj`, `cproj`, `cabs` and `carg` in each precision. The value arrives
/// behind a pointer or as the target's scalar parts after the result area;
/// a complex result is written to that area in memory order.
/// C99: §7.3.8.1, p. 178; PDF p. 190 (`cabs`); §7.3.9.1, p. 179; PDF p. 191
/// (`carg`); §7.3.9.3-§7.3.9.4, pp. 179-180; PDF pp. 191-192 (`conj`,
/// `cproj`).
fn evaluate_basic(
    call: HostCall<'_>,
    memory: &mut Memory<'_>,
    (operation, precision): (Basic, Type),
) -> Result<HostReturn, Fault> {
    if call.args.iter().any(|value| value.is_poison()) {
        return Err(UbKind::PoisonInHostCall.into());
    }
    let complex_result = matches!(operation, Basic::Conj | Basic::Proj);
    let (result, operands) = if complex_result {
        match call.args.split_first() {
            | Some((&result, operands))
                if call.arg_types.first() == Some(&Type::Ptr) && call.result.is_none() =>
                (Some(result), operands),
            | _ => return Err(UbKind::SignatureMismatch.into()),
        }
    } else {
        if call.result != Some(precision) {
            return Err(UbKind::SignatureMismatch.into());
        }
        (None, call.args)
    };
    let types = &call.arg_types[usize::from(complex_result)..];
    // The imaginary part's offset in memory: x87 parts are 16 bytes apart.
    let stride = if precision == Type::F80 {
        16
    } else {
        u128::from(precision.bytes())
    };
    let split = |bits: u128| {
        let part = |shift: u32| {
            RuntimeValue::F32(f32::from_bits(
                u32::try_from((bits >> shift) & 0xFFFF_FFFF).expect("masked"),
            ))
        };
        (part(0), part(32))
    };
    let (real, imaginary) = match (types, operands) {
        | ([Type::Ptr], [address]) => {
            let imaginary = memory.ptr_add(*address, RuntimeValue::Int(stride), false);
            (
                memory.load(precision, *address, Align::BYTE)?,
                memory.load(precision, imaginary, Align::BYTE)?,
            )
        },
        | ([first, second], [real, imaginary]) if *first == precision && *second == precision =>
            (*real, *imaginary),
        | ([Type::F64], [RuntimeValue::F64(bits)]) if precision == Type::F32 =>
            split(u128::from(bits.to_bits())),
        | ([Type::I64], [RuntimeValue::Int(bits)]) if precision == Type::F32 => split(*bits),
        | _ => return Err(UbKind::SignatureMismatch.into()),
    };
    let value = match (operation, real, imaginary) {
        | (Basic::Abs | Basic::Arg, RuntimeValue::F80(_), _) =>
            return Err(Fault::Unsupported(Unsupported::WideFloat)),
        | (Basic::Abs, RuntimeValue::F32(x), RuntimeValue::F32(y)) =>
            RuntimeValue::F32(super::math::native::hypotf(x, y)),
        | (Basic::Abs, RuntimeValue::F64(x), RuntimeValue::F64(y)) =>
            RuntimeValue::F64(super::math::native::hypot(x, y)),
        | (Basic::Arg, RuntimeValue::F32(x), RuntimeValue::F32(y)) =>
            RuntimeValue::F32(super::math::native::atan2f(y, x)),
        | (Basic::Arg, RuntimeValue::F64(x), RuntimeValue::F64(y)) =>
            RuntimeValue::F64(super::math::native::atan2(y, x)),
        | (Basic::Abs | Basic::Arg, ..) => return Err(UbKind::SignatureMismatch.into()),
        | (Basic::Conj | Basic::Proj, real, imaginary) => {
            let infinite = |value: RuntimeValue| match value {
                | RuntimeValue::F32(value) => value.is_infinite(),
                | RuntimeValue::F64(value) => value.is_infinite(),
                | RuntimeValue::F80(value) => value.class() == crate::x87::Class::Infinite,
                | _ => false,
            };
            let negative = |value: RuntimeValue| match value {
                | RuntimeValue::F32(value) => RuntimeValue::F32(-value),
                | RuntimeValue::F64(value) => RuntimeValue::F64(-value),
                | RuntimeValue::F80(value) => RuntimeValue::F80(value.negate()),
                | value => value,
            };
            let (real, imaginary) = if operation == Basic::Conj {
                (real, negative(imaginary))
            } else if infinite(real) || infinite(imaginary) {
                // C99 §7.3.9.4p2: an infinite part projects to positive
                // infinity with the imaginary part's sign on zero.
                let sign = match imaginary {
                    | RuntimeValue::F32(value) => value.is_sign_negative(),
                    | RuntimeValue::F64(value) => value.is_sign_negative(),
                    | RuntimeValue::F80(value) => value.is_negative(),
                    | _ => false,
                };
                match precision {
                    | Type::F32 => (
                        RuntimeValue::F32(f32::INFINITY),
                        RuntimeValue::F32(if sign { -0.0 } else { 0.0 }),
                    ),
                    | Type::F64 => (
                        RuntimeValue::F64(f64::INFINITY),
                        RuntimeValue::F64(if sign { -0.0 } else { 0.0 }),
                    ),
                    | _ => (
                        RuntimeValue::F80(crate::x87::X87::infinity(false)),
                        RuntimeValue::F80(crate::x87::X87::zero(sign)),
                    ),
                }
            } else {
                (real, imaginary)
            };
            let result = result.expect("a complex result has an area");
            memory.store(precision, real, result, Align::BYTE)?;
            let second = memory.ptr_add(result, RuntimeValue::Int(stride), false);
            memory.store(precision, imaginary, second, Align::BYTE)?;
            return Ok(HostReturn::Value(None));
        },
    };
    Ok(HostReturn::Value(Some(value)))
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
    // The x87 helpers' recovery of infinite results from NaN parts is not
    // modelled; component extraction reads the parts directly.
    if precision == Type::F80 && helper {
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
