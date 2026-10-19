//! Register allocation: calls, pressure, fixed registers and copies, and
//! the allocation checker. `rewrite` runs the checker on every function in
//! debug builds; these tests check where values land, and that the checker
//! rejects allocations broken on purpose.

use std::fmt::Write as _;

use super::{
    assembly,
    check_corrupted,
    function_text,
    with_allocation,
};
use crate::{
    backend::native::{
        conv::CallConv,
        mir::{
            MInst,
            Operand,
            PReg,
            Reg,
            VReg,
        },
        regalloc::{
            ErrorKind,
            Location,
        },
    },
    ir::Entity,
    target::Target,
};

#[test]
fn values_live_across_calls_avoid_caller_saved_registers() {
    let ir = "abi lowered
function @g() -> i64 external
function @f(i64) -> i64 external {
block0(v0: i64):
    v1 = call @g()
    v2 = call @g()
    v3 = iadd.i64 v0, v1
    v4 = iadd.i64 v3, v2
    return v4
}
";
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        let conv = CallConv::of(target);
        with_allocation(ir, "f", target, |_, func, allocation| {
            // The parameter's register: the destination of the first copy
            // out of the argument register.
            let MInst::Mov {
                dst: Reg::Virt(param),
                ..
            } = func.blocks[0].insts[0]
            else {
                panic!("the entry begins with the parameter copy");
            };
            let call = func.blocks[0]
                .insts
                .iter()
                .position(|inst| matches!(inst, MInst::Call { .. }))
                .expect("the entry calls");
            let at_clobber = u32::try_from(2 * call + 1).unwrap();
            match allocation.location(param, at_clobber) {
                | Location::Reg(register) => assert!(
                    conv.callee_saved.contains(register),
                    "{target:?}: the parameter lives across calls in {register:?}"
                ),
                | Location::Spill(_) => {},
                | Location::None => panic!("the parameter has no location"),
            }
        });
    }
}

#[test]
fn pressure_beyond_the_registers_spills() {
    // Twenty values loaded before any is combined: more than the thirteen
    // allocatable registers.
    let mut ir = String::from(
        "abi lowered
function @f(ptr) -> i64 external {
block0(v0: ptr):
",
    );
    for index in 0..20 {
        writeln!(
            ir,
            "    v{} = iconst.i64 {}\n    v{} = ptr_add v0, v{}\n    v{} = load.i64 v{}, align 8",
            3 * index + 1,
            8 * index,
            3 * index + 2,
            3 * index + 1,
            3 * index + 3,
            3 * index + 2,
        )
        .unwrap();
    }
    let mut sum = 3;
    for (next, index) in (61..).zip(1..20) {
        writeln!(ir, "    v{next} = iadd.i64 v{sum}, v{}", 3 * index + 3).unwrap();
        sum = next;
    }
    writeln!(ir, "    return v{sum}\n}}").unwrap();
    with_allocation(&ir, "f", Target::LinuxGnu, |_, func, allocation| {
        assert!(allocation.spill_slots > 0, "{ir}");
        assert_eq!(allocation.fallbacks, 0, "every use gets a register");
        let in_registers = (0..func.vreg_count())
            .filter(|&index| {
                allocation
                    .locations(VReg::new(index))
                    .any(|location| matches!(location, Location::Reg(_)))
            })
            .count();
        assert!(in_registers > 10, "most values still get registers");
    });
    let text = assembly(&ir, Target::LinuxGnu);
    let f = function_text(&text, "f");
    assert!(
        f.contains("(%rsp)"),
        "spilled values live in the frame:\n{f}"
    );
}

#[test]
fn a_divisor_never_shares_the_dividend_registers() {
    let ir = "abi lowered
function @f(i64, i64) -> i64 external {
block0(v0: i64, v1: i64):
    v2 = sdiv.i64 v0, v1
    v3 = srem.i64 v0, v1
    v4 = iadd.i64 v2, v3
    return v4
}
";
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        with_allocation(ir, "f", target, |_, func, allocation| {
            for (index, inst) in func.blocks[0].insts.iter().enumerate() {
                if let MInst::Div {
                    divisor: Reg::Virt(divisor),
                    ..
                } = *inst
                {
                    let position = u32::try_from(2 * index).unwrap();
                    let Location::Reg(register) = allocation.location(divisor, position) else {
                        continue;
                    };
                    assert!(
                        register != PReg::Rax && register != PReg::Rdx,
                        "{target:?}: the divisor is in {register:?}"
                    );
                }
            }
        });
    }
}

#[test]
fn swapped_block_arguments_go_through_a_temporary() {
    let ir = "abi lowered
function @f(i32, i32, i32) -> i32 external {
block0(v0: i32, v1: i32, v2: i32):
    jump block1(v0, v1, v2)
block1(v3: i32, v4: i32, v5: i32):
    v6 = iconst.i32 0
    v7 = icmp.i32 sgt v5, v6
    v8 = iconst.i32 1
    v9 = isub.i32 v5, v8
    brif v7, block2, block3
block2:
    jump block1(v4, v3, v9)
block3:
    v10 = iconst.i32 10
    v11 = imul.i32 v3, v10
    v12 = iadd.i32 v11, v4
    return v12
}
";
    with_allocation(ir, "f", Target::LinuxGnu, |_, func, _| {
        // The back edge's block copies three values, one of them twice
        // through the temporary that breaks the swap cycle.
        let copies = func
            .blocks
            .iter()
            .map(|block| {
                block
                    .insts
                    .iter()
                    .filter(|inst| {
                        matches!(
                            inst,
                            MInst::Mov {
                                dst: Reg::Virt(_),
                                src: Operand::Reg(Reg::Virt(_)),
                                ..
                            }
                        )
                    })
                    .count()
            })
            .max()
            .unwrap();
        assert!(copies >= 4, "{copies}");
    });
}

/// A function whose parameter lives across two calls.
const ACROSS_CALLS: &str = "abi lowered
function @g() -> i64 external
function @f(i64) -> i64 external {
block0(v0: i64):
    v1 = call @g()
    v2 = call @g()
    v3 = iadd.i64 v0, v1
    v4 = iadd.i64 v3, v2
    return v4
}
";

#[test]
fn the_checker_accepts_the_allocator_s_work() {
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        check_corrupted(ACROSS_CALLS, "f", target, |_, _| {}).unwrap();
    }
}

#[test]
fn the_checker_rejects_a_value_left_in_a_clobbered_register() {
    let result = check_corrupted(ACROSS_CALLS, "f", Target::LinuxGnu, |func, allocation| {
        let MInst::Mov {
            dst: Reg::Virt(param),
            ..
        } = func.blocks[0].insts[0]
        else {
            panic!("the entry begins with the parameter copy");
        };
        allocation.force(param, Location::Reg(PReg::Rcx));
    });
    let error = result.expect_err("%rcx does not survive the call");
    assert!(matches!(error.kind, ErrorKind::Missing { .. }), "{error}");
}

#[test]
fn the_checker_rejects_two_live_values_in_one_register() {
    let ir = "abi lowered
function @f(i64, i64) -> i64 external {
block0(v0: i64, v1: i64):
    v2 = imul.i64 v0, v1
    v3 = iadd.i64 v2, v0
    v4 = isub.i64 v3, v1
    return v4
}
";
    let result = check_corrupted(ir, "f", Target::LinuxGnu, |func, allocation| {
        // Put every virtual register in %rbx.
        for index in 0..func.vreg_count() {
            allocation.force(VReg::new(index), Location::Reg(PReg::Rbx));
        }
    });
    let error = result.expect_err("the parameters overlap");
    assert!(matches!(error.kind, ErrorKind::Missing { .. }), "{error}");
}

#[test]
fn the_checker_rejects_a_value_in_the_wrong_class() {
    let result = check_corrupted(ACROSS_CALLS, "f", Target::LinuxGnu, |func, allocation| {
        let MInst::Mov {
            dst: Reg::Virt(param),
            ..
        } = func.blocks[0].insts[0]
        else {
            panic!("the entry begins with the parameter copy");
        };
        allocation.force(param, Location::Reg(PReg::Xmm9));
    });
    let error = result.expect_err("a general value cannot live in %xmm9");
    assert!(matches!(error.kind, ErrorKind::Shape), "{error}");
}

#[test]
fn the_checker_rejects_a_spill_slot_shared_by_live_values() {
    let ir = "abi lowered
function @f(i64, i64) -> i64 external {
block0(v0: i64, v1: i64):
    v2 = iadd.i64 v0, v1
    v3 = isub.i64 v2, v0
    v4 = imul.i64 v3, v1
    return v4
}
";
    let result = check_corrupted(ir, "f", Target::LinuxGnu, |func, allocation| {
        for index in 0..func.vreg_count() {
            allocation.force(VReg::new(index), Location::Spill(0));
        }
        allocation.spill_slots = 1;
    });
    let error = result.expect_err("the values overwrite each other's slot");
    assert!(matches!(error.kind, ErrorKind::Missing { .. }), "{error}");
}

#[test]
fn a_loop_counter_is_updated_in_place() {
    // `i + 1` is `mov t, i; add t, 1` and the back edge copies `t` into
    // `i`: coalescing gives all three one register, so the loop has no
    // register-to-register moves.
    let ir = "abi lowered
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 0
    jump block1(v1, v1)
block1(v2: i32, v3: i32):
    v4 = icmp.i32 slt v2, v0
    brif v4, block2, block3
block2:
    v5 = iadd.i32 v3, v2
    v6 = iconst.i32 1
    v7 = iadd.i32 v2, v6
    jump block1(v7, v5)
block3:
    return v3
}
";
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        let text = assembly(ir, target);
        let f = function_text(&text, "f");
        let moves = f
            .lines()
            .filter(|line| {
                let mut parts = line.split_whitespace();
                parts.next().is_some_and(|op| op.starts_with("mov"))
                    && parts.next().is_some_and(|operands| {
                        operands.starts_with('%') && !operands.contains('(')
                    })
                    && parts.next().is_some_and(|operand| operand.starts_with('%'))
            })
            .count();
        // Only the result's copy into %eax may remain.
        assert!(moves <= 1, "{target:?}:\n{f}");
        assert!(f.contains("addl\t$1, %"), "{target:?}:\n{f}");
        // The back edge tests the condition itself.
        assert!(!f.contains("\tjmp\t"), "{target:?}:\n{f}");
    }
}

#[test]
fn pressure_spills_only_around_the_values_needed_later() {
    // Thirty values are loaded, then summed in reverse: every use still
    // gets a register, by reloading the spilled values.
    let mut ir = String::from(
        "abi lowered
function @f(ptr) -> i64 external {
block0(v0: ptr):
",
    );
    for index in 0..30 {
        writeln!(
            ir,
            "    v{} = iconst.i64 {}\n    v{} = ptr_add v0, v{}\n    v{} = load.i64 v{}, align 8",
            3 * index + 1,
            8 * index,
            3 * index + 2,
            3 * index + 1,
            3 * index + 3,
            3 * index + 2,
        )
        .unwrap();
    }
    let mut sum = 3 * 29 + 3;
    for (next, index) in (91..).zip((0..29).rev()) {
        writeln!(ir, "    v{next} = iadd.i64 v{sum}, v{}", 3 * index + 3).unwrap();
        sum = next;
    }
    writeln!(ir, "    return v{sum}\n}}").unwrap();
    with_allocation(&ir, "f", Target::LinuxGnu, |_, _, allocation| {
        assert!(allocation.spill_slots > 0, "{ir}");
        assert_eq!(allocation.fallbacks, 0, "every use gets a register");
    });
    let text = assembly(&ir, Target::LinuxGnu);
    assert!(
        !function_text(&text, "f").contains("%r11"),
        "no scratch register"
    );
}

#[test]
fn a_loop_whose_exit_comes_first_branches_back_on_its_test() {
    // The exit block lies between the header and the latch, so the copied
    // test cannot fall through; its branch is turned to take the back edge.
    let ir = "abi lowered
function @f(i32) -> i32 external {
block0(v0: i32):
    v1 = iconst.i32 0
    jump block1(v1, v1)
block1(v2: i32, v3: i32):
    v4 = icmp.i32 sge v2, v0
    brif v4, block2, block3
block2:
    return v3
block3:
    v5 = iadd.i32 v3, v2
    v6 = iconst.i32 1
    v7 = iadd.i32 v2, v6
    jump block1(v7, v5)
}
";
    let text = assembly(ir, Target::LinuxGnu);
    let f = function_text(&text, "f");
    // The latch ends in `jl` back to itself, then the exit's `jmp`.
    let latch = f.rfind("\tjl\t").expect(f);
    assert!(f[latch..].contains("\tjmp\t"), "{f}");
}
