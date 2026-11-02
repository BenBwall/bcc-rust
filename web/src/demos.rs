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

pub static DEMOS: [Demo; 9] = [
    Demo {
        key:     "tour",
        title:   "Objects, loops and calls",
        summary: "Watch objects appear, a loop run, and a function change the caller's array \
                  through a pointer.",
        topics:  &["objects", "lifetimes", "calls"],
        faults:  false,
        stage:   None,
        path:    &[],
        name:    "objects.c",
        source:  include_str!("../demos/objects.c"),
    },
    Demo {
        key:     "path",
        title:   "From source to a running program",
        summary: "Follow one small program through every stage of the compiler, from its \
                  characters to machine code, then watch it run.",
        topics:  &["tokens", "syntax tree", "machine code"],
        faults:  false,
        stage:   Some(Stage::Lines),
        path:    &PATH,
        name:    "path.c",
        source:  include_str!("../demos/path.c"),
    },
    Demo {
        key:     "pointers",
        title:   "Pointers and arrays",
        summary: "An array decays to a pointer, pointer arithmetic counts elements, and one past \
                  the end is a valid place to stop.",
        topics:  &["decay", "pointer arithmetic", "one past the end"],
        faults:  false,
        stage:   None,
        path:    &[],
        name:    "pointers.c",
        source:  include_str!("../demos/pointers.c"),
    },
    Demo {
        key:     "recursion",
        title:   "Recursion and frames",
        summary: "Each call to factorial gets its own frame and its own n; returns unwind them \
                  one by one.",
        topics:  &["call stack", "parameters", "return values"],
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
        faults:  false,
        stage:   None,
        path:    &[],
        name:    "promotion.c",
        source:  include_str!("../demos/promotion.c"),
    },
    Demo {
        key:     "overflow",
        title:   "Signed overflow",
        summary: "Adding 1 to INT_MAX is undefined behavior: the machine stops at the exact \
                  addition.",
        topics:  &["undefined behavior", "int range"],
        faults:  true,
        stage:   None,
        path:    &[],
        name:    "overflow.c",
        source:  include_str!("../demos/overflow.c"),
    },
    Demo {
        key:     "bounds",
        title:   "Reading past the end",
        summary: "Forming a one-past-the-end pointer is fine; dereferencing it is undefined \
                  behavior.",
        topics:  &["undefined behavior", "bounds"],
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
        faults:  true,
        stage:   None,
        path:    &[],
        name:    "indeterminate.c",
        source:  include_str!("../demos/indeterminate.c"),
    },
    Demo {
        key:     "macros",
        title:   "Before the compiler: the preprocessor",
        summary: "Follow the source through the first translation phases: lines spliced, comments \
                  become spaces, macros expand with # and ##, and #if drops lines.",
        topics:  &["line splicing", "macros", "conditionals"],
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
