//! Host calls enforce defined-value contracts before invoking the host and
//! before accepting its return, including open indirect host signatures.

use super::*;

#[test]
fn open_host_return_contract_mismatches_trap_before_host_invocation() {
    for (declaration, call_signature) in [
        ("(...) -> i32", "() -> i32 noundef"),
        ("(...) -> i32 noundef", "() -> i32"),
    ] {
        let (result, called) = indirect_poison_host_return(declaration, call_signature);
        assert!(!called, "host invoked; result: {result:?}");
        assert!(
            matches!(
                result,
                Err(Trap::UndefinedBehavior(UbKind::SignatureMismatch, _))
            ),
            "{result:?}"
        );
    }
}

#[test]
fn open_call_return_contract_mismatches_trap_before_host_invocation() {
    for (declaration, call_signature) in [
        ("() -> i32", "(...) -> i32 noundef"),
        ("() -> i32 noundef", "(...) -> i32"),
    ] {
        let (result, called) = indirect_poison_host_return(declaration, call_signature);
        assert!(!called, "host invoked; result: {result:?}");
        assert!(
            matches!(
                result,
                Err(Trap::UndefinedBehavior(UbKind::SignatureMismatch, _))
            ),
            "{result:?}"
        );
    }
}

#[test]
fn matching_open_indirect_return_contracts_accept_or_trap_unused_poison() {
    for (declaration_params, call_params) in [("...", ""), ("", "...")] {
        for attribute in ["", " noundef"] {
            let declaration = format!("({declaration_params}) -> i32{attribute}");
            let call_signature = format!("({call_params}) -> i32{attribute}");
            let (result, called) = indirect_poison_host_return(&declaration, &call_signature);
            assert!(called, "host not invoked; result: {result:?}");
            if attribute.is_empty() {
                assert_eq!(
                    result.unwrap().termination,
                    Termination::Returned(Some(int(0)))
                );
            } else {
                assert!(
                    matches!(result, Err(Trap::UndefinedBehavior(UbKind::Noundef, _))),
                    "{result:?}"
                );
            }
        }
    }
}

#[test]
fn noundef_host_arguments_trap_before_the_host_is_invoked() {
    struct IgnoringHost(bool);
    impl Host for IgnoringHost {
        fn call(&mut self, _: HostCall<'_>, _: &mut Memory<'_>) -> Result<HostReturn, Fault> {
            self.0 = true;
            Ok(HostReturn::Value(Some(int(0))))
        }
    }
    for (declaration, call) in [
        ("i32 noundef", "call @host(v0)"),
        ("...", "call_indirect v1(v0) : (i32 noundef) -> i32"),
    ] {
        let text = format!(
            "function @host({declaration}) -> i32 external\n\nfunction @main() -> i32 external \
             {{\nblock0:\nv0 = poison.i32\nv1 = func_addr @host\nv2 = {call}\nreturn v2\n}}\n"
        );
        let arena = Bump::new();
        let module = crate::ir::parse_module(&arena, &text).unwrap();
        let mut host = IgnoringHost(false);
        assert!(matches!(
            run(&module, "main", &[], &mut host),
            Err(Trap::UndefinedBehavior(UbKind::Noundef, _))
        ));
        assert!(!host.0);
    }
}

#[test]
fn noundef_host_returns_trap_even_when_unused() {
    struct PoisonHost;
    impl Host for PoisonHost {
        fn call(&mut self, _: HostCall<'_>, _: &mut Memory<'_>) -> Result<HostReturn, Fault> {
            Ok(HostReturn::Value(Some(RuntimeValue::Poison)))
        }
    }
    for attribute in ["", " noundef"] {
        let text = format!(
            "function @host() -> i32{attribute} external\n\nfunction @main() -> i32 external \
             {{\nblock0:\nv0 = call @host()\nv1 = iconst.i32 0\nreturn v1\n}}\n"
        );
        let arena = Bump::new();
        let module = crate::ir::parse_module(&arena, &text).unwrap();
        let result = run(&module, "main", &[], &mut PoisonHost);
        if attribute.is_empty() {
            assert_eq!(
                result.unwrap().termination,
                Termination::Returned(Some(int(0)))
            );
        } else {
            assert!(matches!(
                result,
                Err(Trap::UndefinedBehavior(UbKind::Noundef, _))
            ));
        }
    }
}

/// Calls the poison-returning host indirectly, ignoring the call's result.
fn indirect_poison_host_return(
    declaration: &str,
    call_signature: &str,
) -> (Result<Outcome, Trap>, bool) {
    struct PoisonHost(bool);
    impl Host for PoisonHost {
        fn call(&mut self, _: HostCall<'_>, _: &mut Memory<'_>) -> Result<HostReturn, Fault> {
            self.0 = true;
            Ok(HostReturn::Value(Some(RuntimeValue::Poison)))
        }
    }
    let text = format!(
        "function @host{declaration} external\n\nfunction @main() -> i32 external {{\nblock0:\nv0 \
         = func_addr @host\nv1 = call_indirect v0() : {call_signature}\nv2 = iconst.i32 0\nreturn \
         v2\n}}\n"
    );
    let arena = Bump::new();
    let module = crate::ir::parse_module(&arena, &text).unwrap();
    let mut host = PoisonHost(false);
    let result = run(&module, "main", &[], &mut host);
    (result, host.0)
}
