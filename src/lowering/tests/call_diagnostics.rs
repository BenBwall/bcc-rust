//! GNU error/warning declarations and emitted-call reachability.
use super::*;
use crate::{
    diagnostics::ToDiagnostic,
    translation_phases::GetSourceVectors,
};

fn diagnostics(source: &str) -> (Vec<String>, Vec<(bool, String)>) {
    diagnostics_with_optimizer(source, false)
}

fn diagnostics_with_optimizer(source: &str, optimized: bool) -> (Vec<String>, Vec<(bool, String)>) {
    diagnostics_with_passes(source, optimized, None)
}

fn diagnostics_with_passes(
    source: &str,
    optimized: bool,
    passes: Option<&str>,
) -> (Vec<String>, Vec<(bool, String)>) {
    diagnostics_for_target(source, optimized, passes, Target::LinuxGnu)
}

fn diagnostics_for_target(
    source: &str,
    optimized: bool,
    passes: Option<&str>,
    target: Target,
) -> (Vec<String>, Vec<(bool, String)>) {
    let tu = Bump::new();
    let configuration = crate::configuration::CompilerConfiguration::default().with_target(target);
    let mut context = Context::with_configuration(&tu, configuration);
    let unit = parse_translation_unit(
        &mut context,
        Path::new("<test>"),
        tu.alloc_str(source),
        HeaderSearch::default(),
    );
    let sema = analyze_translation_unit(&mut context, &unit);
    let pending = context.take_pending_errors();
    let errors = pending
        .iter()
        .map(|e| {
            let source = e.source_vectors(&mut context);
            e.diagnostic_in(&context, source, &tu).message.to_owned()
        })
        .collect::<Vec<_>>();
    if !sema.lowerable(&context) {
        return (errors, vec![]);
    }
    let ir = Bump::new();
    let mut scratch = Bump::new();
    let mut result = lower_with_sources(&context, &unit, &sema, target, &ir, &mut scratch, true)
        .expect("source lowers");
    if optimized {
        scratch.reset();
        let mut options = crate::optimizer::OptimizerOptions::default();
        options.passes = passes.map(|passes| crate::optimizer::parse_pass_list(passes).unwrap());
        _ = crate::optimizer::optimize(&mut result.module, &options, &scratch);
    }
    scratch.reset();
    let calls = retained_call_diagnostics(&result.module, result.call_diagnostics, &ir, &scratch)
        .iter()
        .map(|d| (d.warning, context.string_cache.at(d.message).to_owned()))
        .collect();
    (errors, calls)
}

#[test]
fn diagnostic_attributes_follow_selected_overloads_and_target_abi_names() {
    let source = r#"
int selected(int) __attribute__((overloadable,error("integer path")));
int selected(double) __attribute__((overloadable,warning("floating path")));
int plain(int n) __attribute__((enable_if(n>0,"positive"),error("plain path")));
int f(void) { return selected(1)+selected(2.0)+plain(1); }
"#;
    for target in [
        Target::LinuxGnu,
        Target::LinuxMusl,
        Target::WindowsGnu,
        Target::WindowsMsvc,
    ] {
        for optimized in [false, true] {
            let (errors, calls) = diagnostics_for_target(source, optimized, None, target);
            assert!(errors.is_empty(), "{target:?}: {errors:?}");
            assert_eq!(calls.len(), 3, "{target:?}: {calls:?}");
            for (warning, suffix) in [
                (false, "integer path"),
                (true, "floating path"),
                (false, "plain path"),
            ] {
                assert_eq!(
                    calls
                        .iter()
                        .filter(|(actual, message)| *actual == warning && message.ends_with(suffix))
                        .count(),
                    1,
                    "{target:?}: {calls:?}"
                );
            }
        }
    }
}

/// glibc's `open` wrapper guards an `error` function with the argument
/// pack's length, which is a constant in each call's copy: the guarded call
/// is gone, without the optimizer too, unless the call passes too many. A
/// copy requested only from such a branch is not lowered at all.
/// GNU extension: GCC manual, "Constructing Calls".
#[test]
fn argument_pack_length_guards_fold_before_call_diagnostics() {
    for optimized in [false, true] {
        let (errors, calls) = diagnostics_with_optimizer(
            r#"
extern void too_many(void) __attribute__((error("too many arguments")));
extern int __open_2(const char *, int);
extern int __open_alias(const char *, int, ...) __asm__("open");
extern __inline __attribute__((always_inline, gnu_inline)) int
open(const char *path, int flags, ...) {
    if (__builtin_va_arg_pack_len() > 1) too_many();
    if (__builtin_va_arg_pack_len() < 1) return __open_2(path, flags);
    return __open_alias(path, flags, __builtin_va_arg_pack());
}
static inline __attribute__((always_inline)) int never(int n, ...) {
    too_many(); return n;
}
static inline __attribute__((always_inline)) int guarded(int n, ...) {
    if (__builtin_va_arg_pack_len() > 1) return never(n, __builtin_va_arg_pack());
    return n;
}
int f(void) { return open("a", 0) + open("a", 0101, 0600) + guarded(1, 2); }
int g(void) { return open("a", 0101, 0600, 1); }
"#,
            optimized,
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert!(
            matches!(
                &calls[..],
                [(false, message)] if message.ends_with("too many arguments")
            ),
            "optimized: {optimized}: {calls:?}"
        );
    }
}

#[test]
fn diagnostic_attributes_only_diagnose_emitted_direct_calls() {
    let (errors, calls) = diagnostics(
        r#"
int bad(void) __attribute__((error("boom")));
int warn(void) __attribute__((warning("careful")));
static int unused(void) { return bad(); }
int f(void) {
    if (0) bad();
    (void)sizeof(bad());
    __typeof__(bad()) x;
    int (*pointer)(void)=bad;
    pointer(); (*bad)(); (&bad)();
    bad(); (bad)(); warn();
    return 0;
    bad();
}
"#,
    );
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(
        calls.iter().filter(|(warning, _)| !warning).count(),
        2,
        "{calls:?}"
    );
    assert_eq!(
        calls.iter().filter(|(warning, _)| *warning).count(),
        1,
        "{calls:?}"
    );
    assert!(
        calls
            .iter()
            .all(|(_, message)| message.ends_with("boom") || message.ends_with("careful"))
    );
}

#[test]
fn diagnostic_attributes_merge_messages_and_respect_namespace_spelling() {
    let (errors, calls) = diagnostics(
        r#"
[[gnu::error(("first" " message"))]] int bad(void);
int bad(void) __attribute__((error("first message")));
int bad(void);
[[clang::warning("unknown")]] int ordinary(void);
int f(void) { ordinary(); return bad(); }
"#,
    );
    assert_eq!(
        calls,
        [(
            false,
            "call to 'bad' declared with 'error' attribute: first message".to_owned()
        )]
    );
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("unknown attribute"), "{errors:?}");
}

#[test]
fn diagnostic_attribute_declaration_constraints_are_errors() {
    for source in [
        "int object __attribute__((error(\"bad\")));",
        "typedef void Function(void) __attribute__((warning(\"bad\")));",
        "void (*pointer)(void) __attribute__((error(\"bad\")));",
        "void f(void) __attribute__((error));",
        "void f(void) __attribute__((warning(1)));",
        "void f(void) __attribute__((error(\"a\",\"b\")));",
        "void f(void); void f(void) __attribute__((error(\"late\")));",
        "void f(void){} void f(void) __attribute__((error(\"late\")));",
        "void f(void) __attribute__((error(\"bad\"))); void f(void) \
         __attribute__((warning(\"bad\")));",
    ] {
        let tu = Bump::new();
        let mut context = Context::new(&tu);
        let unit = parse_translation_unit(
            &mut context,
            Path::new("<test>"),
            tu.alloc_str(source),
            HeaderSearch::default(),
        );
        let sema = analyze_translation_unit(&mut context, &unit);
        assert!(!sema.lowerable(&context), "{source}");
    }
}

#[test]
fn changing_a_diagnostic_message_warns_and_retains_the_first() {
    let (errors, calls) = diagnostics(
        r#"
int bad(void) __attribute__((error("first")));
int bad(void) __attribute__((error("second")));
int f(void) { return bad(); }
"#,
    );
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("different arguments"), "{errors:?}");
    assert_eq!(
        calls,
        [(
            false,
            "call to 'bad' declared with 'error' attribute: first".to_owned()
        )]
    );
}

#[test]
fn diagnostic_attributes_follow_calls_through_optimization_and_inlining() {
    for source in [
        "int bad(void) __attribute__((error(\"boom\"))); static int zero(void){return 0;} int \
         f(void){if(zero()) return bad(); return 1;}",
        "int bad(void) __attribute__((error(\"boom\"))); static int helper(int n){if(n) return \
         bad(); return 0;} int f(void){return helper(0);}",
        "int bad(void) __attribute__((error(\"boom\"))); int bad(void){return 1;} int \
         f(void){return bad();}",
    ] {
        let (errors, plain) = diagnostics_with_optimizer(source, false);
        assert!(errors.is_empty(), "{source}: {errors:?}");
        assert_eq!(plain.len(), 1, "{source}: {plain:?}");
        let (errors, optimized) = diagnostics_with_optimizer(source, true);
        assert!(errors.is_empty(), "{source}: {errors:?}");
        assert!(optimized.is_empty(), "{source}: {optimized:?}");
    }
    let source = "int bad(void) __attribute__((error(\"boom\"))); static int helper(int n){if(n) \
                  return bad(); return 0;} int f(void){return helper(1);}";
    let (errors, calls) = diagnostics_with_optimizer(source, true);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(calls.len(), 1, "{calls:?}");
}

#[test]
fn diagnostic_attribute_warning_honors_group_controls() {
    use crate::{
        configuration::{
            CompilerConfiguration,
            DiagnosticGroup,
            WarningOption,
        },
        translation_phases::ErrorSeverity,
    };
    let base = CompilerConfiguration::default();
    for (configuration, expected) in [
        (base, Some(ErrorSeverity::Warning)),
        (
            base.with_warning_option(
                DiagnosticGroup::AttributeWarning,
                WarningOption::Enable(false),
            ),
            None,
        ),
        (
            base.with_warning_option(
                DiagnosticGroup::AttributeWarning,
                WarningOption::Error(true),
            ),
            Some(ErrorSeverity::Error),
        ),
        (
            base.with_warnings_as_errors(true),
            Some(ErrorSeverity::Error),
        ),
    ] {
        let tu = Bump::new();
        let mut context = Context::with_configuration(&tu, configuration);
        let source =
            "int warn(void) __attribute__((warning(\"careful\"))); int f(void){return warn();}";
        let unit = parse_translation_unit(
            &mut context,
            Path::new("<test>"),
            source,
            HeaderSearch::default(),
        );
        let sema = analyze_translation_unit(&mut context, &unit);
        assert!(context.take_pending_errors().is_empty());
        let ir = Bump::new();
        let mut scratch = Bump::new();
        let result = lower_with_sources(
            &context,
            &unit,
            &sema,
            Target::LinuxGnu,
            &ir,
            &mut scratch,
            true,
        )
        .unwrap();
        assert_eq!(result.call_diagnostics.len(), 1);
        result.call_diagnostics[0].report(&mut context);
        let pending = context.take_pending_errors();
        assert_eq!(pending.len(), usize::from(expected.is_some()));
        if let Some(expected) = expected {
            assert_eq!(pending[0].configured_severity(&context), expected);
            let source = pending[0].source_vectors(&mut context);
            assert!(
                pending[0]
                    .diagnostic_in(&context, source, &tu)
                    .message
                    .contains("[-Wattribute-warning]")
            );
        }
    }
}

#[test]
fn diagnostic_attributes_follow_unevaluated_function_selection() {
    let (errors, calls) = diagnostics(
        r#"
int bad(void) __attribute__((error("boom")));
int f(void){
    (__extension__ bad)();
    _Generic(0,int:bad)();
    __builtin_choose_expr(1,bad,bad)();
    (*bad)(); (&bad)(); ((int (*)(void))bad)(); ((void)0,bad)();
    return 0;
}
"#,
    );
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(calls.len(), 3, "{calls:?}");
}

#[test]
fn diagnostic_attributes_ignore_dead_blocks_before_cfg_cleanup() {
    for source in [
        "int bad(void) __attribute__((error(\"boom\"))); int f(int n){if(n^n)return bad();return \
         1;}",
        "int bad(void) __attribute__((error(\"boom\"))); static int helper(void){return bad();} \
         int f(int n){if(n^n)return helper();return 1;}",
    ] {
        let (errors, calls) = diagnostics_with_passes(source, true, Some("promote,fold"));
        assert!(errors.is_empty(), "{source}: {errors:?}");
        assert!(calls.is_empty(), "{source}: {calls:?}");
    }
}

#[test]
fn diagnostic_attributes_keep_each_inlined_call() {
    let source = "int warn(void) __attribute__((warning(\"careful\"))); static int \
                  helper(void){return warn();} int f(void){return helper();} int g(void){return \
                  helper();}";
    let (errors, plain) = diagnostics_with_optimizer(source, false);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(plain.len(), 1, "{plain:?}");
    let (errors, optimized) = diagnostics_with_optimizer(source, true);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(optimized.len(), 2, "{optimized:?}");
}
