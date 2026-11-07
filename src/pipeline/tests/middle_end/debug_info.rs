//! `-g` through the middle end: what lowering records round-trips through the
//! located text, and what the unit asks for decides what it records, without
//! changing the code.
use super::*;
use crate::{
    configuration::{
        CompilerConfiguration,
        DebugOptions,
    },
    ir::{
        DebugLevel,
        VariablePlace,
        parse_module,
    },
    pipeline::{
        InlineCopies,
        lower_translation_unit_with_locals,
    },
};

const PROGRAM: &str = "\
typedef unsigned long my_size;
struct point { int x; int y; struct point *next; };
enum color { RED, GREEN = 5 };
const int table[2] = { 1, 2 };
#line 40 \"moved.c\"
static int twice(int a) { int t = a * 2; return t; }
int sum(struct point *p, my_size n, enum color c)
{
  int total = c;
  for (my_size i = 0; i < n; i++) {
    int here = twice(p[i].x);
    total += here;
  }
  static int calls;
  calls++;
  return total + table[0];
}
";

fn with_unit<R>(
    debug_info: Option<DebugLevel>,
    stack_locals: bool,
    body: impl FnOnce(&Module<'_>) -> R,
) -> R {
    let tu = Bump::new();
    let configuration = CompilerConfiguration::default()
        .with_debug_info(debug_info.map(|level| DebugOptions {
            level,
            dwarf_version: None,
            columns: true,
        }))
        .with_stack_locals(stack_locals);
    let mut context = Context::with_configuration(&tu, configuration);
    let source = tu.alloc_str(PROGRAM);
    let unit = parse_translation_unit(
        &mut context,
        Path::new("a.c"),
        source,
        HeaderSearch::default(),
    );
    let sema = analyze_translation_unit(&mut context, &unit);
    let arena = Bump::new();
    let diagnosed = lower_translation_unit_with_locals(
        &context,
        &unit,
        &sema,
        &arena,
        None,
        Profile::PostAbi,
        (InlineCopies::Keep, false),
    )
    .unwrap_or_else(|_| panic!("the program lowers"));
    body(&diagnosed.module)
}

#[test]
fn the_located_text_of_a_lowered_unit_round_trips() {
    with_unit(Some(DebugLevel::Full), true, |module| {
        let text = module.display_located().to_string();
        let arena = Bump::new();
        let parsed = parse_module(&arena, &text).unwrap_or_else(|error| panic!("{error}\n{text}"));
        pretty_assertions::assert_eq!(parsed.display_located().to_string(), text);
        pretty_assertions::assert_eq!(parsed.to_string(), module.to_string());
    });
}

#[test]
fn the_logical_file_names_the_code_of_a_line_directive() {
    with_unit(Some(DebugLevel::Lines), false, |module| {
        let text = module.display_located().to_string();
        assert!(text.contains("debug file file1 @moved.c"), "{text}");
        assert!(
            text.contains("@sum name @sum file file1 line 41 scope_line 42"),
            "{text}"
        );
    });
}

/// `-g` adds nothing to the instructions, slots or blocks of a function.
#[test]
fn debug_information_leaves_the_module_as_it_is() {
    let plain = with_unit(None, false, |module| module.to_string());
    let stack = with_unit(None, true, |module| module.to_string());
    for level in [DebugLevel::Lines, DebugLevel::Full, DebugLevel::Macros] {
        pretty_assertions::assert_eq!(
            with_unit(Some(level), false, |module| module.to_string()),
            plain
        );
        pretty_assertions::assert_eq!(
            with_unit(Some(level), true, |module| module.to_string()),
            stack
        );
    }
    assert_ne!(plain, stack, "stack locals change the code, -g or not");
}

#[test]
fn without_g_nothing_is_recorded() {
    with_unit(None, true, |module| {
        assert_eq!(module.debug_info().location_count(), 0);
        assert_eq!(module.debug_info().type_count(), 0);
        for (_, function) in module.functions() {
            if let Some(body) = &function.body {
                assert!(!body.has_locations());
                assert_eq!(body.declared_variables(), &[]);
            }
        }
    });
}

#[test]
fn level_one_records_lines_and_no_data() {
    with_unit(Some(DebugLevel::Lines), true, |module| {
        let debug = module.debug_info();
        assert!(debug.location_count() > 0);
        assert_eq!(debug.type_count(), 0);
        assert_eq!(debug.variables().count(), 0);
        assert_eq!(debug.globals(), &[]);
    });
}

#[test]
fn stack_locals_place_every_variable() {
    with_unit(Some(DebugLevel::Full), true, |module| {
        let debug = module.debug_info();
        for (_, variable) in debug.variables() {
            let placed = module.functions().any(|(_, function)| {
                function.body.as_ref().is_some_and(|body| {
                    body.declared_variables().iter().any(|declared| {
                        debug.variable(declared.variable).map(|data| data.name)
                            == Some(variable.name)
                            && matches!(declared.place, VariablePlace::Slot(_))
                    })
                })
            });
            assert!(placed, "{}", variable.name);
        }
        let names: Vec<&str> = debug.variables().map(|(_, data)| data.name).collect();
        for name in ["a", "t", "p", "n", "c", "total", "i", "here"] {
            assert!(names.contains(&name), "{name}: {names:?}");
        }
        assert!(debug.globals().iter().any(|global| global.name == "calls"));
        assert!(debug.globals().iter().any(|global| global.name == "table"));
    });
}

/// In SSA values a variable keeps its description and has no place, so a
/// debugger reports it optimized out instead of a stale value.
#[test]
fn ssa_variables_are_described_without_a_place() {
    with_unit(Some(DebugLevel::Full), false, |module| {
        let debug = module.debug_info();
        assert!(debug.variables().any(|(_, data)| data.name == "total"));
        let placed = |name: &str| {
            module.functions().any(|(_, function)| {
                function.body.as_ref().is_some_and(|body| {
                    body.declared_variables()
                        .iter()
                        .any(|declared| debug.variable(declared.variable).unwrap().name == name)
                })
            })
        };
        assert!(!placed("total"));
    });
}

#[test]
fn llvm_accepts_the_metadata_on_every_target() {
    for target_flags in [false, true] {
        with_unit(Some(DebugLevel::Full), target_flags, |module| {
            let mut text = String::new();
            crate::backend::llvm::emit_module(module, Target::LinuxGnu, &mut text)
                .unwrap_or_else(|error| panic!("{error}"));
            assert!(text.contains("!DICompileUnit"), "{text}");
            assert!(text.contains("!DILocalVariable"), "{text}");
            assert!(text.contains("!DIGlobalVariable"), "{text}");
            assert!(text.contains("!DILexicalBlock"), "{text}");
            assert_eq!(text.contains("#dbg_declare"), target_flags, "{text}");
        });
    }
}
