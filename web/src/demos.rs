//! Demo C programs. These are sources, not execution traces: the Rust engine
//! produces every state. The checks in `web/*.mjs` read the same files.

pub struct Demo {
    pub key:    &'static str,
    pub label:  &'static str,
    pub name:   &'static str,
    pub source: &'static str,
}

pub static DEMOS: [Demo; 3] = [
    Demo {
        key:    "tour",
        label:  "Objects, loops & calls",
        name:   "objects.c",
        source: include_str!("../demos/objects.c"),
    },
    Demo {
        key:    "indeterminate",
        label:  "Indeterminate read · policy stop",
        name:   "indeterminate.c",
        source: include_str!("../demos/indeterminate.c"),
    },
    Demo {
        key:    "bounds",
        label:  "UB · array bounds",
        name:   "bounds.c",
        source: include_str!("../demos/bounds.c"),
    },
];

pub fn find(key: &str) -> Option<&'static Demo> {
    DEMOS.iter().find(|demo| demo.key == key)
}
