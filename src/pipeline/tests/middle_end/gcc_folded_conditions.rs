//! What unoptimized code keeps of a condition GCC's front end folds while
//! its side effects still run (`d() && 0`, `d() || 1`), pinned to GCC 13
//! and Clang 23 at `-O0` (`nm` of `-c` output): GCC never references `u`
//! below, nor emits the unused `static inline` function `r`. TinyCC's
//! `tcctest.c` (`optimize_out_test`) links only if `u` stays unreferenced.
//! C99: §6.5.13-§6.5.14, pp. 89-90; PDF pp. 101-102; §6.5.17p2, p. 94;
//! PDF p. 106; §6.8.4.1p2, pp. 133-134; PDF pp. 145-146.
use super::*;
use crate::configuration::CompilerIdentity;

/// What Clang emits of `u`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Clang {
    Omits,
    Emits,
    /// Clang emits the call in a block no path reaches, which bcc's
    /// lowering never keeps.
    EmitsUnreachable,
}

const FORMS: &[(&str, Clang)] = &[
    // tcctest.c `optimize_out_test`.
    ("int i = 0 ? u() : d(); p(i);", Clang::Omits),
    ("int j = 1 ? d() : u(); p(j);", Clang::Omits),
    ("if (0) p(u()); else p(d());", Clang::Omits),
    ("if (1) p(d()); else p(u());", Clang::Omits),
    ("while (1) { p(d()); break; p(u()); }", Clang::Omits),
    ("while (1) { { break; } p(u()); }", Clang::Omits),
    (
        "int j = 1; while (j) { j--; continue; p(u()); }",
        Clang::Omits,
    ),
    ("int i = 0 && u(); p(i);", Clang::Omits),
    ("int i = d() && 0 && u(); p(i);", Clang::Emits),
    ("if (0 && u()) u();", Clang::Omits),
    ("if (d() && 0) u();", Clang::EmitsUnreachable),
    ("if (0 && 0) u();", Clang::Omits),
    ("if (d() && 0 && u()) u();", Clang::EmitsUnreachable),
    ("int i = 1 || u(); p(i);", Clang::Omits),
    ("int i = d() || 1 || u(); p(i);", Clang::Emits),
    ("if (1 || u()) ; else u();", Clang::Omits),
    ("if (d() || 1) ; else u();", Clang::EmitsUnreachable),
    ("if (1 || 1) ; else u();", Clang::Omits),
    ("if (d() || 1 || u()) ; else u();", Clang::EmitsUnreachable),
    ("if (d() && 0) r();", Clang::Emits),
    (
        "if (0) { (void)sizeof(({ do { } while (0); 0; })); u(); }",
        Clang::Omits,
    ),
    (
        "if (0) { switch (d()) { case 0: u(); break; default: u(); break; } }",
        Clang::Omits,
    ),
    ("if (1) return; p(u());", Clang::Omits),
    // The comma, `!` and nested operands GCC folds through.
    ("if ((d(), 0)) u();", Clang::EmitsUnreachable),
    ("int i = (d(), 0) && u(); p(i);", Clang::Emits),
    ("if (!(d() && 0)) ; else u();", Clang::EmitsUnreachable),
    ("int x = d(); int i = x && 0 && u(); p(i);", Clang::Emits),
    ("int i = 1 && d() && 0 && u(); p(i);", Clang::Emits),
    ("int i = (d() && (d() && 0)) && u(); p(i);", Clang::Emits),
    ("if (d() && 0) u(); else p(1);", Clang::EmitsUnreachable),
];

/// The forms where `u` stays live, in both compilers: the folded operand
/// does not decide the operator around it.
const LIVE: &[&str] = &[
    "int i = (d() && 0) || u(); p(i);",
    "int i = (d() || 1) && u(); p(i);",
    "int i; if ((i = d() && 0)) u(); p(i);",
];

fn module_text(body: &str, identity: CompilerIdentity) -> String {
    let source = format!(
        "extern int u(void); extern int d(void); extern void p(int);\nstatic inline void r(void) \
         {{ u(); }}\nvoid f(void) {{ {body} }}\n"
    );
    let target = Target::LinuxGnu;
    with_configured_module(
        &source,
        target,
        false,
        Profile::PreAbi,
        CompilerConfiguration::default()
            .with_target(target)
            .with_compiler_identity(identity)
            .with_gnu_extensions(true),
        |module, _| module.to_string(),
    )
}

#[test]
fn gcc_rules_omit_the_code_a_folded_condition_decides_against() {
    for &(body, clang) in FORMS {
        let text = module_text(body, CompilerIdentity::Gcc);
        assert!(!text.contains("call @u"), "GCC: {body}\n{text}");
        assert!(!text.contains("function @r"), "GCC: {body}\n{text}");
        let text = module_text(body, CompilerIdentity::Clang);
        assert_eq!(
            text.contains("call @u"),
            clang == Clang::Emits,
            "Clang: {body}\n{text}"
        );
    }
    for body in LIVE {
        for identity in [CompilerIdentity::Gcc, CompilerIdentity::Clang] {
            let text = module_text(body, identity);
            assert!(text.contains("call @u"), "{identity:?}: {body}\n{text}");
        }
    }
}

/// The folded condition still runs: `d` is called as often as GCC calls it.
#[test]
fn folded_conditions_keep_their_side_effects() {
    let source = "static int calls, wrong;\nint d(void) { return ++calls; }\nint u(void) { \
                  ++wrong; return 1; }\nstatic inline void r(void) { u(); }\nint main(void) \
                  {\nint i, ok = 1;\ni = d() && 0 && u(); ok &= i == 0 && calls == 1;\ni = d() || \
                  1 || u(); ok &= i == 1 && calls == 2;\nif (d() && 0) r();\nok &= calls == \
                  3;\nif (d() && 0 && u()) u(); else ok &= calls == 4;\nif (d() || 1 || u()) ok \
                  &= calls == 5; else u();\ni = (d(), 0) && u(); ok &= i == 0 && calls == 6;\ni = \
                  !(d() && 0) || u(); ok &= i == 1 && calls == 7;\ni = (d() && (d() && 0)) && \
                  u(); ok &= i == 0 && calls == 9;\n(void)({ d() && 0 && u(); 0; });\nok &= calls \
                  == 10;\nreturn !(ok && wrong == 0);\n}\n";
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        for identity in [CompilerIdentity::Gcc, CompilerIdentity::Clang] {
            for (optimize, profile) in STAGES {
                with_configured_module(
                    source,
                    target,
                    optimize,
                    profile,
                    CompilerConfiguration::default()
                        .with_target(target)
                        .with_compiler_identity(identity)
                        .with_gnu_extensions(true),
                    |module, target| {
                        assert_eq!(interpret(module, target).0, 0, "{identity:?}\n{module}");
                    },
                );
            }
        }
    }
}
