//! The lessons: demo C programs and what each teaches. These are sources, not
//! execution traces: the Rust engine produces every state, and
//! `web/pipeline/` holds what the real compiler makes of each.

use crate::pipeline::Stage;

pub struct Demo {
    pub key:     &'static str,
    pub title:   &'static str,
    /// One sentence on what to watch for.
    pub summary: &'static str,
    pub topics:  &'static [&'static str],
    /// The part of Table 1 the lesson belongs to. [`DEMOS`] lists each
    /// part's lessons together, in the order of [`Part::ALL`].
    pub part:    Part,
    /// Whether the program ends in a detected fault.
    pub faults:  bool,
    /// The compiler stage a lesson about the compiler opens at. The abstract
    /// machine has no preprocessor, so it does not run the preprocessor's
    /// lesson; it does run the whole-path one.
    pub stage:   Option<Stage>,
    /// For the lesson that follows one program through the whole pipeline,
    /// what each stage of [`Stage::ALL`] does to it at `-O0`, in order; empty
    /// for the others.
    pub path:    &'static [&'static str],
    pub name:    &'static str,
    pub source:  &'static str,
}

/// A part of Table 1: lessons that build on one another, read in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Objects,
    Types,
    Memory,
    Compiler,
}

impl Part {
    pub const ALL: [Self; 4] = [Self::Objects, Self::Types, Self::Memory, Self::Compiler];

    pub fn title(self) -> &'static str {
        match self {
            | Self::Objects => "Objects and control",
            | Self::Types => "Types and conversions",
            | Self::Memory => "Memory and undefined behavior",
            | Self::Compiler => "Inside the compiler",
        }
    }

    /// What the part's lessons have in common, in a few words.
    pub fn summary(self) -> &'static str {
        match self {
            | Self::Objects => "Objects, output, names for values, and calls.",
            | Self::Types =>
                "What each type can hold, and what happens to a value that does not fit.",
            | Self::Memory => "Pointers, structures, and the reads C does not define.",
            | Self::Compiler => "What bcc makes of a program, from its characters to machine code.",
        }
    }
}

/// The lessons, in reading order: each part's lessons together, the parts in
/// the order of [`Part::ALL`].
pub static DEMOS: [Demo; 14] = [
    Demo {
        key:     "tour",
        title:   "Objects, loops and calls",
        summary: "Watch objects appear, a loop run, and a function change the caller's array \
                  through a pointer.",
        topics:  &["objects", "lifetimes", "calls"],
        part:    Part::Objects,
        faults:  false,
        stage:   None,
        path:    &[],
        name:    "objects.c",
        source:  include_str!("../demos/objects.c"),
    },
    Demo {
        key:     "hello",
        title:   "Hello, world: printf and strings",
        summary: "#include <stdio.h> declares printf; a string is an array of chars that ends in \
                  '\\0', and printf fills its format with each argument.",
        topics:  &["printf", "strings", "prototypes"],
        part:    Part::Objects,
        faults:  false,
        stage:   None,
        path:    &[],
        name:    "hello.c",
        source:  include_str!("../demos/hello.c"),
    },
    Demo {
        key:     "constants",
        title:   "Names for values: #define, const and casts",
        summary: "A macro is replaced before the program runs, a const object may not change, a \
                  cast converts a value, and += updates an object in one step.",
        topics:  &["#define", "const", "casts"],
        part:    Part::Objects,
        faults:  false,
        stage:   None,
        path:    &[],
        name:    "constants.c",
        source:  include_str!("../demos/constants.c"),
    },
    Demo {
        key:     "recursion",
        title:   "Recursion and frames",
        summary: "Each call to factorial gets its own frame and its own n; returns unwind them \
                  one by one.",
        topics:  &["call stack", "parameters", "return values"],
        part:    Part::Objects,
        faults:  false,
        stage:   None,
        path:    &[],
        name:    "recursion.c",
        source:  include_str!("../demos/recursion.c"),
    },
    Demo {
        key:     "promotion",
        title:   "Integer promotions",
        summary: "char values are promoted to int before arithmetic, and storing 200 into a char \
                  has an implementation-defined result.",
        topics:  &["char", "promotions", "conversions"],
        part:    Part::Types,
        faults:  false,
        stage:   None,
        path:    &[],
        name:    "promotion.c",
        source:  include_str!("../demos/promotion.c"),
    },
    Demo {
        key:     "unsigned",
        title:   "Unsigned numbers wrap; signed numbers don't",
        summary: "Unsigned arithmetic wraps modulo 2^N and is defined: a countdown wraps to \
                  4294967295, -1 < 1u is false, and sizeof gives an unsigned size_t.",
        topics:  &["unsigned", "conversions", "sizeof"],
        part:    Part::Types,
        faults:  false,
        stage:   None,
        path:    &[],
        name:    "unsigned.c",
        source:  include_str!("../demos/unsigned.c"),
    },
    Demo {
        key:     "overflow",
        title:   "Signed overflow",
        summary: "Adding 1 to INT_MAX is undefined behavior: the machine stops at the exact \
                  addition.",
        topics:  &["undefined behavior", "int range"],
        part:    Part::Types,
        faults:  true,
        stage:   None,
        path:    &[],
        name:    "overflow.c",
        source:  include_str!("../demos/overflow.c"),
    },
    Demo {
        key:     "floating",
        title:   "Floating point: close, not exact",
        summary: "A double holds the nearest binary value, not what you wrote: 0.1 + 0.2 is not \
                  0.3, ten 0.1s make 0.99999999999999989, and (int)2.99 truncates to 2.",
        topics:  &["floating point", "conversions", "rounding"],
        part:    Part::Types,
        faults:  false,
        stage:   None,
        path:    &[],
        name:    "floating.c",
        source:  include_str!("../demos/floating.c"),
    },
    Demo {
        key:     "pointers",
        title:   "Pointers and arrays",
        summary: "An array decays to a pointer, pointer arithmetic counts elements, and one past \
                  the end is a valid place to stop.",
        topics:  &["decay", "pointer arithmetic", "one past the end"],
        part:    Part::Memory,
        faults:  false,
        stage:   None,
        path:    &[],
        name:    "pointers.c",
        source:  include_str!("../demos/pointers.c"),
    },
    Demo {
        key:     "structs",
        title:   "Structs and static storage",
        summary: "Objects outside functions exist for the whole run and start at zero; a \
                  structure's members sit at fixed offsets, with padding, and = copies them all.",
        topics:  &["static storage", "structures", "padding"],
        part:    Part::Memory,
        faults:  false,
        stage:   None,
        path:    &[],
        name:    "structs.c",
        source:  include_str!("../demos/structs.c"),
    },
    Demo {
        key:     "bounds",
        title:   "Reading past the end",
        summary: "Forming a one-past-the-end pointer is fine; dereferencing it is undefined \
                  behavior.",
        topics:  &["undefined behavior", "bounds"],
        part:    Part::Memory,
        faults:  true,
        stage:   None,
        path:    &[],
        name:    "bounds.c",
        source:  include_str!("../demos/bounds.c"),
    },
    Demo {
        key:     "indeterminate",
        title:   "Uninitialized reads",
        summary: "A local without an initializer holds an indeterminate value, not zero.",
        topics:  &["indeterminate values", "initialization"],
        part:    Part::Memory,
        faults:  true,
        stage:   None,
        path:    &[],
        name:    "indeterminate.c",
        source:  include_str!("../demos/indeterminate.c"),
    },
    Demo {
        key:     "path",
        title:   "From source to a running program",
        summary: "Follow one small program through every stage of the compiler, from its \
                  characters to machine code, then watch it run.",
        topics:  &["tokens", "syntax tree", "machine code"],
        part:    Part::Compiler,
        faults:  false,
        stage:   Some(Stage::Lines),
        path:    &PATH,
        name:    "path.c",
        source:  include_str!("../demos/path.c"),
    },
    Demo {
        key:     "macros",
        title:   "Before the compiler: the preprocessor",
        summary: "Follow the source through the first translation phases: lines spliced, comments \
                  become spaces, macros expand with # and ##, and #if drops lines.",
        topics:  &["line splicing", "macros", "conditionals"],
        part:    Part::Compiler,
        faults:  false,
        stage:   Some(Stage::Preprocessor),
        path:    &[],
        name:    "macros.c",
        source:  include_str!("../demos/macros.c"),
    },
];

/// What each stage does to `demos/path.c` at `-O0`, checked against
/// `bcc-rust`'s output for it: one step of the path to the next.
const PATH: [&str; 11] = [
    "This file splices nothing, so each physical line is a logical line, and the comment on line \
     1 is still here, since comments go in phase 3.",
    "The comment on line 1 becomes one space, and `square(side + 1)` is cut into six \
     preprocessing tokens: `square`, `(`, `side`, `+`, `1` and `)`.",
    "This program has no directives and no macros, so phase 4 passes its preprocessing tokens on \
     unchanged.",
    "Now `int`, `void` and `return` are keywords and `3` and `1` are integer constants of type \
     int: 38 tokens in all, each with its line and column.",
    "`square(side + 1)` is a call node with a `+` node as its argument, and `n * n` is a `*` \
     node under `square`'s return statement.",
    "`square` has the type `int (int)`; `n`, `side` and `area` are 4-byte ints, and each read of \
     them is listed as a conversion of kind lvalue.",
    "No variable needs memory here: `side + 1` becomes `iadd.i32 nsw v0, v1` on values, and the \
     call is `call @square(v2)`.",
    "An int argument and an int result need no rewriting on x86-64, so for this program the \
     ABI-lowered IR is the same as the IR.",
    "`n * n` is `mul nsw i32 %v0, %v0`; `nsw` (no signed wrap) tells LLVM the product never \
     overflows, since signed overflow is undefined in C.",
    "`n` arrives in `%rdi`; `movq %rdi, %rax` copies it, `imull %edi, %eax` multiplies the low \
     32-bit halves, and `%eax` returns n * n to main's `callq square`.",
    "`ret` is the single byte c3, and the call is e8 then four zero bytes, where the linker will \
     write the distance to `square`.",
];

#[cfg(test)]
pub fn find(key: &str) -> Option<&'static Demo> {
    DEMOS.iter().find(|demo| demo.key == key)
}

/// The demo whose source is exactly `source`.
pub fn matching(source: &str) -> Option<&'static Demo> {
    DEMOS.iter().find(|demo| demo.source == source)
}

#[cfg(test)]
mod tests {
    use super::{
        DEMOS,
        Part,
    };

    #[test]
    fn each_part_lists_its_lessons_together_in_order() {
        let places: Vec<usize> = DEMOS
            .iter()
            .map(|demo| {
                Part::ALL
                    .iter()
                    .position(|&part| part == demo.part)
                    .expect("a part")
            })
            .collect();
        assert!(places.is_sorted(), "{places:?}");
        for part in Part::ALL {
            assert!(
                DEMOS.iter().any(|demo| demo.part == part),
                "{part:?} has no lesson"
            );
        }
        assert_eq!(DEMOS[0].key, "tour", "the page opens at the first lesson");
    }
}
