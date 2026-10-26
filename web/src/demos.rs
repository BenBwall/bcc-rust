//! The lessons: demo C programs and what each teaches. These are sources, not
//! execution traces: the Rust engine produces every state, and
//! `web/pipeline/` holds what the real compiler makes of each.

pub struct Demo {
    pub key:     &'static str,
    pub title:   &'static str,
    /// One sentence on what to watch for.
    pub summary: &'static str,
    pub topics:  &'static [&'static str],
    /// Whether the program ends in a detected fault.
    pub faults:  bool,
    pub name:    &'static str,
    pub source:  &'static str,
}

pub static DEMOS: [Demo; 7] = [
    Demo {
        key:     "tour",
        title:   "Objects, loops and calls",
        summary: "Watch objects appear, a loop run, and a function change the caller's array \
                  through a pointer.",
        topics:  &["objects", "lifetimes", "calls"],
        faults:  false,
        name:    "objects.c",
        source:  include_str!("../demos/objects.c"),
    },
    Demo {
        key:     "pointers",
        title:   "Pointers and arrays",
        summary: "An array decays to a pointer, pointer arithmetic counts elements, and one past \
                  the end is a valid place to stop.",
        topics:  &["decay", "pointer arithmetic", "one past the end"],
        faults:  false,
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
        name:    "bounds.c",
        source:  include_str!("../demos/bounds.c"),
    },
    Demo {
        key:     "indeterminate",
        title:   "Uninitialized reads",
        summary: "A local without an initializer holds an indeterminate value, not zero.",
        topics:  &["indeterminate values", "initialization"],
        faults:  true,
        name:    "indeterminate.c",
        source:  include_str!("../demos/indeterminate.c"),
    },
];

#[cfg(test)]
pub fn find(key: &str) -> Option<&'static Demo> {
    DEMOS.iter().find(|demo| demo.key == key)
}

/// The demo whose source is exactly `source`.
pub fn matching(source: &str) -> Option<&'static Demo> {
    DEMOS.iter().find(|demo| demo.source == source)
}
