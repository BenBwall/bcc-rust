//! Register allocation: calls, pressure, fixed registers and copies, and
//! the allocation checker. `rewrite` runs the checker on every function in
//! debug builds; these tests check where values land, and that the checker
//! rejects allocations broken on purpose.

use std::fmt::Write as _;

use super::{
    assembly,
    check_corrupted,
    function_text,
    spilled,
    with_allocation,
};
use crate::{
    backend::native::{
        conv::CallConv,
        mir::{
            Base,
            FrameObject,
            Index,
            MFunction,
            MInst,
            Mem,
            Operand,
            PReg,
            PRegSet,
            Reg,
            VReg,
            Width,
        },
        regalloc::{
            CheckError,
            ErrorKind,
            INSERTED,
            Location,
            Value,
            check,
        },
    },
    ir::Entity,
    target::Target,
    util::bump::Bump,
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

/// Instruction forms with their operands, each in a function whose every
/// value [`spilled`] puts in a spill slot: the ones naming three general
/// registers (an indexed address's base and index and a third), and the
/// others that name two of a class besides fixed registers.
const FORMS: &[(&str, &str)] = &[
    (
        "indexed store",
        "
function @f(ptr, i64, i64) external {
block0(v0: ptr, v1: i64, v2: i64):
    v3 = iconst.i64 8
    v4 = imul.i64 v1, v3
    v5 = ptr_add v0, v4
    store.i64 v2, v5, align 8
    return
}",
    ),
    (
        "narrow indexed store with a displacement",
        "
function @f(ptr, i64, i32) external {
block0(v0: ptr, v1: i64, v2: i32):
    v3 = iconst.i64 4
    v4 = imul.i64 v1, v3
    v5 = ptr_add v0, v4
    v6 = iconst.i64 12
    v7 = ptr_add v5, v6
    store.i32 v2, v7, align 4
    v8 = trunc.i8 v2
    store.i8 v8, v7, align 1
    return
}",
    ),
    (
        "indexed loads",
        "
function @f(ptr, i64) -> i64 external {
block0(v0: ptr, v1: i64):
    v2 = iconst.i64 2
    v3 = imul.i64 v1, v2
    v4 = ptr_add v0, v3
    v5 = load.i16 v4, align 2
    v6 = sext.i64 v5
    v7 = load.i64 v4, align 8
    v8 = iadd.i64 v6, v7
    return v8
}",
    ),
    (
        "indexed address",
        "
function @f(ptr, i64) -> ptr external {
block0(v0: ptr, v1: i64):
    v2 = iconst.i64 8
    v3 = imul.i64 v1, v2
    v4 = ptr_add v0, v3
    return v4
}",
    ),
    (
        "indexed read-modify-write",
        "
function @f(ptr, i64, i64) external {
block0(v0: ptr, v1: i64, v2: i64):
    v3 = iconst.i64 8
    v4 = imul.i64 v1, v3
    v5 = ptr_add v0, v4
    v6 = load.i64 v5, align 8
    v7 = iadd.i64 v6, v2
    store.i64 v7, v5, align 8
    return
}",
    ),
    (
        "indexed SSE store and load",
        "
function @f(ptr, i64, f64) -> f64 external {
block0(v0: ptr, v1: i64, v2: f64):
    v3 = iconst.i64 8
    v4 = imul.i64 v1, v3
    v5 = ptr_add v0, v4
    store.f64 v2, v5, align 8
    v6 = load.f64 v5, align 8
    v7 = fadd.f64 v6, v2
    return v7
}",
    ),
    (
        "indexed i128 store",
        "
function @f(ptr, i64, i64) external {
block0(v0: ptr, v1: i64, v2: i64):
    v3 = iconst.i64 1
    v4 = shl.i64 v1, v3
    v5 = ptr_add v0, v4
    v6 = sext.i128 v2
    store.i128 v6, v5, align 16
    return
}",
    ),
    (
        "shifts by %cl, single and double",
        "
function @f(i64, i64, i64) -> i64 external {
block0(v0: i64, v1: i64, v2: i64):
    v3 = zext.i128 v0
    v4 = zext.i128 v2
    v5 = shl.i128 v3, v4
    v6 = iconst.i128 64
    v7 = lshr.i128 v5, v6
    v8 = trunc.i64 v7
    v9 = ashr.i64 v1, v2
    v10 = iadd.i64 v8, v9
    return v10
}",
    ),
    (
        "division",
        "
function @f(i64, i64, i32, i32) -> i64 external {
block0(v0: i64, v1: i64, v2: i32, v3: i32):
    v4 = sdiv.i64 v0, v1
    v5 = srem.i64 v0, v1
    v6 = udiv.i32 v2, v3
    v7 = zext.i64 v6
    v8 = iadd.i64 v4, v5
    v9 = iadd.i64 v8, v7
    return v9
}",
    ),
    (
        "multiplications and selects",
        "
function @f(i64, i64, i64) -> i64 external {
block0(v0: i64, v1: i64, v2: i64):
    v3 = iconst.i64 1000003
    v4 = imul.i64 v0, v3
    v5 = imul.i64 v4, v1
    v6 = icmp.i64 ult v0, v1
    v7 = select.i64 v6, v5, v2
    return v7
}",
    ),
    (
        "bit operations",
        "
function @f(i64, i64) -> i64 external {
block0(v0: i64, v1: i64):
    v2 = clz.i64 v0
    v3 = ctz.i64 v1
    v4 = bswap.i64 v0
    v5 = iadd.i64 v2, v3
    v6 = xor.i64 v5, v4
    return v6
}",
    ),
    (
        "float compares and conversions",
        "
function @f(f64, f64, i64) -> i64 external {
block0(v0: f64, v1: f64, v2: i64):
    v3 = fcmp.f64 olt v0, v1
    v4 = sitofp.f64 v2
    v5 = fmul.f64 v4, v0
    v6 = fptosi.i64 v5
    v7 = zext.i64 v3
    v8 = iadd.i64 v6, v7
    return v8
}",
    ),
];

#[test]
fn every_operand_of_every_form_can_be_spilled() {
    for (form, function) in FORMS {
        let ir = format!("abi lowered{function}\n");
        for target in [Target::LinuxGnu, Target::WindowsGnu] {
            // The checker proves the rewrite in debug builds; a form that
            // needed a third scratch register would panic.
            let text = spilled(&ir, target);
            assert!(
                function_text(&text, "f").contains("(%rsp)"),
                "{form}, {target:?}: every value is in a slot:\n{text}"
            );
        }
    }
}

#[test]
fn an_indexed_store_of_three_spilled_registers_computes_its_address_first() {
    // Review of stage 4, finding 1: the base, the index and the stored
    // value all in spill slots. The address goes into %r11 with a `lea`, and
    // %r10 is then free for the value.
    let ir = format!("abi lowered{}\n", FORMS[0].1);
    let text = spilled(&ir, Target::LinuxGnu);
    let f = function_text(&text, "f");
    let lea = f
        .find("\tleaq\t(%r11,%r10,8), %r11\n")
        .unwrap_or_else(|| panic!("{f}"));
    assert!(
        f[lea..].contains("(%rsp), %r10\n\tmovq\t%r10, (%r11)\n"),
        "the value is reloaded into %r10 and stored through %r11:\n{f}"
    );
    // A narrow store keeps its displacement on the instruction.
    let ir = format!("abi lowered{}\n", FORMS[1].1);
    let text = spilled(&ir, Target::LinuxGnu);
    let f = function_text(&text, "f");
    assert!(f.contains("\tleaq\t(%r11,%r10,4), %r11\n"), "{f}");
    assert!(f.contains("\tmovl\t%r10d, 12(%r11)\n"), "{f}");
    assert!(f.contains("\tmovb\t%r10b, 12(%r11)\n"), "{f}");
}

#[test]
fn an_indexed_load_into_a_spilled_register_reuses_a_scratch_register() {
    // Two spilled reads and a spilled result: the load writes the register
    // its base came in, after reading it.
    let ir = format!("abi lowered{}\n", FORMS[2].1);
    let text = spilled(&ir, Target::LinuxGnu);
    let f = function_text(&text, "f");
    assert_eq!(f.matches("(%r11,%r10,2), %r11").count(), 2, "{f}");
    assert!(!f.contains("leaq"), "two scratch registers suffice:\n{f}");
}

/// Checks the program `v0..v2 = %rdi, %rsi, %rdx; store v2, 16(v0, v1, 8)`
/// against its rewrite with all three spilled, the address computed into
/// %r11 first, after `mutate` changed the rewrite and its origins. With
/// `rebase`, the program sets `v0 = %rcx` just before the store, and the
/// rewrite does so between the `lea` and the store.
fn check_indexed_store(
    rebase: bool,
    mutate: impl FnOnce(&mut Vec<MInst>, &mut Vec<u32>),
) -> Result<(), Box<CheckError>> {
    let arena = Bump::new();
    let mut func = MFunction::new_in(&arena);
    let block = func.block(&arena);
    let [base, index, value] = [func.vreg(), func.vreg(), func.vreg()];
    let copy = |dst: VReg, src: PReg| MInst::Mov {
        width: Width::B64,
        dst:   Reg::Virt(dst),
        src:   Operand::Reg(Reg::Phys(src)),
    };
    let indexed = |base: Reg, index: Reg, scale: u8, disp: i32| Mem {
        base: Base::Reg(base),
        index: Some(Index { reg: index, scale }),
        disp,
    };
    let mut original = vec![
        copy(base, PReg::Rdi),
        copy(index, PReg::Rsi),
        copy(value, PReg::Rdx),
        MInst::Store {
            width: Width::B64,
            dst:   indexed(Reg::Virt(base), Reg::Virt(index), 8, 16),
            src:   Operand::Reg(Reg::Virt(value)),
        },
        MInst::Ret {
            uses: PRegSet::EMPTY,
        },
    ];
    let slot = |slot: u32| Mem::new(Base::Frame(FrameObject::Spill(slot)), 0);
    let spill = |slot_index: u32, src: PReg| MInst::Store {
        width: Width::B64,
        dst:   slot(slot_index),
        src:   Operand::Reg(Reg::Phys(src)),
    };
    let reload = |dst: PReg, slot_index: u32| MInst::Mov {
        width: Width::B64,
        dst:   Reg::Phys(dst),
        src:   Operand::Mem(slot(slot_index)),
    };
    let (r10, r11) = (Reg::Phys(PReg::R10), Reg::Phys(PReg::R11));
    let mut rewritten = vec![
        spill(0, PReg::Rdi),
        spill(1, PReg::Rsi),
        spill(2, PReg::Rdx),
        reload(PReg::R11, 0),
        reload(PReg::R10, 1),
        MInst::Lea {
            dst:  r11,
            addr: indexed(r11, r10, 8, 0),
        },
        reload(PReg::R10, 2),
        MInst::Store {
            width: Width::B64,
            dst:   Mem::new(Base::Reg(r11), 16),
            src:   Operand::Reg(r10),
        },
        MInst::Ret {
            uses: PRegSet::EMPTY,
        },
    ];
    let mut origins = vec![0, 1, 2, INSERTED, INSERTED, INSERTED, INSERTED, 3, 4];
    if rebase {
        original.insert(3, copy(base, PReg::Rcx));
        rewritten.insert(6, spill(0, PReg::Rcx));
        origins = vec![0, 1, 2, INSERTED, INSERTED, INSERTED, 3, INSERTED, 4, 5];
    }
    mutate(&mut rewritten, &mut origins);
    func.blocks[block.index()].insts.extend(rewritten);
    check(&func, &[&original[..]], &[&origins[..]], &arena).map_err(|error| Box::new(*error))
}

#[test]
fn the_checker_accepts_an_address_computed_ahead() {
    check_indexed_store(false, |_, _| {}).unwrap();
}

#[test]
fn the_checker_rejects_an_address_of_the_wrong_values() {
    // Base and index swapped: the address of other values.
    let error = check_indexed_store(false, |rewritten, _| {
        rewritten[3] = MInst::Mov {
            width: Width::B64,
            dst:   Reg::Phys(PReg::R11),
            src:   Operand::Mem(Mem::new(Base::Frame(FrameObject::Spill(1)), 0)),
        };
        rewritten[4] = MInst::Mov {
            width: Width::B64,
            dst:   Reg::Phys(PReg::R10),
            src:   Operand::Mem(Mem::new(Base::Frame(FrameObject::Spill(0)), 0)),
        };
    })
    .expect_err("the index is not the base");
    assert!(
        matches!(
            error.kind,
            ErrorKind::Missing {
                value: Value::Address { .. },
                ..
            }
        ),
        "{error}"
    );
    // Another scale.
    let error = check_indexed_store(false, |rewritten, _| {
        let MInst::Lea { addr, .. } = &mut rewritten[5] else {
            unreachable!()
        };
        addr.index = addr.index.map(|index| Index { scale: 4, ..index });
    })
    .expect_err("the scale differs");
    assert!(matches!(error.kind, ErrorKind::Missing { .. }), "{error}");
    // Another displacement.
    let error = check_indexed_store(false, |rewritten, _| {
        let MInst::Store { dst, .. } = &mut rewritten[7] else {
            unreachable!()
        };
        dst.disp = 8;
    })
    .expect_err("the displacement differs");
    assert!(matches!(error.kind, ErrorKind::Shape), "{error}");
}

#[test]
fn the_checker_rejects_an_address_whose_base_changed() {
    // The program sets the base again between the `lea` and the store, so
    // the address in %r11 is of its old value.
    let error = check_indexed_store(true, |_, _| {}).expect_err("the address is stale");
    assert!(
        matches!(
            error.kind,
            ErrorKind::Missing {
                value: Value::Address { .. },
                ..
            }
        ),
        "{error}"
    );
    // Computed after the new base is in its slot, it is the right one.
    check_indexed_store(true, |rewritten, origins| {
        let rebase = rewritten.remove(6);
        rewritten.insert(3, rebase);
        let origin = origins.remove(6);
        origins.insert(3, origin);
    })
    .unwrap();
}

#[test]
fn an_inserted_lea_with_a_displacement_is_not_a_move() {
    let error = check_indexed_store(false, |rewritten, _| {
        let MInst::Lea { addr, .. } = &mut rewritten[5] else {
            unreachable!()
        };
        addr.disp = 16;
    })
    .expect_err("only a plain sum is an address");
    assert!(matches!(error.kind, ErrorKind::NotAMove), "{error}");
}
