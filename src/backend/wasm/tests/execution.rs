//! Whole programs through the WebAssembly back end: each is compiled in
//! process, validated by Node's WebAssembly engine, and run under Node's
//! WASI preview 1 (`scripts/wasm/run.mjs`), comparing the exit status and
//! the output. Without `node` on the path the tests print a note and pass.

use std::{
    io::Write,
    path::{
        Path,
        PathBuf,
    },
    process::{
        Command,
        Stdio,
    },
};

use crate::{
    backend::wasm,
    configuration::CompilerConfiguration,
    headers::{
        HeaderSearch,
        WASI_DIRECTORY,
    },
    ir::Profile,
    lowering::tests::programs::PROGRAMS,
    optimizer::OptimizerOptions,
    pipeline::{
        InlineCopies,
        analyze_translation_unit,
        lower_translation_unit_with_locals,
        parse_translation_unit,
    },
    target::Target,
    translation_phases::Context,
    util::bump::Bump,
};

/// Compiles `source` for `target`, optimized or not, into a linked module.
pub(crate) fn compile(source: &str, target: Target, optimize: bool) -> Result<Vec<u8>, String> {
    compile_with(source, target, optimize, |compiled| compiled.bytes.to_vec())
}

/// Compiles `source` and hands the compiled module to `with`.
pub(crate) fn compile_with<T: Send>(
    source: &str,
    target: Target,
    optimize: bool,
    with: impl FnOnce(&wasm::Compiled<'_>) -> T + Send,
) -> Result<T, String> {
    let configuration = CompilerConfiguration::default()
        .with_target(target)
        .with_gnu_extensions(true);
    compile_configured(source, configuration, optimize, with)
}

/// Compiles `source` under `configuration` (whose target it uses), optimized
/// or not, and hands the compiled module to `with`.
pub(crate) fn compile_configured<T: Send>(
    source: &str,
    configuration: CompilerConfiguration,
    optimize: bool,
    with: impl FnOnce(&wasm::Compiled<'_>) -> T + Send,
) -> Result<T, String> {
    let target = configuration.target();
    crate::test_support::on_compilation_stack(|| {
        let configuration = configuration.with_optimization((optimize, false));
        let tu = Bump::new();
        let mut context = Context::with_configuration(&tu, configuration);
        let source = tu.alloc_str(source);
        let after: &[&Path] = if target == Target::Wasm32Wasi {
            &[Path::new(WASI_DIRECTORY)]
        } else {
            &[]
        };
        let search = HeaderSearch {
            after,
            ..HeaderSearch::default()
        };
        let unit = parse_translation_unit(&mut context, Path::new("program.c"), source, search);
        let sema = analyze_translation_unit(&mut context, &unit);
        if !sema.lowerable(&context) {
            return Err("the program has errors".to_owned());
        }
        let arena = Bump::new();
        let optimizer = optimize.then(OptimizerOptions::default);
        let diagnosed = lower_translation_unit_with_locals(
            &context,
            &unit,
            &sema,
            &arena,
            optimizer.as_ref(),
            Profile::PostAbi,
            (InlineCopies::Resolve, false),
        )
        .map_err(|error| format!("{error:?}"))?;
        let runtime: &[crate::ir::Module<'_>] = if target == Target::Wasm32Wasi {
            wasm::runtime::modules(target).map_err(str::to_owned)?
        } else {
            &[]
        };
        let mut units = vec![&diagnosed.module];
        units.extend(runtime.iter());
        let compiled =
            wasm::compile(&units, 1, target, &arena).map_err(|error| error.to_string())?;
        Ok(with(&compiled))
    })
}

/// What running a module printed and returned.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Run {
    pub(crate) status: i32,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

/// Whether Node.js is available to run modules.
pub(crate) fn node_available() -> bool {
    Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn runner() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/wasm/run.mjs")
}

/// A fresh scratch directory for one module.
fn scratch_path(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("bcc-wasm-tests-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    directory.join(format!("{name}.wasm"))
}

/// Validates `bytes` with Node's engine, returning the engine's complaint.
pub(crate) fn validate(name: &str, bytes: &[u8]) -> Result<(), String> {
    let path = scratch_path(name);
    std::fs::write(&path, bytes).unwrap();
    let output = Command::new("node")
        .arg(runner())
        .arg("--validate")
        .arg(&path)
        .output()
        .unwrap();
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned())
    }
}

/// Runs a WASI module under Node.
pub(crate) fn run(name: &str, bytes: &[u8]) -> Run {
    run_with_input(name, bytes, b"")
}

/// Runs a WASI module under Node with `input` on its standard input.
pub(crate) fn run_with_input(name: &str, bytes: &[u8], input: &[u8]) -> Run {
    run_with_arguments(name, bytes, input, &[])
}

/// Runs a WASI module under Node with `input` on its standard input and
/// `arguments` after its path in `argv`.
pub(crate) fn run_with_arguments(
    name: &str,
    bytes: &[u8],
    input: &[u8],
    arguments: &[&str],
) -> Run {
    let path = scratch_path(name);
    std::fs::write(&path, bytes).unwrap();
    let mut child = Command::new("node")
        .arg("--no-warnings")
        .arg(runner())
        .arg(&path)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Dropping the handle closes the pipe, so the program sees the end.
    child.stdin.take().unwrap().write_all(input).unwrap();
    let output = child.wait_with_output().unwrap();
    Run {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

#[test]
fn every_lowering_program_runs_under_wasi_with_its_status() {
    if !node_available() {
        eprintln!("note: node is not on the path; skipping the WebAssembly execution test");
        return;
    }
    let mut failures = Vec::new();
    for program in PROGRAMS {
        for optimize in [false, true] {
            let label = format!("{}{}", program.name, if optimize { "-O" } else { "" });
            match compile(program.source, Target::Wasm32Wasi, optimize) {
                | Ok(bytes) => {
                    if let Err(error) = validate(&label, &bytes) {
                        failures.push(format!("{label}: invalid module: {error}"));
                        continue;
                    }
                    let run = run(&label, &bytes);
                    if run.status != program.expected & 0xFF {
                        failures.push(format!(
                            "{label}: status {} instead of {}; stderr: {}",
                            run.status, program.expected, run.stderr
                        ));
                    }
                },
                | Err(error) => failures.push(format!("{label}: {error}")),
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn freestanding_modules_validate_and_export_their_functions() {
    if !node_available() {
        eprintln!("note: node is not on the path; skipping the WebAssembly validation test");
        return;
    }
    for program in PROGRAMS {
        let bytes = compile(program.source, Target::Wasm32Unknown, false)
            .unwrap_or_else(|error| panic!("{}: {error}", program.name));
        validate(program.name, &bytes).unwrap_or_else(|error| panic!("{}: {error}", program.name));
    }
}

/// `printf` against glibc's output for the same calls.
#[test]
fn printf_matches_glibc() {
    if !node_available() {
        eprintln!("note: node is not on the path; skipping the WebAssembly printf test");
        return;
    }
    let source = r#"
#include <stdio.h>
int main(void) {
    printf("[%d|%5d|%-5d|%05d|%+d|% d|%.3d]\n", 42, 42, 42, 42, 42, 42, 7);
    printf("[%u|%x|%X|%#x|%o|%#o|%lld|%llu]\n", 3000000000u, 255, 255, 255, 8, 8, -1234567890123LL, 18446744073709551615ULL);
    printf("[%c|%s|%.2s|%10s|%-10s|]\n", 'x', "text", "text", "right", "left");
    printf("[%f|%.2f|%10.3f|%-10.1f|%e|%.0e|%g|%g|%g|%g]\n", 3.14159265, 2.675, -1.5, 2.25, 12345.678, 9.5, 0.0001, 123456789.0, 1e-5, 100.0);
    printf("[%.17g|%.0f|%.0f|%.0f|%a|%a|%A]\n", 0.1, 0.5, 1.5, 2.5, 1.0, 0.1, -3.75);
    printf("[%f|%e|%g|%f|%F]\n", 0.0, -0.0, 1e300, 1.0/0.0, -1.0/0.0);
    printf("[%p|%%|%5.1f%%]\n", (void *)0, 99.5);
    printf("[%.30f]\n", 1e-20);
    printf("[%.3e|%G|%#g|%#.0f]\n", 9.9996, 1e-10, 1.0, 3.0);
    char buffer[16];
    int n = snprintf(buffer, sizeof buffer, "%s-%d", "truncated output", 12345);
    printf("[%s|%d]\n", buffer, n);
    return 0;
}
"#;
    let bytes = compile(source, Target::Wasm32Wasi, false).unwrap();
    let run = run("printf", &bytes);
    assert_eq!(run.status, 0, "{}", run.stderr);
    let expected = [
        "[42|   42|42   |00042|+42| 42|007]",
        "[3000000000|ff|FF|0xff|10|010|-1234567890123|18446744073709551615]",
        "[x|text|te|     right|left      |]",
        "[3.141593|2.67|    -1.500|2.2       |1.234568e+04|1e+01|0.0001|1.23457e+08|1e-05|100]",
        "[0.10000000000000001|0|2|2|0x1p+0|0x1.999999999999ap-4|-0X1.EP+1]",
        "[0.000000|-0.000000e+00|1e+300|inf|-INF]",
        "[(nil)|%| 99.5%]",
        "[0.000000000000000000010000000000]",
        "[1.000e+01|1E-10|1.00000|3.]",
        "[truncated outpu|22]",
    ];
    let expected = expected.join("\n") + "\n";
    assert_eq!(run.stdout, expected);
}

#[test]
fn wasi_programs_use_the_c_library() {
    if !node_available() {
        eprintln!("note: node is not on the path; skipping the WebAssembly C library test");
        return;
    }
    let source = r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <ctype.h>
static int compare(const void *a, const void *b) { return *(const int *)a - *(const int *)b; }
struct node { int value; struct node *next; };
static void bye(void) { puts("atexit ran"); }
int main(void) {
    atexit(bye);
    int values[] = {5, 3, 9, 1, 7, 2, 8, 6, 4, 0};
    qsort(values, 10, sizeof values[0], compare);
    for (int i = 0; i < 10; i++) printf("%d ", values[i]);
    putchar('\n');
    struct node *list = 0;
    for (int i = 0; i < 1000; i++) {
        struct node *n = malloc(sizeof *n);
        n->value = i;
        n->next = list;
        list = n;
    }
    long sum = 0;
    while (list) { struct node *next = list->next; sum += list->value; free(list); list = next; }
    printf("sum %ld\n", sum);
    char *big = calloc(1 << 20, 1);
    big[(1 << 20) - 1] = 'z';
    big = realloc(big, 3 << 20);
    printf("big %c\n", big[(1 << 20) - 1]);
    char text[64];
    strcpy(text, "Hello, ");
    strcat(text, "World");
    printf("%s %zu %d %s\n", text, strlen(text), strcmp("abc", "abd") < 0, strchr(text, 'W'));
    for (char *p = text; *p; p++) *p = (char)toupper((unsigned char)*p);
    puts(text);
    printf("%ld %lu %d\n", strtol("-0x1f", 0, 16), strtoul("777", 0, 8), atoi("  123abc"));
    printf("%g %g\n", strtod("2.5e3", 0), atof("0.125"));
    int a, b; char word[16];
    printf("%d\n", sscanf("12 34 xyz", "%d %d %15s", &a, &b, word));
    printf("%d %d %s\n", a, b, word);
    fprintf(stderr, "to stderr\n");
    exit(7);
}
"#;
    for optimize in [false, true] {
        let bytes = compile(source, Target::Wasm32Wasi, optimize).unwrap();
        let run = run("libc", &bytes);
        assert_eq!(run.status, 7, "{}", run.stderr);
        assert_eq!(
            run.stdout,
            "0 1 2 3 4 5 6 7 8 9 \nsum 499500\nbig z\nHello, World 12 1 World\nHELLO, WORLD\n-31 \
             511 123\n2500 0.125\n3\n12 34 xyz\natexit ran\n"
        );
        assert_eq!(run.stderr, "to stderr\n");
    }
}

/// Compound literals in static initializers keep the target's formats:
/// `long double` is binary128 on WebAssembly, whose bytes a literal gives
/// whole, through a cast, as a converted braced element and as Clang's
/// folded value, and `__fp16` literals fold in `float`. The bytes are those
/// x86-64 Clang gives `__float128` and `__fp16`.
#[test]
fn static_compound_literals_keep_binary128_and_fp16_bytes() {
    if !node_available() {
        eprintln!("note: node is not on the path; skipping the WebAssembly literal test");
        return;
    }
    let source = r#"
int printf(const char *, ...);
static long double whole = (long double){0.1L};
static long double cast = (long double)(long double){0.1L};
static long double elements[2] = { (long double){1.5L}, (double){0.1} };
static long double product = (long double){0.1L} * 3;
static __fp16 half = (__fp16){0.1};
static float half_product = (__fp16){0.1} * 3;
static __fp16 half_element[1] = { (float){0.3f} };
static void dump(const char *name, const void *p, unsigned n) {
    const unsigned char *b = p;
    printf("%s", name);
    for (unsigned i = 0; i < n; i++) printf(" %02x", b[i]);
    printf("\n");
}
int main(void) {
    dump("whole", &whole, sizeof whole);
    dump("cast", &cast, sizeof cast);
    dump("elements", elements, sizeof elements);
    dump("product", &product, sizeof product);
    dump("half", &half, sizeof half);
    printf("half_product %a\n", (double)half_product);
    dump("half_element", half_element, sizeof half_element);
    return 0;
}
"#;
    let tenth = "9a 99 99 99 99 99 99 99 99 99 99 99 99 99 fb 3f";
    let expected = format!(
        "whole {tenth}\ncast {tenth}\nelements 00 00 00 00 00 00 00 00 00 00 00 00 00 80 ff 3f 00 \
         00 00 00 00 00 00 a0 99 99 99 99 99 99 fb 3f\nproduct 34 33 33 33 33 33 33 33 33 33 33 \
         33 33 33 fd 3f\nhalf 66 2e\nhalf_product 0x1.332p-2\nhalf_element cd 34\n"
    );
    for optimize in [false, true] {
        let bytes = compile(source, Target::Wasm32Wasi, optimize).unwrap();
        let run = run("literals", &bytes);
        assert_eq!(run.status, 0, "{}", run.stderr);
        assert_eq!(run.stdout, expected, "optimize {optimize}");
    }
}

/// Every instruction's offset and text line agree: the byte at the offset is
/// the opcode the text line spells, and the source map's offsets start
/// instructions of the right function.
#[test]
fn offsets_text_lines_and_source_map_agree() {
    let source = "int add(int a, int b) {\n    int c = a + b;\n    return c * 3;\n}\n";
    let (bytes, text, offsets, lines, entries) =
        compile_with(source, Target::Wasm32Unknown, false, |compiled| {
            let mut text = String::new();
            let mut lines = Vec::new();
            compiled
                .write_text_with_lines(&mut text, &mut lines)
                .unwrap();
            let entries: Vec<_> = compiled
                .entries
                .iter()
                .map(|entry| (entry.offset, entry.line))
                .collect();
            // Every instruction lies in its function's code range.
            for &offset in &compiled.offsets {
                assert!(
                    compiled
                        .bodies
                        .iter()
                        .any(|&(_, start, end)| (start..end).contains(&offset))
                );
            }
            // Each function's range starts at its first instruction, after
            // the locals, and ends at its final `end`.
            for &(name, start, end) in &compiled.bodies {
                let first = compiled
                    .offsets
                    .iter()
                    .copied()
                    .filter(|offset| (start..end).contains(offset))
                    .min();
                assert_eq!(first, Some(start), "{name}");
                assert_eq!(compiled.bytes[end as usize - 1], 0x0B, "{name}");
            }
            (
                compiled.bytes.to_vec(),
                text,
                compiled.offsets.to_vec(),
                lines,
                entries,
            )
        })
        .unwrap();
    assert_eq!(offsets.len(), lines.len());
    let text_lines: Vec<&str> = text.lines().collect();
    let mut adds = 0;
    for (&offset, &line) in offsets.iter().zip(&lines) {
        let spelled = text_lines[line as usize - 1].trim();
        match bytes[offset as usize] {
            | 0x6A => {
                assert_eq!(spelled, "i32.add");
                adds += 1;
            },
            | 0x6C => assert_eq!(spelled, "i32.mul"),
            | 0x20 => assert!(spelled.starts_with("local.get "), "{spelled}"),
            | 0x0F => assert_eq!(spelled, "return"),
            | _ => {},
        }
    }
    assert_eq!(adds, 1);
    // Each instruction's code starts where an instruction starts, and the
    // statements keep their source lines.
    for &(offset, _) in &entries {
        assert!(
            offsets.contains(&offset) || bytes[offset as usize] == 0x0B,
            "{offset:#x}"
        );
    }
    let lines_seen: Vec<u32> = entries.iter().map(|&(_, line)| line).collect();
    assert!(
        lines_seen.contains(&2) && lines_seen.contains(&3),
        "{lines_seen:?}"
    );
}

/// Code a macro expands to has the line of the outermost invocation, never
/// one inside the `#define`, as Clang's `-g` line tables give it, also for
/// an argument spelled in another macro's definition (`NESTED`).
#[test]
fn macro_expansions_have_the_lines_of_their_invocations() {
    let source = r"#define SWAP(a, b) do {   \
        int t = (a);      \
        (a) = (b);        \
        (b) = t;          \
    } while (0)
#define INC(x) ((x) + 1)
#define TWICE(y) INC(INC(y))
#define NESTED INC(seven)
int seven = 7;
int f(int p, int q) {
    if (p > q) {
        SWAP(p, q);
    }
    int r = TWICE(p);
    int s = INC(
        q);
    int u = NESTED;
    return r + s + u;
}
";
    assert_eq!(source.lines().nth(11), Some("        SWAP(p, q);"));
    let lines = compile_with(source, Target::Wasm32Unknown, false, |compiled| {
        let mut lines: Vec<u32> = compiled.entries.iter().map(|entry| entry.line).collect();
        lines.sort_unstable();
        lines.dedup();
        lines
    })
    .unwrap();
    // Line 0 is the prologue; every other line is one of `f`'s own. bcc
    // gives a statement or expression the line its first token is on, so
    // `INC(\n q)` is line 15 as `((\n q) + 1)` would be.
    assert_eq!(lines, [0, 11, 12, 14, 15, 17, 18], "{lines:?}");
}

/// The one case where bcc's lines differ from Clang's (see
/// `Context::file_vector`): a macro invoked within an argument that starts
/// on a later line takes the outer invocation's line, so `BUMP`'s
/// `do … while` is on line 4 where Clang gives it line 5. Its `v += 2` is
/// on line 5 anyway, since the file spells its first token, `x`, there.
#[test]
fn a_macro_in_a_later_argument_line_takes_the_outer_invocations_line() {
    let source = "#define F(a) a\n#define BUMP(v) do { v += 2; } while (0)\nint f(int x) {\n    \
                  F(\n        BUMP(x));\n    return x + 1;\n}\n";
    let lines = compile_with(source, Target::Wasm32Unknown, false, |compiled| {
        let mut lines: Vec<u32> = compiled.entries.iter().map(|entry| entry.line).collect();
        lines.sort_unstable();
        lines.dedup();
        lines
    })
    .unwrap();
    assert_eq!(lines, [0, 4, 5, 6], "{lines:?}");
}

/// Symbols keep their underscores in the text, since `_` is an `idchar`
/// (WebAssembly 2.0 §6.3.5); only characters outside that set are escaped.
#[test]
fn text_identifiers_keep_underscores() {
    let source = "int my_value(void) { return 7; }\nint main(void) { return my_value(); }\n";
    let text = compile_with(source, Target::Wasm32Wasi, false, |compiled| {
        let mut text = String::new();
        compiled
            .write_text_with_lines(&mut text, &mut Vec::new())
            .unwrap();
        text
    })
    .unwrap();
    for spelled in [
        "(export \"_start\" (func $_start))",
        "(func $_start ",
        "(func $my_value ",
        "call $__bcc_main",
    ] {
        assert!(text.contains(spelled), "{spelled} not in the text");
    }
    assert!(!text.contains('`'), "nothing needs escaping");
}

/// Every function's code starts with a source-map entry, so a prologue or
/// a function the linker made never takes the line of the code before it:
/// the entry has no IR position and line 0 unless an IR instruction's code
/// starts there.
#[test]
fn every_function_starts_with_a_source_entry() {
    let source = "int twice(int x) {\n    return 2 * x;\n}\nint main(void) {\n    char \
                  buffer[64];\n    buffer[0] = (char)twice(3);\n    return buffer[0];\n}\n";
    compile_with(source, Target::Wasm32Wasi, false, |compiled| {
        let mut names = Vec::new();
        for &(name, start, end) in &compiled.bodies {
            let first = compiled
                .entries
                .iter()
                .find(|entry| (start..end).contains(&entry.offset))
                .unwrap_or_else(|| panic!("{name} has no entry"));
            assert_eq!(first.offset, start, "{name}");
            if first.block == u32::MAX {
                assert_eq!((first.inst, first.line), (u32::MAX, 0), "{name}");
            }
            names.push(name);
        }
        for linked in ["_start", "__bcc_main"] {
            assert!(names.contains(&linked), "{names:?}");
        }
        let main = compiled
            .bodies
            .iter()
            .find(|&&(name, ..)| name == "main")
            .unwrap();
        let entry = compiled
            .entries
            .iter()
            .find(|entry| entry.offset == main.1)
            .unwrap();
        assert!(entry.line == 0 || entry.line == 4, "{entry:?}");
    })
    .unwrap();
}

/// A call through a declaration without a prototype passes its promoted
/// arguments as the definition's parameters, not as variadic arguments.
#[test]
fn unprototyped_calls_reach_prototyped_definitions() {
    if !node_available() {
        eprintln!("note: node is not on the path; skipping the WebAssembly K&R call test");
        return;
    }
    let source = "extern unsigned long strlen();\nextern void exit();\nstatic int twice(int x) { \
                  return 2 * x; }\nint (*keep)(int) = twice;\nint main(void) { if \
                  (strlen(\"four\") != 4) return 1; exit(9); }\n";
    let bytes = compile(source, Target::Wasm32Wasi, false).unwrap();
    let run = run("unprototyped", &bytes);
    assert_eq!(run.status, 9, "{}", run.stderr);
}

/// Atomic read-modify-writes of every lock-free width, emulated with plain
/// accesses, read and write the object: a 32-bit one used to keep its
/// address and the old value in the same scratch local, so `x += 2` on an
/// `_Atomic int` or `_Atomic float` stored nothing.
#[test]
fn atomic_read_modify_writes_update_their_object() {
    if !node_available() {
        eprintln!("note: node is not on the path; skipping the WebAssembly atomics test");
        return;
    }
    let source = r"
_Atomic signed char c = 100;
_Atomic short s = -300;
_Atomic int i = 7;
_Atomic long long ll = 1LL << 40;
_Atomic float f = 1.5f;
_Atomic double d = 0.5;
_Atomic long double l = 0.5L;
int plain = 5;
int main(void) {
    if ((i += 2) != 9 || i != 9 || i++ != 9 || --i != 9) return 1;
    if ((i -= 0.5) != 8 || (i *= 2.5f) != 20 || i != 20) return 2;
    if ((c += 27) != 127 || (s *= 2.25) != -675 || (ll -= 1.0) != (1LL << 40) - 1) return 3;
    if ((f += 0.25) != 1.75f || f-- != 1.75f || f != 0.75f) return 4;
    if ((d *= 3) != 1.5 || (l /= 4) != 0.125L) return 5;
    if (__atomic_exchange_n(&plain, 6, 5) != 5 || plain != 6) return 6;
    int expected = 6;
    if (!__atomic_compare_exchange_n(&plain, &expected, 8, 0, 5, 5) || plain != 8) return 7;
    if (__atomic_compare_exchange_n(&plain, &expected, 9, 0, 5, 5) || expected != 8) return 8;
    if (__atomic_fetch_add(&plain, 4, 5) != 8 || __atomic_fetch_and(&plain, 6, 5) != 12) return 9;
    if (__atomic_fetch_or(&plain, 1, 5) != 4 || __atomic_fetch_xor(&plain, 7, 5) != 5) return 10;
    return plain == 2 ? 0 : 11;
}";
    for optimize in [false, true] {
        let bytes = compile(source, Target::Wasm32Wasi, optimize).unwrap();
        let run = run("atomics", &bytes);
        assert_eq!(run.status, 0, "optimize {optimize}: {}", run.stderr);
    }
}

/// The runtime is compiled once per thread, and the kept modules link to
/// the same module a fresh build of the runtime does.
#[test]
fn cached_runtime_matches_a_fresh_build() {
    let first = wasm::runtime::modules(Target::Wasm32Wasi).unwrap();
    let second = wasm::runtime::modules(Target::Wasm32Wasi).unwrap();
    assert!(
        std::ptr::eq(first, second),
        "the second call reuses the modules"
    );
    _ = wasm::runtime::modules(Target::Wasm32Unknown).unwrap_err();
    let source = "#include <stdio.h>\nint main(void) { printf(\"%d\\n\", 42); return 0; }\n";
    let cached = compile(source, Target::Wasm32Wasi, false).unwrap();
    let fresh = crate::test_support::on_compilation_stack(|| {
        let arena = Bump::new();
        let runtime = wasm::runtime::compile(Target::Wasm32Wasi, &arena).unwrap();
        let program = compile_module(source, &arena);
        let mut units = vec![&program];
        units.extend(runtime.iter());
        wasm::compile(&units, 1, Target::Wasm32Wasi, &arena)
            .unwrap()
            .bytes
            .to_vec()
    });
    assert_eq!(cached, fresh);
}

/// `source` lowered for `wasm32-wasip1` into `arena`.
fn compile_module<'ir>(source: &str, arena: &'ir Bump) -> crate::ir::Module<'ir> {
    let configuration = CompilerConfiguration::default()
        .with_target(Target::Wasm32Wasi)
        .with_gnu_extensions(true);
    let tu = Bump::new();
    let mut context = Context::with_configuration(&tu, configuration);
    let source = tu.alloc_str(source);
    let search = HeaderSearch {
        after: &[Path::new(WASI_DIRECTORY)],
        ..HeaderSearch::default()
    };
    let unit = parse_translation_unit(&mut context, Path::new("program.c"), source, search);
    let sema = analyze_translation_unit(&mut context, &unit);
    assert!(sema.lowerable(&context));
    lower_translation_unit_with_locals(
        &context,
        &unit,
        &sema,
        arena,
        None,
        Profile::PostAbi,
        (InlineCopies::Resolve, false),
    )
    .unwrap()
    .module
}
