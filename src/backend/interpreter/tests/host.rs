//! The default host: output, formatting, the heap, and ending a program;
//! and a custom host.

use super::*;
use crate::{
    ir::{
        Align,
        Block,
        Entity,
        Symbol,
        Type,
        parse_module,
    },
    target::Target,
    util::bump::Bump,
};

/// The C library functions the tests declare.
const DECLARATIONS: &str = "\
function @printf(ptr, ...) -> i32 external
function @putchar(i32) -> i32 external
function @puts(ptr) -> i32 external
function @malloc(i64) -> ptr external
function @calloc(i64, i64) -> ptr external
function @free(ptr) external
function @memcpy(ptr, ptr, i64) -> ptr external
function @memset(ptr, i32, i64) -> ptr external
function @strlen(ptr) -> i64 external
function @exit(i32) external
function @abort() external
function @memmove(ptr, ptr, i64) -> ptr external
function @memcmp(ptr, ptr, i64) -> i32 external
function @strcmp(ptr, ptr) -> i32 external
function @strncmp(ptr, ptr, i64) -> i32 external
function @strcpy(ptr, ptr) -> ptr external
function @sprintf(ptr, ptr, ...) -> i32 external
function @abs(i32) -> i32 external
function @fabs(f64) -> f64 external
function @floor(f64) -> f64 external
";

/// Runs `@main`, whose body is `body`, after `globals` and the
/// declarations.
fn run_main(globals: &str, body: &str) -> TextRun {
    let text = format!(
        "{globals}\n{DECLARATIONS}\nfunction @main() -> i32 external {{\n    slot0 = stack_slot \
         16, align 8\nblock0:\n{body}}}\n"
    );
    run_text(&text, "main")
}

#[test]
fn torture_string_comparisons_use_unsigned_bytes_and_stop_at_the_limit() {
    let globals = "global @a internal constant size 3, align 1 = bytes \"ff0061\"\nglobal @b \
                   internal constant size 3, align 1 = bytes \"010062\"\n";
    let body = "    v0 = global_addr @a\n    v1 = global_addr @b\nv2 = iconst.i64 1\n    v3 = \
                call @strncmp(v0, v1, v2)\nv4 = call @strcmp(v0, v1)\n    v5 = iconst.i64 3\nv6 = \
                call @memcmp(v0, v1, v5)\n    v7 = iconst.i32 254\nv8 = icmp.i32 eq v3, v7\n    \
                v9 = icmp.i32 eq v4, v7\nv10 = icmp.i32 eq v6, v7\n    v11 = and.i1 v8, v9\nv12 = \
                and.i1 v11, v10\n    v13 = zext.i32 v12\n    return v13\n";
    assert_eq!(
        run_main(globals, body).result.unwrap().termination,
        Termination::Returned(Some(int(1)))
    );
    // A nonterminated one-byte array is valid for strncmp with n == 1.
    let run = run_main(
        "global @byte internal constant size 1, align 1 = bytes \"61\"\n",
        "    v0 = global_addr @byte\n    v1 = iconst.i64 1\nv2 = call @strncmp(v0, v0, v1)\n    \
         return v2\n",
    );
    assert_eq!(
        run.result.unwrap().termination,
        Termination::Returned(Some(int(0)))
    );
}

#[test]
fn torture_sprintf_writes_a_terminated_string_without_stdout() {
    let run = run_main(
        &c_string("format", "%d-%s"),
        "    v0 = stack_addr slot0\n    v1 = global_addr @format\nv2 = iconst.i32 42\n    v3 = \
         call @sprintf(v0, v1, v2, v1)\nv4 = call @strlen(v0)\n    v5 = trunc.i32 v4\nv6 = \
         icmp.i32 eq v3, v5\n    v7 = zext.i32 v6\n    return v7\n",
    );
    assert_eq!(run.output, "");
    assert_eq!(
        run.result.unwrap().termination,
        Termination::Returned(Some(int(1)))
    );
}

#[test]
fn torture_strcpy_and_overlapping_memmove_preserve_the_bytes() {
    let run = run_main(
        &c_string("word", "abc"),
        "    v0 = stack_addr slot0\n    v1 = global_addr @word\nv2 = call @strcpy(v0, v1)\n    v3 \
         = iconst.i64 1\nv4 = ptr_add inbounds v0, v3\n    v5 = iconst.i64 4\nv6 = call \
         @memmove(v4, v0, v5)\n    v7 = call @strcmp(v4, v1)\n    return v7\n",
    );
    assert_eq!(
        run.result.unwrap().termination,
        Termination::Returned(Some(int(0)))
    );
}

#[test]
fn printf_formats_each_conversion() {
    let globals = [
        c_string(
            "format",
            "%d %i %u %x %X %c %s %ld %lld %lu %llu %% [%5d] [%-4d] [%04d] [%3s] [%hhd]\n",
        ),
        c_string("word", "ok"),
    ]
    .concat();
    let body = "\
    v0 = global_addr @format
    v1 = iconst.i32 -5
    v2 = iconst.i32 255
    v3 = iconst.i32 65
    v4 = global_addr @word
    v5 = iconst.i64 -9000000000
    v6 = iconst.i64 -1
    v7 = iconst.i32 300
    v8 = call @printf(v0, v1, v1, v1, v2, v2, v3, v4, v5, v5, v6, v6, v7, v1, v1, v4, v7)
    return v8
";
    let run = run_main(&globals, body);
    let expected = "-5 -5 4294967291 ff FF A ok -9000000000 -9000000000 18446744073709551615 \
                    18446744073709551615 % [  300] [-5  ] [-005] [ ok] [44]\n";
    assert_eq!(run.output, expected);
    assert_eq!(
        run.result.unwrap().termination,
        Termination::Returned(Some(int(expected.len() as u128)))
    );
}

/// Precisions, `*` widths, the sign flags and the floating conversions
/// print as glibc prints them, with halfway cases rounded to even; `%.Ns`
/// reads only `N` bytes of an unterminated array.
#[test]
fn printf_formats_precisions_and_floating_conversions() {
    let globals = [c_string(
        "format",
        "[%.3d] [%+d] [% d] [%#o] [%#x] [%.0d] [%*d] [%-*d] [%.*f] [%.2s] [%.0f] [%.0f] [%e] \
         [%.3E] [%g] [%g] [%#g] [%G] [%a] [%.1a] [%La] [%.3Lf] [%Lg] [%010.2f] [%+.1e] [%p]\n",
    )]
    .concat()
        + "global @bytes internal constant size 2, align 1 = bytes \"6869\"\n";
    let body = "\
    v0 = global_addr @format
    v1 = iconst.i32 7
    v2 = iconst.i32 5
    v3 = iconst.i32 0
    v4 = iconst.i32 255
    v5 = iconst.i32 6
    v6 = iconst.i32 42
    v7 = iconst.i32 2
    v8 = fconst.f64 0x400921F9F01B866E
    v9 = global_addr @bytes
    v10 = fconst.f64 0x3FE0000000000000
    v11 = fconst.f64 0x4004000000000000
    v12 = fconst.f64 0x3EE9E3779B97F4A8
    v13 = fconst.f64 0x412E848000000000
    v14 = fconst.f80 0x3FFDAAAAAAAAAAAAAAAB
    v15 = fconst.f64 0xC059000000000000
    v16 = iconst.i64 0
    v17 = inttoptr.ptr v16
    v18 = call @printf(v0, v1, v2, v2, v3, v4, v3, v5, v6, v5, v6, v7, v8, v9, v10, v11, v8, v8, \
                v12, v13, v8, v12, v8, v8, v14, v14, v14, v15, v15, v17)
    return v18
";
    let run = run_main(&globals, body);
    let expected = "[007] [+5] [ 5] [0] [0xff] [] [    42] [42    ] [3.14] [hi] [0] [2] \
                    [3.141590e+00] [3.142E+00] [1.23446e-05] [1e+06] [3.14159] [1.23446E-05] \
                    [0x1.921f9f01b866ep+1] [0x1.9p+1] [0xa.aaaaaaaaaaaaaabp-5] [0.333] [0.333333] \
                    [-000100.00] [-1.0e+02] [(nil)]\n";
    assert_eq!(run.output, expected);
}

/// `%ld` reads a `long`, which is 32 bits on Windows: a Windows program
/// passes `-5L` as an `i32`, and `%lld` still reads 64 bits.
#[test]
fn printf_reads_long_at_the_targets_width() {
    let text = format!(
        "{}\n{DECLARATIONS}\nfunction @main() -> i32 external {{\nblock0:
    v0 = global_addr @format
    v1 = iconst.i32 -5
    v2 = iconst.i64 -9000000000
    v3 = call @printf(v0, v1, v1, v2)
    return v3
}}\n",
        c_string("format", "%ld %lu %lld")
    );
    let arena = Bump::new();
    let module = parse_module(&arena, &text).unwrap();
    for (target, expected) in [
        (Target::WindowsGnu, "-5 4294967291 -9000000000"),
        (Target::WindowsMsvc, "-5 4294967291 -9000000000"),
    ] {
        let mut host = DefaultHost::for_target(&arena, target);
        _ = run(&module, "main", &[], &mut host).unwrap();
        assert_eq!(host.output(), expected.as_bytes(), "{target:?}");
    }
    let mut host = DefaultHost::for_target(&arena, Target::LinuxGnu);
    let linux = text.replace("iconst.i32 -5", "iconst.i64 -5");
    let module = parse_module(&arena, &linux).unwrap();
    _ = run(&module, "main", &[], &mut host).unwrap();
    assert_eq!(
        host.output(),
        b"-5 18446744073709551611 -9000000000".as_slice()
    );
}

#[test]
fn putchar_and_puts_write_output() {
    let body = "\
    v0 = iconst.i32 104
    v1 = call @putchar(v0)
    v2 = global_addr @line
    v3 = call @puts(v2)
    v4 = call @strlen(v2)
    v5 = trunc.i32 v4
    return v5
";
    let run = run_main(&c_string("line", "ello"), body);
    assert_eq!(run.output, "hello\n");
    assert_eq!(
        run.result.unwrap().termination,
        Termination::Returned(Some(int(4)))
    );
}

#[test]
fn the_heap_allocates_copies_and_frees() {
    let body = "\
    v0 = iconst.i64 8
    v1 = call @malloc(v0)
    v2 = iconst.i32 7
    v3 = call @memset(v1, v2, v0)
    v4 = iconst.i64 2
    v5 = call @calloc(v4, v4)
    v6 = call @memcpy(v5, v1, v4)
    v7 = load.i32 v5, align 4
    call @free(v1)
    call @free(v5)
    v8 = null
    call @free(v8)
    return v7
";
    let run = run_main("", body);
    assert_eq!(
        run.result.unwrap().termination,
        Termination::Returned(Some(int(0x0707)))
    );
}

#[test]
fn heap_misuse_is_undefined() {
    let misuse = |body: &str| match run_main("", body).result {
        | Err(Trap::UndefinedBehavior(kind, _)) => kind,
        | other => panic!("{other:?}"),
    };
    let use_after_free = "\
    v0 = iconst.i64 4
    v1 = call @malloc(v0)
    call @free(v1)
    v2 = load.i32 v1, align 4
    return v2
";
    assert_eq!(misuse(use_after_free), UbKind::DanglingPointer);
    let double_free = "\
    v0 = iconst.i64 4
    v1 = call @malloc(v0)
    call @free(v1)
    call @free(v1)
    v2 = iconst.i32 0
    return v2
";
    assert_eq!(misuse(double_free), UbKind::InvalidFree);
    let stack_free = "\
    v0 = stack_addr slot0
    call @free(v0)
    v1 = iconst.i32 0
    return v1
";
    assert_eq!(misuse(stack_free), UbKind::InvalidFree);
    let uninitialized = "\
    v0 = iconst.i64 4
    v1 = call @malloc(v0)
    v2 = load.i32 v1, align 4
    v3 = iconst.i32 0
    v4 = icmp.i32 eq v2, v3
    brif v4, block1, block1
block1:
    return v3
";
    assert_eq!(misuse(uninitialized), UbKind::BranchOnPoison);
    let missing_argument = "\
    v0 = global_addr @format
    v1 = call @printf(v0)
    return v1
";
    let text = c_string("format", "%d");
    match run_main(&text, missing_argument).result {
        | Err(Trap::UndefinedBehavior(UbKind::MissingFormatArgument, _)) => {},
        | other => panic!("{other:?}"),
    }
}

#[test]
fn oversized_allocations_return_null() {
    let body = "\
    v0 = iconst.i64 -1
    v1 = call @malloc(v0)
    v2 = ptrtoint.i32 v1
    return v2
";
    assert_eq!(
        run_main("", body).result.unwrap().termination,
        Termination::Returned(Some(int(0)))
    );
}

#[test]
fn exit_and_abort_end_the_program() {
    let body = "\
    v0 = iconst.i32 120
    v1 = call @putchar(v0)
    v2 = iconst.i32 3
    call @exit(v2)
    unreachable
";
    let run = run_main("", body);
    assert_eq!(run.output, "x");
    assert_eq!(run.result.unwrap().termination, Termination::Exited(3));
    let body = "\
    call @abort()
    unreachable
";
    assert_eq!(
        run_main("", body).result.unwrap().termination,
        Termination::Aborted
    );
}

/// A host that provides `@answer`, which returns a fresh heap object holding
/// 42, and nothing else.
struct AnswerHost;

impl Host for AnswerHost {
    fn call(&mut self, call: HostCall<'_>, memory: &mut Memory<'_>) -> Result<HostReturn, Fault> {
        if call.name != "answer" {
            return Ok(HostReturn::NotProvided);
        }
        let align = Align::from_bytes(4).unwrap();
        let pointer = RuntimeValue::Ptr(memory.allocate(ObjectKind::Heap, 4, align)?);
        memory.store(Type::I32, int(42), pointer, align)?;
        Ok(HostReturn::Value(Some(pointer)))
    }
}

#[test]
fn programs_run_with_a_custom_host() {
    let text = "function @answer() -> ptr external
function @putchar(i32) -> i32 external

function @main() -> i32 external {
block0:
    v0 = call @answer()
    v1 = load.i32 v0, align 4
    v2 = call @putchar(v1)
    return v1
}
";
    let arena = Bump::new();
    let module = parse_module(&arena, text).unwrap();
    let function = |name: &str| match module.symbol(name) {
        | Some(Symbol::Function(func)) => func,
        | other => panic!("@{name} is {other:?}"),
    };
    let main = function("main");
    let call = module
        .function(main)
        .body
        .as_ref()
        .unwrap()
        .block_insts(Block::new(0))[2];
    assert_eq!(
        run(&module, "main", &[], &mut AnswerHost),
        Err(Trap::UnknownFunction(
            function("putchar"),
            Location {
                func: main,
                inst: call,
            }
        )),
        "the custom host provides only @answer"
    );
}

#[test]
fn custom_host_integer_results_must_fit_the_declared_width() {
    struct IntegerHost(u128);
    impl Host for IntegerHost {
        fn call(&mut self, _: HostCall<'_>, _: &mut Memory<'_>) -> Result<HostReturn, Fault> {
            Ok(HostReturn::Value(Some(RuntimeValue::Int(self.0))))
        }
    }

    for ty in [
        Type::I8,
        Type::I1,
        Type::I16,
        Type::I32,
        Type::I64,
        Type::I128,
    ] {
        let text = format!(
            "function @answer() -> {ty} external\nfunction @main() -> i32 external \
             {{\nblock0:\nv0 = call @answer()\nv1 = iconst.{ty} 0\nv2 = icmp.{ty} eq v0, v1\nv3 = \
             zext.i32 v2\nreturn v3\n}}\n"
        );
        let arena = Bump::new();
        let module = parse_module(&arena, &text).unwrap();
        if ty != Type::I128 {
            // In particular, @answer() -> i8 returning 256 must trap before
            // the comparison can consume an unnormalized SSA value.
            for bits in [ty.mask() + 1, u128::MAX] {
                assert!(
                    matches!(
                        run(&module, "main", &[], &mut IntegerHost(bits)),
                        Err(Trap::UndefinedBehavior(UbKind::SignatureMismatch, _))
                    ),
                    "{ty}: {bits}"
                );
            }
        }
        for (bits, expected) in [(0, 1), (ty.mask(), 0)] {
            assert_eq!(
                run(&module, "main", &[], &mut IntegerHost(bits))
                    .unwrap()
                    .termination,
                Termination::Returned(Some(int(expected))),
                "{ty}: {bits}"
            );
        }
    }
}

#[test]
fn custom_host_results_are_checked_before_entering_typed_slots() {
    // AnswerHost always returns a pointer; both declarations are incompatible.
    for result in [" -> f64", ""] {
        let text = format!(
            "function @answer(){result} external\nfunction @main() -> i32 external {{\nblock0:\n"
        );
        let text = if result.is_empty() {
            format!("{text}    call @answer()\n    v0 = iconst.i32 0\n    return v0\n}}\n")
        } else {
            format!("{text}    v0 = call @answer()\n    v1 = fptosi.i32 v0\n    return v1\n}}\n")
        };
        let arena = Bump::new();
        let module = parse_module(&arena, &text).unwrap();
        assert!(matches!(
            run(&module, "main", &[], &mut AnswerHost),
            Err(Trap::UndefinedBehavior(UbKind::SignatureMismatch, _))
        ));
    }
}
