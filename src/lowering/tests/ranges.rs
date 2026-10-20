//! GNU range initializers whose later elements copy the first (issue #82):
//! the copies must leave exactly the bytes, relocations and effects that
//! storing each element does, and lowering work must not grow with the
//! range.
use super::*;
use crate::{
    backend::interpreter::{
        RuntimeValue,
        Termination,
        run_text,
    },
    ir::GlobalInit,
    lowering::initializers::{
        REPLAY_STEPS,
        REPLICATION_MINIMUM,
    },
};

/// `source` lowered with ranges of at least `minimum` elements copied, or
/// with every element replayed for `u64::MAX`.
fn lowered_copying(source: &str, minimum: u64) -> String {
    REPLICATION_MINIMUM.set(Some(minimum));
    let text = lower(source);
    REPLICATION_MINIMUM.set(None);
    text.unwrap_or_else(|errors| panic!("lowering failed: {errors:?}\n{source}"))
}

#[test]
fn static_range_copies_keep_bytes_and_relocations() {
    let sources = [
        "int a[100] = {[0 ... 99] = 3, [50] = 4};",
        "int a[2][3] = {[0 ... 1][1 ... 2] = 5};",
        "int a[] = {[0 ... 9] = 1, [12] = 2};",
        "int a[10] = {[0 ... 7] = 1, [3 ... 9] = 2, [5] = 3};",
        "int a[30][30] = {[0 ... 29][0 ... 29] = 0};",
        "struct S { int x, y; } s[8] = {[1].x = 9, [0 ... 7].y = 2, [3] = {1}};",
        "struct S { int x, y, z; } s[6] = {[2] = {1, 2, 3}, [0 ... 5].y = 4, [4].z = 5};",
        "int g; struct P { char c; int *p; } q[6] = {[0 ... 5] = {1, &g + 1}, [2].p = 0};",
        "int g; int *p[5] = {[3] = &g, [0 ... 4] = &g + 1, [2] = 0};",
        "int g; struct P { int *p; char c; } q[4][3] = {[1] = {[0 ... 2] = {&g}}, [0 ... 3][1].c \
         = 1};",
        "int g; struct P { int *p, *q; } q[5] = {[1].q = &g, [0 ... 4].p = &g};",
        "int f(void); int (*fp[3])(void) = {[0 ... 2] = f};",
        "char s[4][6] = {[0 ... 3] = \"ab\", [1] = \"xyz\"};",
        "char s[5][3] = {[0 ... 4] = {[0 ... 2] = 'q'}, [2][1] = 0};",
        "struct B { unsigned a:3, b:5; int c; } b[5] = {[0 ... 4] = {1, 2, 3}, [1].b = 7};",
        "struct B { unsigned a:3, b:5; int c; } b[4] = {[2].b = 1, [0 ... 3].a = 5};",
        "struct B { unsigned a:4, b:4; } b[6] = {[0 ... 5].a = 1, [0 ... 5].b = 2, [1].a = 3};",
        "union U { int i; char c; } u[4] = {[1].c = 1, [0 ... 3].i = 2};",
        "union U { int i; char c; } u[3] = {[0 ... 2] = {.c = 3}, [1].i = 9};",
        "struct T { int a[3]; } t[4] = {[0 ... 3] = {{[0 ... 2] = 6}}, [2].a[1] = 0};",
        "struct F { int n; int v[]; } f = {1, {[0 ... 3] = 7}};",
        "long double d[4] = {[0 ... 3] = 1.5L, [2] = 2};",
        "_Complex double c[3] = {[0 ... 2] = 1.0};",
        "struct Q { char c; long l; } q[5] = {[0 ... 4] = {1, 2}};",
        "struct E { int x; } e[3][4] = {[0 ... 2] = {[0 ... 3] = {1}}, [1][2] = {}};",
        "int x; void *v[4] = {[0 ... 3] = (char *)&x + 2};",
        "struct R { int n; struct { int v[3]; } in[2]; } r[3] = {[0 ... 2].in[0 ... 1].v[1] = 4};",
    ];
    for source in sources {
        let copied = lowered_copying(source, 2);
        let replayed = lowered_copying(source, u64::MAX);
        pretty_assertions::assert_eq!(copied, replayed, "{source}");
    }
}

#[test]
fn automatic_range_copies_keep_values_and_evaluate_once() {
    let source = "int printf(const char *, ...);
int calls;
int next(void) { return ++calls * 3; }
struct S { int x, y; };
struct S make(void) { struct S s = {calls++, 7}; return s; }
union U { int i; char c; };
struct B { unsigned a:3, b:5; int c; };
unsigned sum(const void *p, unsigned long n) {
  const unsigned char *b = p; unsigned h = 0;
  for (unsigned long i = 0; i < n; ++i) h = h * 31 + b[i];
  return h;
}
int main(void) {
  int a[40] = {[0 ... 39] = next(), [5] = next()};
  int b[6][7] = {[1 ... 4][2 ... 5] = next()};
  struct S c[20] = {[0 ... 19] = make(), [3].y = 1};
  struct S d[20] = {[0 ... 19] = {next()}, [7].x = 4};
  struct S e[20] = {[2].x = 5, [0 ... 19].y = next()};
  char s[20][4] = {[0 ... 19] = \"ab\", [4] = \"c\"};
  union U u[20] = {[0 ... 19] = {.c = 1}};
  union U w[20] = {[3].c = 2, [0 ... 19].i = 0x01020304};
  struct B f[20] = {[0 ... 19] = {1, 2, next()}, [9].b = 3};
  int z[100] = {[0 ... 99] = 0, [50] = 1};
  long g[3][30] = {[0 ... 2] = {[0 ... 29] = next()}, [1][4] = 2};
  printf(\"%u %u %u %u %u %u %u %u %u %u %u %d\\n\", sum(a, sizeof a), sum(b, sizeof b),
         sum(c, sizeof c), sum(d, sizeof d), sum(e, sizeof e), sum(s, sizeof s),
         sum(u, sizeof u), sum(w, sizeof w), sum(z, sizeof z), sum(g, sizeof g),
         f[9].a + f[9].b * 8u + f[19].c * 256u, calls);
  return a[5] != 6 || a[39] != 3 || e[2].x != 5 || e[19].y != 18 || g[2][29] != 24 || calls != 8;
}";
    let mut outputs = Vec::new();
    let mut texts = Vec::new();
    for minimum in [2, u64::MAX] {
        let text = lowered_copying(source, minimum);
        let run = run_text(&text, "main");
        let outcome = run
            .result
            .unwrap_or_else(|trap| panic!("{trap:?} {:?}\n{text}", run.message));
        assert_eq!(
            outcome.termination,
            Termination::Returned(Some(RuntimeValue::Int(0))),
            "{}",
            run.output
        );
        outputs.push(run.output);
        texts.push(text);
    }
    assert_ne!(texts[0], texts[1], "the first lowering copies elements");
    assert_eq!(
        outputs[0], outputs[1],
        "copies print what element stores print"
    );
}

#[test]
fn short_or_volatile_automatic_ranges_store_each_element() {
    let text = lowered("void f(int n) { int a[15] = {[0 ... 14] = n}; }");
    assert!(!text.contains("copy "), "{text}");
    let text = lowered("void f(int n) { volatile int a[64] = {[0 ... 63] = n}; }");
    assert!(!text.contains("copy "), "{text}");
    let text = lowered("void f(int n) { int a[16] = {[0 ... 15] = n}; }");
    assert_eq!(text.matches("copy ").count(), 4, "{text}");
    let text = lowered("void f(void) { int a[64][64] = {[0 ... 63][0 ... 63] = 0}; }");
    assert_eq!(text.matches("fill ").count(), 2, "{text}");
    assert!(!text.contains("copy "), "{text}");
}

#[test]
fn million_element_ranges_lower_in_constant_work() {
    type Shape = fn(u64) -> String;
    let shapes: [(&str, Shape); 4] = [
        ("static", |n| {
            format!("int a[{n}] = {{[0 ... {}] = 1}};", n - 1)
        }),
        ("static rows", |n| {
            let side = n.isqrt();
            format!(
                "int a[{side}][{side}] = {{[0 ... {0}][0 ... {0}] = 0}};",
                side - 1
            )
        }),
        ("static members", |n| {
            format!(
                "struct S {{ int x, y; }} a[{n}] = {{[0 ... {}].y = 2, [7].x = 1}};",
                n - 1
            )
        }),
        ("automatic", |n| {
            format!(
                "int g(int *); int f(int v) {{ int a[{n}] = {{[0 ... {}] = v}}; return g(a); }}",
                n - 1
            )
        }),
    ];
    for (name, shape) in shapes {
        let small = replay_steps(&shape(1_024));
        let large = replay_steps(&shape(1_048_576));
        eprintln!("{name}: {small} replay steps for 2^10 elements, {large} for 2^20");
        assert_eq!(small, large, "{name}");
    }
}

/// Lowers `source` without printing it and returns the plan replay steps.
/// The reviewer's two-range zero array must stay a zero-filled object.
fn replay_steps(source: &str) -> usize {
    let tu = Bump::new();
    let mut context = Context::new(&tu);
    let syntax = parse_translation_unit(
        &mut context,
        Path::new("<test>"),
        tu.alloc_str(source),
        HeaderSearch::default(),
    );
    let sema = analyze_translation_unit(&mut context, &syntax);
    assert!(sema.lowerable(&context), "{source}");
    let arena = Bump::new();
    let mut scratch = Bump::new();
    let before = REPLAY_STEPS.get();
    let module = lower_translation_unit(
        &context,
        &syntax,
        &sema,
        Target::LinuxGnu,
        &arena,
        &mut scratch,
    )
    .unwrap();
    let steps = REPLAY_STEPS.get() - before;
    if source.contains("= 0}") {
        let (_, global) = module.globals().next().unwrap();
        assert!(matches!(global.init, Some(GlobalInit::Zero)), "{source}");
    }
    steps
}

/// An element type for the generated initializers: the designators that
/// reach its members and the values a whole element can get; `1` becomes a
/// call in the automatic object.
#[derive(Clone, Copy)]
struct Element {
    name:    &'static str,
    members: &'static [&'static str],
    values:  &'static [&'static str],
}

const ELEMENTS: [Element; 6] = [
    Element {
        name:    "int",
        members: &[""],
        values:  &["7", "-1", "0", "1"],
    },
    Element {
        name:    "struct P",
        members: &["", ".x", ".y", ".c", ".d"],
        values:  &["3", "1", "{1, 2, 3}", "{.y = 5}", "{}"],
    },
    Element {
        name:    "struct B",
        members: &["", ".a", ".b", ".c"],
        values:  &["1", "0", "{1, 2, 3}", "{.c = 9}"],
    },
    Element {
        name:    "union U",
        members: &["", ".i", ".c[1]", ".c"],
        values:  &["4", "1", "{.c = {1, 2}}", "{.i = 0x01020304}", "{}"],
    },
    Element {
        name:    "struct Q",
        members: &["", ".p", ".n"],
        values:  &["{&g[1], 2}", "0", "{.n = 5}", "{&g[3]}"],
    },
    Element {
        name:    "char",
        members: &[""],
        values:  &["\"ab\"", "\"\"", "{'x'}"],
    },
];

const DECLARATIONS: &str = "int printf(const char *, ...);
int g[4], calls;
int v(void) { return ++calls; }
struct P { int x; short y; char c, d; };
struct B { unsigned a:3, b:5, c:24; };
union U { int i; char c[4]; };
struct Q { int *p; long n; };
void dump(const unsigned char *p, unsigned long n) { unsigned h = 0;
  for (unsigned long i = 0; i < n; i++) h = h * 31 + p[i]; printf(\"%u \", h); }
";

/// Generated initializers of arrays of each element type, with overlapping
/// ranges, single elements and member designators, lower to the same static
/// IR and print the same automatic contents and evaluation counts whether
/// ranges copy their first element or replay each.
#[test]
fn generated_range_initializers_match_element_replay() {
    use std::fmt::Write as _;

    let mut state = 0x0BCC_0082_u64;
    let mut random = |bound: usize| {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        usize::try_from(state >> 33).unwrap() % bound
    };
    for case in 0..120 {
        let kind = random(ELEMENTS.len());
        let Element {
            name,
            members,
            values,
        } = ELEMENTS[kind];
        let strings = kind == 5;
        let rows = [1, 3][random(2)];
        let length = [17, 24, 40][random(3)];
        let mut entries = String::new();
        for _ in 0..2 + random(5) {
            let low = random(length);
            let high = if random(3) == 0 {
                low
            } else {
                low + random(length - low)
            };
            if rows > 1 {
                let row = random(rows);
                let top = row + random(rows - row);
                write!(entries, "[{row} ... {top}]").unwrap();
            }
            write!(entries, "[{low} ... {high}]").unwrap();
            let member = members[random(members.len())];
            let value = if member.is_empty() {
                values[random(values.len())].to_string()
            } else if kind == 4 && member == ".p" {
                "&g[2]".to_string()
            } else if kind == 3 && member == ".c" {
                "{5, 6}".to_string()
            } else {
                (1 + random(9)).to_string()
            };
            write!(entries, "{member} = {value}, ").unwrap();
        }
        let extent = if strings { "[6]" } else { "" };
        let shape = if rows > 1 {
            format!("[{rows}][{length}]{extent}")
        } else {
            format!("[{length}]{extent}")
        };
        let automatic_entries = entries.replace("= 1,", "= v(),");
        let source = format!(
            "{DECLARATIONS}{name} s{shape} = {{{entries}}};\nint main(void) {{ {name} a{shape} = \
             {{{automatic_entries}}}; dump((void *)s, sizeof s); {} printf(\"%d\\n\", calls); \
             return 0; }}",
            if kind == 4 {
                "for (unsigned i = 0; i < sizeof a / sizeof(struct Q); i++) printf(\"%d %ld \", \
                 (int)(((struct Q *)a)[i].p ? ((struct Q *)a)[i].p - g : -1), ((struct Q \
                 *)a)[i].n);"
            } else {
                "dump((void *)a, sizeof a);"
            },
        );
        let copied = lowered_copying(&source, 2);
        let replayed = lowered_copying(&source, u64::MAX);
        let statics = |text: &str| {
            text.lines()
                .filter(|line| line.starts_with("global "))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        pretty_assertions::assert_eq!(
            statics(&copied),
            statics(&replayed),
            "case {case}\n{source}"
        );
        let outputs = [&copied, &replayed].map(|text| {
            let run = run_text(text, "main");
            if let Err(trap) = run.result {
                panic!("case {case} {trap:?} {:?}\n{source}", run.message);
            }
            run.output
        });
        assert_eq!(outputs[0], outputs[1], "case {case}\n{source}");
    }
}
