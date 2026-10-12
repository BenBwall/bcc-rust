//! Host calls enforce defined-value contracts before invoking the host and
//! before accepting its return, including open indirect host signatures.

use super::*;

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
