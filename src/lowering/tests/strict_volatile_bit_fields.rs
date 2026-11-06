//! `-fstrict-volatile-bitfields`: the width of each access to a `volatile`
//! bit-field. The expected widths are what GCC 13.3 and 14.2 emit for x86-64
//! at `-O0` and `-O2`, for the strict access and for the fallback alike.

use super::*;
use crate::{
    configuration::CompilerConfiguration,
    target::CodeOptions,
};

/// One case: the declarations, an lvalue over `struct S *p`, and the volatile
/// accesses that reading it and that assigning 3 to it perform.
struct Case {
    declarations: &'static str,
    lvalue:       &'static str,
    read:         &'static [&'static str],
    write:        &'static [&'static str],
}

/// The memory instructions of the volatile accesses in `source`, in order.
fn volatile_accesses(source: &str, strict: bool) -> Vec<String> {
    let options = CodeOptions {
        strict_volatile_bit_fields: strict,
        ..CodeOptions::default()
    };
    let configuration = CompilerConfiguration::default().with_code_options(options);
    let text = lower_with(source, configuration)
        .unwrap_or_else(|errors| panic!("lowering failed: {errors:?}\n{source}"));
    text.lines()
        .filter(|line| line.contains(" volatile "))
        .filter_map(|line| {
            line.split_whitespace()
                .find(|word| word.starts_with("load.") || word.starts_with("store."))
        })
        .map(Into::into)
        .collect()
}

/// Reads `case.lvalue`, then assigns to it, in functions of their own.
fn check(case: &Case) {
    let Case {
        declarations,
        lvalue,
        read,
        write,
    } = case;
    let reader = format!("{declarations} long r(struct S *p) {{ return {lvalue}; }}");
    assert_eq!(volatile_accesses(&reader, true), *read, "read: {reader}");
    let writer = format!("{declarations} void w(struct S *p) {{ {lvalue} = 3; }}");
    assert_eq!(volatile_accesses(&writer, true), *write, "write: {writer}");
}

#[test]
fn accesses_use_the_declared_type_where_gcc_does() {
    const I8: &[&str] = &["load.i8", "store.i8"];
    const I16: &[&str] = &["load.i16", "store.i16"];
    const I32: &[&str] = &["load.i32", "store.i32"];
    const I64: &[&str] = &["load.i64", "store.i64"];
    for case in [
        // Consecutive fields that fill the unit, in each width.
        Case {
            declarations: "struct S { volatile unsigned a : 8, b : 8, c : 16; };",
            lvalue:       "p->b",
            read:         &["load.i32"],
            write:        I32,
        },
        Case {
            declarations: "struct S { volatile unsigned short a : 8, b : 8; };",
            lvalue:       "p->b",
            read:         &["load.i16"],
            write:        I16,
        },
        Case {
            declarations: "struct S { volatile long long f : 40, g : 24; };",
            lvalue:       "p->g",
            read:         &["load.i64"],
            write:        I64,
        },
        Case {
            declarations: "struct S { volatile char a : 3, b : 5; };",
            lvalue:       "p->b",
            read:         &["load.i8"],
            write:        I8,
        },
        // An unnamed field belongs to the run.
        Case {
            declarations: "struct S { volatile unsigned a : 4; unsigned : 28; };",
            lvalue:       "p->a",
            read:         &["load.i32"],
            write:        I32,
        },
        // A field that fills the unit is stored without a read.
        Case {
            declarations: "struct S { volatile unsigned a : 32; };",
            lvalue:       "p->a",
            read:         &["load.i32"],
            write:        &["store.i32"],
        },
        Case {
            declarations: "struct S { int i; volatile unsigned short b : 16, c : 16; };",
            lvalue:       "p->c",
            read:         &["load.i16"],
            write:        &["store.i16"],
        },
        // The qualifier may come from the object.
        Case {
            declarations: "struct S { unsigned a : 8, b : 24; }; extern volatile struct S g;",
            lvalue:       "g.b",
            read:         &["load.i32"],
            write:        I32,
        },
        // Each declared type has its own width, and `_Bool` and an enum count.
        Case {
            declarations: "struct S { volatile _Bool a : 1, b : 1; };",
            lvalue:       "p->b",
            read:         &["load.i8"],
            write:        I8,
        },
        Case {
            declarations: "enum E { A, B }; struct S { volatile enum E a : 2; enum E b : 30; };",
            lvalue:       "p->a",
            read:         &["load.i32"],
            write:        I32,
        },
        Case {
            declarations: "struct S { volatile unsigned a : 4; volatile unsigned char b : 4; \
                           unsigned char d; unsigned short e; };",
            lvalue:       "p->b",
            read:         &["load.i8"],
            write:        I8,
        },
        // A read reaches the neighbors; a write stays in its memory location
        // and loads the byte though it replaces it.
        Case {
            declarations: "struct S { volatile unsigned a : 8; char c; };",
            lvalue:       "p->a",
            read:         &["load.i32"],
            write:        I8,
        },
        Case {
            declarations: "struct S { char c; volatile unsigned a : 8; };",
            lvalue:       "p->a",
            read:         &["load.i32"],
            write:        I8,
        },
        // The region is the smallest integer that holds the run, not the
        // padding after it.
        Case {
            declarations: "struct S { volatile unsigned a : 4; };",
            lvalue:       "p->a",
            read:         &["load.i32"],
            write:        I8,
        },
        Case {
            declarations: "struct S { volatile long long a : 4, b : 4; int c; };",
            lvalue:       "p->b",
            read:         &["load.i64"],
            write:        I8,
        },
        Case {
            declarations: "struct S { volatile unsigned a : 4; volatile unsigned char b : 4; \
                           unsigned char d; unsigned short e; };",
            lvalue:       "p->a",
            read:         &["load.i32"],
            write:        I8,
        },
        // A zero-width field ends a run.
        Case {
            declarations: "struct S { volatile unsigned a : 4; int : 0; volatile unsigned b : 4; \
                           };",
            lvalue:       "p->b",
            read:         &["load.i32"],
            write:        I8,
        },
        // Each field of a union is its own run.
        Case {
            declarations: "struct S { union { volatile unsigned a : 3; volatile unsigned short b \
                           : 5; char c; int d; } u; };",
            lvalue:       "p->u.a",
            read:         &["load.i32"],
            write:        I8,
        },
        // Members of an anonymous record belong to its run.
        Case {
            declarations: "struct S { int z; struct { volatile unsigned a : 4, b : 28; }; };",
            lvalue:       "p->b",
            read:         &["load.i32"],
            write:        I32,
        },
        // A packed record: its alignment is too low, unless a member's
        // offset or the object's own alignment makes up for it.
        Case {
            declarations: "struct S { char c, d, e, f; volatile unsigned b : 4, bb : 28; } \
                           __attribute__((packed));",
            lvalue:       "p->b",
            read:         &["load.i8"],
            write:        I8,
        },
        Case {
            declarations: "struct S { char c, d, e, f; volatile unsigned b : 4, bb : 28; } \
                           __attribute__((packed)); extern struct S g __attribute__((aligned(4)));",
            lvalue:       "g.b",
            read:         &["load.i32"],
            write:        I32,
        },
        Case {
            declarations: "struct S { int x; struct { char c; volatile unsigned short f : 4, g : \
                           12; } __attribute__((packed)) in; };",
            lvalue:       "p->in.f",
            read:         &["load.i16"],
            write:        I8,
        },
        Case {
            declarations: "struct S { char c; volatile unsigned short b : 4; } \
                           __attribute__((aligned(4)));",
            lvalue:       "p->b",
            read:         &["load.i16"],
            write:        I8,
        },
        // A record that ends within the unit, within the object.
        Case {
            declarations: "struct S { char c, d, e, f; volatile unsigned a : 4; } \
                           __attribute__((packed, aligned(4)));",
            lvalue:       "p->a",
            read:         &["load.i32"],
            write:        I8,
        },
    ] {
        check(&case);
    }
}

/// The unit is as aligned as its type, whether it starts at the field or
/// before it, where the field's own address has a lower alignment.
#[test]
fn units_have_the_alignment_of_their_type() {
    for lvalue in ["p->a", "p->b", "p->c"] {
        let source = format!(
            "struct S {{ volatile unsigned a : 8, b : 8, c : 16; }}; int r(struct S *p) {{ \
             {lvalue} = 1; return {lvalue}; }}"
        );
        let configuration = CompilerConfiguration::default().with_code_options(CodeOptions {
            strict_volatile_bit_fields: true,
            ..CodeOptions::default()
        });
        let text = lower_with(&source, configuration).unwrap();
        let accesses: Vec<&str> = text.lines().filter(|l| l.contains(" volatile ")).collect();
        assert_eq!(accesses.len(), 3, "{text}");
        assert!(accesses.iter().all(|l| l.ends_with("align 4")), "{text}");
    }
}

/// When the strict access does not apply: GCC's ordinary access, with
/// volatile's preference for the widest mode that works.
#[test]
fn fallbacks_use_the_accesses_gcc_does() {
    const I8: &[&str] = &["load.i8", "store.i8"];
    const I16: &[&str] = &["load.i16", "store.i16"];
    const I64: &[&str] = &["load.i64", "store.i64"];
    const TWO_I8: &[&str] = &["load.i8", "store.i8", "load.i8", "store.i8"];
    for case in [
        // A write stays in a region of 16 bits: one access, not two bytes.
        Case {
            declarations: "struct S { volatile unsigned a : 12; };",
            lvalue:       "p->a",
            read:         &["load.i32"],
            write:        I16,
        },
        Case {
            declarations: "struct S { volatile unsigned a : 9; char c; short d; };",
            lvalue:       "p->a",
            read:         &["load.i32"],
            write:        I16,
        },
        Case {
            declarations: "struct S { int i; volatile unsigned a : 12; int j; };",
            lvalue:       "p->a",
            read:         &["load.i32"],
            write:        I16,
        },
        Case {
            declarations: "struct S { short c; volatile unsigned a : 12; };",
            lvalue:       "p->a",
            read:         &["load.i32"],
            write:        I16,
        },
        // A region's offset limits its alignment: one byte at a time.
        Case {
            declarations: "struct S { char c; volatile unsigned a : 12; };",
            lvalue:       "p->a",
            read:         &["load.i32"],
            write:        TWO_I8,
        },
        Case {
            declarations: "enum E { A, B }; struct S { char c; volatile enum E a : 12; };",
            lvalue:       "p->a",
            read:         &["load.i32"],
            write:        TWO_I8,
        },
        // The qualifier comes from the access.
        Case {
            declarations: "struct S { char c; unsigned a : 12; };",
            lvalue:       "((volatile struct S *) p)->a",
            read:         &["load.i32"],
            write:        TWO_I8,
        },
        // No mode fits a region of 3 or 5 to 7 bytes: halve the unit at its
        // end.
        Case {
            declarations: "struct S { volatile unsigned a : 17; char c; };",
            lvalue:       "p->a",
            read:         &["load.i32"],
            write:        &["load.i16", "store.i16", "load.i8", "store.i8"],
        },
        Case {
            declarations: "struct S { volatile long a : 41; char c; };",
            lvalue:       "p->a",
            read:         &["load.i64"],
            write:        &["load.i32", "store.i32", "load.i16", "store.i16"],
        },
        Case {
            declarations: "struct S { volatile long a : 49; char c; };",
            lvalue:       "p->a",
            read:         &["load.i64"],
            write:        &[
                "load.i32",
                "store.i32",
                "load.i16",
                "store.i16",
                "load.i8",
                "store.i8",
            ],
        },
        // Packed: as wide as the record's alignment allows, split below that.
        Case {
            declarations: "struct S { volatile unsigned a : 12; } __attribute__((packed));",
            lvalue:       "p->a",
            read:         &["load.i8", "load.i8"],
            write:        TWO_I8,
        },
        Case {
            declarations: "struct S { volatile unsigned a : 12; } __attribute__((packed, \
                           aligned(2)));",
            lvalue:       "p->a",
            read:         &["load.i16"],
            write:        I16,
        },
        Case {
            declarations: "struct S { char c; volatile unsigned a : 12; } __attribute__((packed));",
            lvalue:       "p->a",
            read:         &["load.i8", "load.i8"],
            write:        TWO_I8,
        },
        // A byte-aligned read as wide as a mode is one access, however
        // misaligned; a write only at the declared type's own width.
        Case {
            declarations: "struct S { char c; volatile unsigned a : 16; } __attribute__((packed));",
            lvalue:       "p->a",
            read:         &["load.i16"],
            write:        TWO_I8,
        },
        Case {
            declarations: "struct S { char c; volatile unsigned short a : 16; } \
                           __attribute__((packed));",
            lvalue:       "p->a",
            read:         &["load.i16"],
            write:        &["store.i16"],
        },
        Case {
            declarations: "struct S { char c; volatile long a : 64; } __attribute__((packed));",
            lvalue:       "p->a",
            read:         &["load.i64"],
            write:        &["store.i64"],
        },
        Case {
            declarations: "struct S { char c; volatile unsigned a : 8; } __attribute__((packed));",
            lvalue:       "p->a",
            read:         &["load.i8"],
            write:        I8,
        },
        Case {
            declarations: "struct S { char c : 4; volatile unsigned short a : 16; } \
                           __attribute__((packed));",
            lvalue:       "p->a",
            read:         &["load.i8", "load.i8", "load.i8"],
            write:        &[
                "load.i8", "store.i8", "load.i8", "store.i8", "load.i8", "store.i8",
            ],
        },
        // A field that straddles the unit, here at bits 7 to 32.
        Case {
            declarations: "struct S { char x : 7; volatile unsigned y : 26; } \
                           __attribute__((packed, aligned(4)));",
            lvalue:       "p->y",
            read:         &["load.i32", "load.i32"],
            write:        &["load.i32", "store.i32", "load.i32", "store.i32"],
        },
        // A wider alignment gives a read one wider access, but the write's
        // mode is still the declared type's.
        Case {
            declarations: "struct S { char x : 7; volatile unsigned y : 26; } \
                           __attribute__((packed, aligned(8)));",
            lvalue:       "p->y",
            read:         &["load.i64"],
            write:        &["load.i32", "store.i32", "load.i32", "store.i32"],
        },
        Case {
            declarations: "struct S { char x : 7; volatile unsigned y : 26; } \
                           __attribute__((packed, aligned(2)));",
            lvalue:       "p->y",
            read:         &["load.i16", "load.i16", "load.i16"],
            write:        &[
                "load.i16",
                "store.i16",
                "load.i16",
                "store.i16",
                "load.i8",
                "store.i8",
            ],
        },
        // The outermost record sets the alignment and the position.
        Case {
            declarations: "struct I { char c; volatile unsigned a : 12; } \
                           __attribute__((packed)); struct S { char x; struct I in; } \
                           __attribute__((packed, aligned(4)));",
            lvalue:       "p->in.a",
            read:         &["load.i32"],
            write:        I16,
        },
        Case {
            declarations: "struct I { volatile unsigned a : 4, b : 28; } __attribute__((packed)); \
                           struct S { char x; struct I in; } __attribute__((aligned(4)));",
            lvalue:       "p->in.b",
            read:         &["load.i32", "load.i32"],
            write:        &[
                "load.i8", "store.i8", "load.i8", "store.i8", "load.i8", "store.i8", "load.i8",
                "store.i8",
            ],
        },
        // A write of a register's second byte is 16 bits, however aligned,
        // when both bytes are in the region: not a read.
        Case {
            declarations: "struct S { char m[3]; unsigned char b : 8; volatile unsigned a : 8; \
                           unsigned : 13; };",
            lvalue:       "p->a",
            read:         &["load.i32"],
            write:        I16,
        },
        Case {
            declarations: "struct S { unsigned char b : 8; volatile unsigned a : 8; } \
                           __attribute__((packed));",
            lvalue:       "p->a",
            read:         &["load.i8"],
            write:        I16,
        },
        Case {
            declarations: "struct S { char m[3]; volatile unsigned a : 8; unsigned : 13; };",
            lvalue:       "p->a",
            read:         &["load.i32"],
            write:        I8,
        },
        // A variable index: the stride limits the alignment.
        Case {
            declarations: "struct I { volatile unsigned a : 12; }; struct S { long x; struct I \
                           in[3]; };",
            lvalue:       "p->in[p->x].a",
            read:         &["load.i32"],
            write:        I16,
        },
        // Types wider than a word: a word at a time, a full word stored
        // without a read.
        Case {
            declarations: "struct S { volatile __int128 a : 4; };",
            lvalue:       "p->a",
            read:         &["load.i64"],
            write:        I8,
        },
        Case {
            declarations: "struct S { volatile __int128 a : 64; };",
            lvalue:       "p->a",
            read:         &["load.i64"],
            write:        I64,
        },
        Case {
            declarations: "struct S { volatile __int128 a : 70; };",
            lvalue:       "p->a",
            read:         &["load.i64", "load.i64"],
            write:        &["store.i64", "load.i64", "store.i64"],
        },
        Case {
            declarations: "struct S { long c; volatile __int128 a : 72; long d; };",
            lvalue:       "p->a",
            read:         &["load.i64", "load.i8"],
            write:        &["store.i64", "load.i64", "store.i64"],
        },
        Case {
            declarations: "struct S { char c : 3; volatile __int128 a : 100; } \
                           __attribute__((packed, aligned(8)));",
            lvalue:       "p->a",
            read:         &["load.i64", "load.i64", "load.i64"],
            write:        &[
                "load.i64",
                "store.i64",
                "load.i64",
                "store.i64",
                "load.i64",
                "store.i64",
            ],
        },
        // A small `_BitInt` follows the size of its type.
        Case {
            declarations: "struct S { volatile unsigned _BitInt(40) a : 33; char c; };",
            lvalue:       "p->a",
            read:         &["load.i64"],
            write:        &["load.i32", "store.i32", "load.i8", "store.i8"],
        },
        // The objects' own alignment and size count, where GCC does not raise
        // it.
        Case {
            declarations: "struct S { volatile unsigned a : 12; } __attribute__((packed)); extern \
                           struct S g;",
            lvalue:       "g.a",
            read:         &["load.i8", "load.i8"],
            write:        TWO_I8,
        },
        Case {
            declarations: "struct S { volatile unsigned a : 12; } __attribute__((packed)); extern \
                           struct S g __attribute__((aligned(2)));",
            lvalue:       "g.a",
            read:         &["load.i16"],
            write:        I16,
        },
        Case {
            declarations: "struct S { char c; volatile unsigned a : 12; char d[3]; } \
                           __attribute__((packed)); struct S g;",
            lvalue:       "g.a",
            read:         &["load.i8", "load.i8"],
            write:        TWO_I8,
        },
        Case {
            declarations: "struct S { char c; volatile unsigned a : 12; }; struct S g;",
            lvalue:       "g.a",
            read:         &["load.i32"],
            write:        TWO_I8,
        },
    ] {
        check(&case);
    }
}

/// A compound assignment is the read's accesses, then the write's, and the
/// value of an assignment is not read back.
#[test]
fn compound_assignment_reads_then_writes() {
    let source = "struct S { volatile unsigned a : 12; }; int f(struct S *p) { p->a += 1; return \
                  p->a = 2; }";
    assert_eq!(
        volatile_accesses(source, true),
        ["load.i32", "load.i16", "store.i16", "load.i16", "store.i16"],
    );
}

/// The shapes whose accesses bcc cannot make GCC's are errors under the
/// option, for a read and a write alike, and compile without it.
#[test]
fn unmatched_shapes_are_refused() {
    for (declarations, lvalue) in [
        // GCC's widths and order change with the level and the value.
        ("struct S { volatile __int128 a : 128; };", "p->a"),
        (
            "struct S { char c; volatile __int128 a : 128; } __attribute__((packed));",
            "p->a",
        ),
        // GCC has none or crashes.
        (
            "struct S { volatile unsigned _BitInt(200) a : 190; };",
            "p->a",
        ),
        // GCC reads past the object.
        (
            "struct S { volatile unsigned a : 12; } __attribute__((packed)); extern struct S g \
             __attribute__((aligned(4)));",
            "g.a",
        ),
        // GCC aligns the object further than its type asks, and the access
        // depends on it.
        (
            "struct S { volatile unsigned a : 12; char b[40]; } __attribute__((packed)); struct S \
             g;",
            "g.a",
        ),
        // An array's offset hides its elements' alignment.
        (
            "struct I { volatile unsigned a : 4, b : 28; } __attribute__((packed)); struct S { \
             char x; struct I in[3]; } __attribute__((packed, aligned(4)));",
            "p->in[p->x].b",
        ),
    ] {
        for body in [
            format!("return {lvalue};"),
            format!("{lvalue} = 3; return 0;"),
        ] {
            let source = format!("{declarations} long f(struct S *p) {{ {body} }}");
            let strict = CompilerConfiguration::default().with_code_options(CodeOptions {
                strict_volatile_bit_fields: true,
                ..CodeOptions::default()
            });
            let errors = lower_with(&source, strict).expect_err(&source);
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("volatile bit-field")),
                "{source}: {errors:?}"
            );
            let plain = lower_with(&source, CompilerConfiguration::default());
            assert!(plain.is_ok(), "without the option: {plain:?}\n{source}");
        }
    }
    let local = "struct S { volatile unsigned a : 12; } __attribute__((packed)); long f(void) { \
                 struct S s; s.a = 1; return s.a; }";
    let strict = CompilerConfiguration::default().with_code_options(CodeOptions {
        strict_volatile_bit_fields: true,
        ..CodeOptions::default()
    });
    assert!(lower_with(local, strict).is_err(), "{local}");
}

/// An automatic object GCC may align further still has its accesses when they
/// do not depend on that.
#[test]
fn automatic_objects_keep_accesses_that_do_not_depend_on_their_alignment() {
    let source = "struct S { char c; volatile unsigned a : 12; }; long f(void) { struct S s; s.a \
                  = 1; return s.a; }";
    assert_eq!(
        volatile_accesses(source, true),
        ["load.i8", "store.i8", "load.i8", "store.i8", "load.i32"],
    );
}

#[test]
fn accesses_are_narrow_without_the_option() {
    let source = "struct S { volatile unsigned a : 8, b : 8, c : 16; }; int r(struct S *p) { \
                  return p->b; } void w(struct S *p) { p->b = 3; }";
    assert_eq!(volatile_accesses(source, false), ["load.i8", "store.i8"]);
}

/// A field that is not `volatile` is unchanged by the option.
#[test]
fn ordinary_fields_are_unaffected() {
    let source = "struct S { unsigned a : 8, b : 8, c : 16; }; int r(struct S *p) { return p->b; }";
    let configuration = |strict_volatile_bit_fields| {
        CompilerConfiguration::default().with_code_options(CodeOptions {
            strict_volatile_bit_fields,
            ..CodeOptions::default()
        })
    };
    let plain = lower_with(source, configuration(false)).unwrap();
    assert_eq!(lower_with(source, configuration(true)).unwrap(), plain);
    assert!(plain.contains("load.i8"), "{plain}");
}

/// A signed field shifts up and back within the unit's integer.
#[test]
fn signed_fields_extract_from_the_unit() {
    let source =
        "struct S { volatile int a : 8, b : 8, c : 16; }; int r(struct S *p) { return p->b; }";
    let configuration = CompilerConfiguration::default().with_code_options(CodeOptions {
        strict_volatile_bit_fields: true,
        ..CodeOptions::default()
    });
    let text = lower_with(source, configuration).unwrap();
    for wanted in ["load.i32 volatile", "shl.i32", "ashr.i32"] {
        assert!(text.contains(wanted), "{wanted}\n{text}");
    }
}
