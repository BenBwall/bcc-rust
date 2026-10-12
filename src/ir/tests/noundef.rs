//! Defined-value contracts in both profiles, including signature interning.

use super::*;

#[test]
fn noundef_signatures_round_trip_in_both_profiles() {
    for text in [
        "function @f(i8 signed noundef, ptr noundef, ... noundef) -> i8 signed noundef external\n",
        "abi lowered\n\nfunction @f(i8 signext noundef, ptr byval(size 24, align 8) noundef, ... \
         noundef) -> i8 signext noundef external\n",
        "abi lowered\n\nfunction @f(ptr registers(i64, i64, align 8)) -> noundef external\n",
    ] {
        assert_round_trip(text);
        let arena = Bump::new();
        let module = parse(&arena, text);
        let no_errors: [String; 0] = [];
        assert_eq!(verify(&module, module.profile()), no_errors);
    }
    // Attributes participate in identity, including indirect call signatures.
    let arena = Bump::new();
    let mut module = parse(
        &arena,
        "function @plain(i32) -> i32 external\nfunction @defined(i32 noundef) -> i32 noundef \
         external\n",
    );
    assert_eq!(module.signature_count(), 2);
    let sig = module.function_signature(FuncId::new(1));
    let interned = module.intern(sig);
    assert_eq!(interned, module.function(FuncId::new(1)).signature);
    assert!(module.signature(interned).param_noundef(0));
    assert!(module.signature(interned).result_noundef);
}

#[test]
fn noundef_lookup_handles_sparse_fixed_and_variadic_positions() {
    let arena = Bump::new();
    let module = parse(
        &arena,
        "function @f(i32, i32 noundef, i32, i32 noundef, ... noundef) external\n",
    );
    let signature = module.function_signature(FuncId::new(0));
    for (index, expected) in [
        (0, false),
        (1, true),
        (2, false),
        (3, true),
        (4, true),
        (usize::MAX, true),
    ] {
        assert_eq!(signature.param_noundef(index), expected, "index {index}");
    }
    let plain_variadic = Signature {
        variadic_noundef: false,
        ..signature
    };
    assert!(!plain_variadic.param_noundef(4));
    assert!(!plain_variadic.param_noundef(usize::MAX));
    let unmarked = Signature {
        noundef: &[],
        ..signature
    };
    assert!(!unmarked.param_noundef(0));
    assert!(unmarked.param_noundef(4));
}

#[test]
fn noundef_verifier_preserves_register_diagnostics_with_unordered_aggregates() {
    let arena = Bump::new();
    let mut module = parse(
        &arena,
        "abi lowered\nfunction @f(ptr registers(i64, align 8), ptr byval(size 8, align 8)) \
         external\n",
    );
    let signature = module.function_signature(FuncId::new(0));
    let attributes = [signature.aggregates[1], signature.aggregates[0]];
    let signature = module.intern(Signature {
        aggregates: &attributes,
        noundef: &[0],
        ..signature
    });
    let function = module.declare_function("unordered", signature, Linkage::External);
    let scratch = Bump::new();
    let errors = verify_function(&module, function, Profile::PostAbi, &scratch);
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert!(
        errors
            .iter()
            .any(|error| matches!(error.kind, VerifierErrorKind::Noundef(_)))
    );
    assert!(
        errors
            .iter()
            .any(|error| matches!(error.kind, VerifierErrorKind::ParamAbi { index: 0, .. }))
    );
}

#[test]
fn noundef_verifier_rejects_missing_and_unpassed_positions() {
    let arena = Bump::new();
    let mut module = Module::new(&arena);
    let plain = module.intern_signature(&[Type::I32, Type::Ptr], None, false);
    for (indices, variadic, result) in [
        (&[2][..], false, false),
        (&[0, 0][..], false, false),
        (&[1, 0][..], false, false),
        (&[][..], true, false),
        (&[][..], false, true),
    ] {
        let sig = module.intern(Signature {
            noundef: indices,
            variadic_noundef: variadic,
            result_noundef: result,
            ..module.signature(plain)
        });
        let func = module.declare_function(
            &format!("f{}", module.signature_count()),
            sig,
            Linkage::External,
        );
        let scratch = Bump::new();
        for profile in [Profile::PreAbi, Profile::PostAbi] {
            let errors = verify_function(&module, func, profile, &scratch);
            assert_eq!(errors.len(), 1, "{errors:?}");
            assert!(errors[0].kind.to_string().contains("invalid noundef"));
        }
    }
    let text = "abi lowered\n\nfunction @f(ptr registers(i64, align 8) noundef) external\n";
    let module = parse(&arena, text);
    assert!(verify(&module, Profile::PostAbi)[0].contains("invalid noundef"));
    _ = parse_module(&arena, "function @f(i32 noundef noundef) external\n").unwrap_err();
}
