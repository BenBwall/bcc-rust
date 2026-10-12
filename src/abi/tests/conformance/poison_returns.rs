//! Partially poison aggregates crossing register arguments and return areas.

use super::*;

#[test]
fn partially_poison_aggregates_survive_both_abi_round_trips() {
    use crate::backend::interpreter::{
        RuntimeValue,
        Termination,
        run_text,
    };

    for (size, scalars, defined, poisoned) in [
        (2, "0: i8, 1: i8", 0, 1),
        (2, "0: i16, 0: i8, 1: i8", 0, 1),
        (8, "0: f32, 4: f32", 4, 0),
        (16, "0: i32, 8: f64", 8, 0),
    ] {
        let layout = format!("size {size}, align 1, [{scalars}]");
        let program = format!(
            "function @echo(ptr result({layout}), ptr aggregate({layout})) internal {{
block0(v0: ptr, v1: ptr):
 v2 = iconst.i64 {size}
 copy v0, v1, v2, align 1
 v3 = iconst.i64 {defined}
 v4 = ptr_add inbounds v1, v3
 v5 = load.i8 v4, align 1
 v6 = ptr_add inbounds v0, v3
 store.i8 v5, v6, align 1
 v7 = iconst.i64 {neighbor}
 v8 = ptr_add inbounds v0, v7
 v9 = poison.i8
 store.i8 v9, v8, align 1
 return
}}
function @main() -> i32 external {{
 slot0 = stack_slot {size}, align 1
 slot1 = stack_slot {size}, align 1
block0:
 v0 = stack_addr slot0
 v1 = stack_addr slot1
 v2 = iconst.i8 0
 v3 = iconst.i64 {size}
 fill v0, v2, v3, align 1
 v4 = iconst.i64 {defined}
 v5 = ptr_add inbounds v0, v4
 v6 = iconst.i8 42
 store.i8 v6, v5, align 1
 v7 = iconst.i64 {poisoned}
 v8 = ptr_add inbounds v0, v7
 v9 = poison.i8
 store.i8 v9, v8, align 1
 call @echo(v1, v0)
 v10 = ptr_add inbounds v1, v4
 v11 = load.i8 v10, align 1
 v12 = icmp.i8 ne v11, v6
 v13 = func_addr @echo
 call_indirect v13(v1, v0) : (ptr result({layout}), ptr aggregate({layout}))
 v14 = load.i8 v10, align 1
 v15 = icmp.i8 ne v14, v6
 v16 = or.i1 v12, v15
 v17 = zext.i32 v16
 return v17
}}
",
            neighbor = defined + 1
        );
        assert_eq!(
            run_text(&program, "main").result.unwrap().termination,
            Termination::Returned(Some(RuntimeValue::Int(0)))
        );
        for target in [Target::WindowsGnu, Target::LinuxGnu] {
            let arena = Bump::new();
            let mut module = parse_module(&arena, &program).unwrap();
            lower_abi(&mut module, target, &Bump::new()).unwrap();
            assert_verified(&module, Profile::PostAbi, &module.to_string());
            assert_eq!(
                run_text(&module.to_string(), "main")
                    .result
                    .unwrap()
                    .termination,
                Termination::Returned(Some(RuntimeValue::Int(0))),
                "{target:?}: {module}"
            );
            assert_conforms("poison-aggregate", target, &module, "");
        }
    }
}
