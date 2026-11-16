//! Named target instructions: the textual form round-trips a vector result
//! with an immediate, a scalar result and an operand-less instruction, and
//! the verifier checks each against its signature.

use super::*;

const INTRINSICS: &str = "\
function @f(ptr, ptr, ptr, i32, f64) -> i32 external {
block0(v0: ptr, v1: ptr, v2: ptr, v3: i32, v4: f64):
    intrinsic x86.pclmulqdq 17, v0, v1, v2
    intrinsic x86.aesenc v0, v1, v2
    v5 = intrinsic x86.pcmpestri128 12, v1, v3, v2, v3
    v6 = intrinsic x86.sqrtsd v4
    intrinsic x86.vzeroupper
    return v5
}
";

#[test]
fn intrinsics_round_trip_and_verify() {
    assert_round_trip(INTRINSICS);
    assert!(
        verify_text(INTRINSICS).is_empty(),
        "{:?}",
        verify_text(INTRINSICS)
    );
}

/// Every intrinsic's name is unique and parses back to it.
#[test]
fn intrinsic_names_are_unique() {
    for &intrinsic in Intrinsic::ALL {
        assert_eq!(Intrinsic::from_name(intrinsic.name()), Some(intrinsic));
    }
}

/// An operand of the wrong type, and an immediate an intrinsic does not
/// take.
#[test]
fn the_verifier_checks_intrinsic_operands() {
    let arena = Bump::new();
    let mut module = Module::new(&arena);
    let signature = module.intern_signature(&[Type::Ptr, Type::I32], None, false);
    let func = module.declare_function("g", signature, Linkage::External);
    let mut builder = FunctionBuilder::new(&mut module, func);
    let entry = builder.create_block();
    builder.switch_to_block(entry);
    let pointer = builder.append_block_param(entry, Type::Ptr);
    let int = builder.append_block_param(entry, Type::I32);
    let args = builder.value_list(&[pointer, int, pointer]);
    _ = builder.insert(InstData::Intrinsic {
        intrinsic: Intrinsic::AesEnc,
        immediate: 3,
        args,
    });
    builder.ret(None);
    builder.finish();
    let errors = verify(&module, Profile::PreAbi);
    assert!(
        errors
            .iter()
            .any(|error| error.contains("operand 1 is i32, expected ptr")),
        "{errors:?}"
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("an intrinsic without an immediate has one")),
        "{errors:?}"
    );
}
