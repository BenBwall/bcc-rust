//! Debug information: the direct encoder's line table and debug sections
//! against the pinned clang assembler, byte for byte, and the DWARF the
//! emitter writes for C programs.

use std::{
    fmt::Write as _,
    path::Path,
    process::Command,
};

use super::{
    object::equivalent,
    programs::PROGRAMS,
};
use crate::{
    abi::tests::programs::PROGRAMS as BY_VALUE_PROGRAMS,
    backend::native::emit_assembly_with,
    configuration::{
        CompilerConfiguration,
        DebugOptions,
    },
    headers::HeaderSearch,
    ir::{
        DebugLevel,
        Profile,
    },
    lowering::tests::programs::PROGRAMS as LOWERING_PROGRAMS,
    optimizer::OptimizerOptions,
    pipeline::{
        analyze_translation_unit,
        lower_translation_unit,
        parse_translation_unit,
    },
    target::{
        CodeOptions,
        Target,
    },
    test_support::TempDir,
    translation_phases::Context,
    util::bump::Bump,
};

/// Rows across relaxed branches and alignment, in three sections (one a
/// COMDAT group), with line advances and address advances at each
/// encoding's boundary: special opcodes, `DW_LNS_const_add_pc`,
/// `DW_LNS_advance_pc`, `DW_LNS_advance_line` and both ends of a sequence.
#[test]
fn line_tables_match_the_assembler() {
    let mut text = String::from(
        ".text\n.file 0 \"/work/dir\" \"main.c\"\n.file 1 \"/usr/include\" \"stdio.h\"\n.file 2 \
         \"/work/dir\" \"local.h\"\n.file 3 \"inc\" \"other.h\"\n.file 4 \"/usr/include\" \
         \"stdlib.h\"\nf:\n.cfi_startproc\n.loc 0 3 0\npushq %rbp\n.loc 0 4 7 prologue_end\nmovl \
         $1, %eax\n.loc 0 4 9 is_stmt 0\njmp .Lend\n",
    );
    for (step, (line, column, gap)) in [
        (5, 1, 0),
        (13, 2, 16),
        (14, 3, 17),
        (15, 4, 18),
        (2, 5, 30),
        (30, 6, 200),
        (31, 7, 255),
        (32, 8, 256),
        (32, 9, 300),
        (900, 10, 4),
        (1, 300, 1),
    ]
    .into_iter()
    .enumerate()
    {
        let file = step % 5;
        let stmt = if step % 3 == 0 { " is_stmt 1" } else { "" };
        let end = if step == 6 { " epilogue_begin" } else { "" };
        writeln!(
            text,
            ".loc {file} {line} {column}{stmt}{end}\nxorl %eax,%eax\n.zero {gap}\nje .Lend"
        )
        .unwrap();
    }
    text.push_str(
        ".p2align 4, 0x90\n.Lend:\n.loc 0 40 1\nretq\n.cfi_endproc\n.section \
         .text.g,\"ax\",@progbits\ng:\n.loc 2 300 0 is_stmt 1\nretq\n.loc 2 300 0\nretq\n.zero \
         17\n.section .text.h,\"axG\",@progbits,h,comdat\n.weak h\nh:\n.loc 1 1 \
         1\nretq\n.text\nk:\n.loc 3 5 5\nretq\n.section \
         .debug_line,\"\",@progbits\n.Lline_table_start0:\n",
    );
    equivalent(&text).unwrap();
}

/// A listing with files and no rows has a header alone. As MC places rows,
/// data that follows a `.loc` takes its row, so does a `.loc` that another
/// follows before any code, and a change of section drops a pending one,
/// even one changed back.
#[test]
fn line_tables_without_code_rows_match_the_assembler() {
    for body in [
        ".file 1 \"/w/x\" \"b.h\"\n",
        ".data\n.loc 0 2 3\n.byte 1\n.text\n.loc 0 5 1\nretq\n",
        ".loc 0 2 0\n.cfi_startproc\n.L0:\n.loc 0 2 9 prologue_end\nretq\n.cfi_endproc\n.loc 0 7 \
         1\n.loc 0 8 1\n.data\n.byte 1\n",
        ".loc 0 3 1\n.data\nretq\n.text\n.loc 0 4 1\n.data\n.text\nretq\n.loc 0 5 1\n.text\nretq\n",
    ] {
        equivalent(&format!(".file 0 \"/w\" \"a.c\"\n{body}")).unwrap();
    }
}

/// The data directives of the debug sections: LEB128 constants at their
/// size boundaries, strings with octal escapes, and the label differences
/// and relocations a compilation unit names.
#[test]
fn debug_section_data_matches_the_assembler() {
    let mut text = String::from(
        ".text\nf:\n.Lbegin:\nretq\n.Lend:\n.section .debug_abbrev,\"\",@progbits\n.Labbrev:\n",
    );
    for value in [0_u64, 1, 63, 64, 127, 128, 16383, 16384, u64::MAX] {
        writeln!(text, ".uleb128 {value}").unwrap();
    }
    for value in [
        0_i64,
        1,
        -1,
        63,
        64,
        -64,
        -65,
        8191,
        -8192,
        i64::MIN,
        i64::MAX,
    ] {
        writeln!(text, ".sleb128 {value}").unwrap();
    }
    text.push_str(
        ".section .debug_info,\"\",@progbits\n.Lunit:\n.long \
         .Lunit_end-.Lunit_start\n.Lunit_start:\n.short 5\n.byte 1, 8\n.long .Labbrev\n.asciz \
         \"bcc\\042x\\134y\\001\\377\"\n.ascii \"plain\"\n.asciz \"\"\n.quad .Lbegin\n.long \
         .Lend-.Lbegin\n.quad f+1\n.long .Lline\n.Lunit_end:\n.section \
         .debug_line,\"\",@progbits\n.Lline:\n",
    );
    equivalent(&text).unwrap();
}

/// Each native, lowering and by-value test program at `-O0` with its locals
/// in memory, and through the bcc optimizer: `-g` leaves the code as it was,
/// the direct object matches the assembler's byte for byte, and
/// `llvm-dwarfdump --verify`, where it is installed, accepts the object.
#[test]
fn c_programs_describe_themselves_without_changing_their_code() {
    let directory = TempDir::new("native-debug-info");
    let mut described = 0;
    for (name, source) in PROGRAMS
        .iter()
        .map(|program| (program.name, program.source))
        .chain(
            LOWERING_PROGRAMS
                .iter()
                .map(|program| (program.name, program.source)),
        )
        .chain(
            BY_VALUE_PROGRAMS
                .iter()
                .map(|program| (program.name, program.source)),
        )
    {
        for optimize in [false, true] {
            let Some(plain) = listing(source, optimize, false) else {
                continue;
            };
            let text = listing(source, optimize, true).expect("-g refuses nothing more");
            assert_eq!(
                code(&text),
                code(&plain),
                "{name} (optimized: {optimize}): -g changed the code"
            );
            assert!(text.contains(".loc\t"), "{name}: no rows\n{text}");
            equivalent(&text).unwrap();
            verify(&directory, name, &text);
            described += 1;
        }
    }
    assert!(described > 100, "only {described} programs compiled");
}

/// The native listing of `source` for x86-64 Linux, lowered with `-g` if
/// `debug` and with the locals in memory unless `optimize`, as the command
/// line keeps them under a compiler personality at `-O0`; `None` if the
/// back end refuses the program.
fn listing(source: &str, optimize: bool, debug: bool) -> Option<String> {
    let tu = Bump::new();
    let source = tu.alloc_str(source);
    let configuration = CompilerConfiguration::default()
        .with_target(Target::LinuxGnu)
        .with_debug_info(debug.then_some(DebugOptions {
            level:         DebugLevel::Full,
            dwarf_version: None,
            columns:       true,
            physical:      false,
        }))
        .with_stack_locals(!optimize);
    let mut context = Context::with_configuration(&tu, configuration);
    let unit = parse_translation_unit(
        &mut context,
        Path::new("test.c"),
        source,
        HeaderSearch::default(),
    );
    let sema = analyze_translation_unit(&mut context, &unit);
    if !sema.lowerable(&context) {
        return None;
    }
    let ir = Bump::new();
    let options = OptimizerOptions::default();
    let module = lower_translation_unit(
        &context,
        &unit,
        &sema,
        &ir,
        optimize.then_some(&options),
        Profile::PostAbi,
    )
    .ok()?;
    let mut text = String::new();
    let code = CodeOptions {
        debug_info: debug,
        ..CodeOptions::default()
    };
    emit_assembly_with(&module, Target::LinuxGnu, code, &mut text).ok()?;
    Some(text)
}

/// A listing without its debug information: its rows, files and labels,
/// and the debug sections that end it.
fn code(text: &str) -> String {
    let end = text.find("\t.section\t.debug_abbrev").unwrap_or(text.len());
    text[..end]
        .lines()
        .filter(|line| {
            !line.starts_with("\t.loc\t")
                && !line.starts_with("\t.file\t")
                && !line.starts_with(".Ldebug_")
        })
        .fold(String::new(), |mut code, line| {
            code.push_str(line);
            code.push('\n');
            code
        })
}

/// Runs `llvm-dwarfdump --verify` on the direct object of `text`, if the
/// tool is installed.
fn verify(directory: &TempDir, name: &str, text: &str) {
    let arena = Bump::new();
    let bytes = crate::backend::native::object::encode(text, &arena)
        .unwrap_or_else(|error| panic!("{error}\n{text}"));
    let path = directory.join(format!("{name}.o"));
    std::fs::write(&path, &*bytes).unwrap();
    match Command::new("llvm-dwarfdump")
        .arg("--verify")
        .arg(&path)
        .output()
    {
        | Ok(output) => assert!(
            output.status.success(),
            "{name}: {}\n{text}",
            String::from_utf8_lossy(&output.stdout)
        ),
        | Err(error) => eprintln!("SKIP llvm-dwarfdump --verify: {error}"),
    }
}

/// What the test programs may miss: code outside `.text`, which makes the
/// unit's code a range list; a block whose code comes in several runs;
/// files that `#line` names with a directory, with a quote and a
/// backslash, and with the main file's name; static locals; a
/// variable-length array; and a structure passed by value.
#[test]
fn unusual_programs_describe_themselves() {
    let directory = TempDir::new("native-debug-info-unusual");
    let source = "\
struct big { long a, b, c; };
__attribute__((section(\".text.hot\"))) int hot(int x) { return x * 3; }
long by_value(struct big b) { return b.a + b.c; }
int shadow(int n)
{
  int total = 0;
  for (int i = 0; i < n; i++) {
    int total = i * 2;
    if (total > 4)
      break;
    { static int seen; seen += total; }
  }
  return total;
}
#line 7 \"/abs/dir/header.h\"
static int in_header(int v) { return v + 1; }
#line 3 \"odd \\\"name\\\\.h\"
int odd(int v) { int vla[v + 1]; vla[v] = v; return vla[v] + in_header(v); }
#line 9 \"test.c\"
int main(void) { struct big b = { 1, 2, 3 }; return hot(1) + (int)by_value(b) + shadow(5) + \
                  odd(2); }
";
    for optimize in [false, true] {
        let text = listing(source, optimize, true).expect("the back end compiles the program");
        assert!(
            text.contains(".section\t.debug_rnglists"),
            "the unit's code is in two sections\n{text}"
        );
        assert!(
            text.contains("\"odd \\042name\\134.h\""),
            "the file name is escaped\n{text}"
        );
        equivalent(&text).unwrap();
        verify(&directory, "unusual", &text);
    }
}
