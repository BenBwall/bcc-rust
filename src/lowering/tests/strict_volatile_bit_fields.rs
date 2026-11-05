//! `-fstrict-volatile-bitfields`: the width of each access to a `volatile`
//! bit-field. The expected widths are what GCC 13.3 and 14.2 emit for x86-64
//! at `-O2`, except where a comment on the row says how the ordinary access
//! differs: GCC's own has other widths, which this option does not set.

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
        // and falls back. A whole byte is stored without a read here, where
        // GCC also reads it under this option.
        Case {
            declarations: "struct S { volatile unsigned a : 8; char c; };",
            lvalue:       "p->a",
            read:         &["load.i32"],
            write:        &["store.i8"],
        },
        Case {
            declarations: "struct S { char c; volatile unsigned a : 8; };",
            lvalue:       "p->a",
            read:         &["load.i32"],
            write:        &["store.i8"],
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
        // A field that straddles the unit, here at bits 7 to 32, keeps the
        // ordinary access to its own bytes.
        Case {
            declarations: "struct S { char x : 7; volatile unsigned y : 26; } \
                           __attribute__((packed, aligned(4)));",
            lvalue:       "p->y",
            read:         &["load.i32", "load.i8"],
            write:        &[
                "load.i8",
                "store.i8",
                "store.i16",
                "store.i8",
                "load.i8",
                "store.i8",
            ],
        },
        // A type wider than a machine word: GCC's ordinary access is a word.
        Case {
            declarations: "struct S { volatile __int128 a : 4; };",
            lvalue:       "p->a",
            read:         &["load.i8"],
            write:        I8,
        },
        // A record that ends within the unit: GCC reads past it.
        Case {
            declarations: "struct S { volatile unsigned a : 4; } __attribute__((packed)); extern \
                           struct S g __attribute__((aligned(4)));",
            lvalue:       "g.a",
            read:         &["load.i8"],
            write:        I8,
        },
    ] {
        check(&case);
    }
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
