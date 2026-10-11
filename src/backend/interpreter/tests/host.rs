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
