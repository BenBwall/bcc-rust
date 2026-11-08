//! Debug information: the direct encoder's line table and debug sections
//! against the pinned clang assembler, byte for byte, and the DWARF the
//! emitter writes for C programs.

use std::fmt::Write as _;

use super::object::equivalent;

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
         17\n.section .text.h,\"axG\",@progbits,h,comdat\n.weak h\nh:\n.loc 1 1 1\nretq\n.text\nk:\n.loc \
         3 5 5\nretq\n.section .debug_line,\"\",@progbits\n.Lline_table_start0:\n",
    );
    equivalent(&text).unwrap();
}

/// A listing with files and no rows has a header alone, and data that
/// follows a `.loc` takes its row, as MC gives it one.
#[test]
fn line_tables_without_code_rows_match_the_assembler() {
    equivalent(".file 0 \"/w\" \"a.c\"\n.file 1 \"/w/x\" \"b.h\"\n").unwrap();
    equivalent(".file 0 \"/w\" \"a.c\"\n.data\n.loc 0 2 3\n.byte 1\n.text\n.loc 0 5 1\nretq\n")
        .unwrap();
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
    for value in [0_i64, 1, -1, 63, 64, -64, -65, 8191, -8192, i64::MIN, i64::MAX] {
        writeln!(text, ".sleb128 {value}").unwrap();
    }
    text.push_str(
        ".section .debug_info,\"\",@progbits\n.Lunit:\n.long .Lunit_end-.Lunit_start\n.Lunit_start:\n.\
         short 5\n.byte 1, 8\n.long .Labbrev\n.asciz \"bcc\\042x\\134y\\001\\377\"\n.ascii \
         \"plain\"\n.asciz \"\"\n.quad .Lbegin\n.long .Lend-.Lbegin\n.quad f+1\n.long \
         .Lline\n.Lunit_end:\n.section .debug_line,\"\",@progbits\n.Lline:\n",
    );
    equivalent(&text).unwrap();
}
