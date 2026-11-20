use super::*;
#[test]
fn all_modes_derive_acceptance_and_native_features() {
    for standard in [
        CStandard::C89,
        CStandard::C95,
        CStandard::C99,
        CStandard::C11,
        CStandard::C17,
        CStandard::C23,
        CStandard::C2y,
    ] {
        for gnu in [false, true] {
            let configuration = CompilerConfiguration::new(standard, ExtensionPolicy::Allow)
                .with_gnu_extensions(gnu);
            for &feature in Feature::ALL {
                if feature == Feature::Trigraphs {
                    continue;
                }
                assert_eq!(
                    configuration.is_native(feature),
                    configuration.origin_is_native(feature.origin())
                );
            }
            assert_eq!(
                configuration.accepts(Feature::LineComments),
                standard >= CStandard::C99 || gnu
            );
            assert_eq!(
                configuration.accepts(Feature::Digraphs),
                standard >= CStandard::C95 || gnu
            );
            assert_eq!(
                configuration.accepts(Feature::Trigraphs),
                standard <= CStandard::C17 && !gnu
            );
            assert_eq!(
                configuration.accepts(Feature::Inline),
                standard >= CStandard::C99 || gnu
            );
            assert_eq!(
                configuration.accepts(Feature::Restrict),
                standard >= CStandard::C99
            );
            assert_eq!(
                configuration.accepts(Feature::C23Keywords),
                standard >= CStandard::C23
            );
            assert_eq!(
                configuration.accepts(Feature::UnicodeLiteralPrefixes),
                standard >= CStandard::C11
            );
            // GCC's GNU modes take them from gnu99 on.
            assert_eq!(
                configuration
                    .with_compiler_identity(CompilerIdentity::Gcc)
                    .accepts(Feature::UnicodeLiteralPrefixes),
                standard >= CStandard::C11 || (gnu && standard >= CStandard::C99)
            );
            assert_eq!(
                configuration.is_native(Feature::ImplicitInt),
                standard < CStandard::C99
            );
            assert_eq!(
                configuration.is_native(Feature::Trigraphs),
                standard <= CStandard::C17
            );
            assert!(configuration.accepts(Feature::StaticAssert));
            assert_eq!(
                configuration.accepts(Feature::CaseRanges),
                standard >= CStandard::C2y || gnu
            );
            assert_eq!(
                configuration.accepts(Feature::OldStyleFunctionDeclarators),
                standard < CStandard::C23
            );
            assert_eq!(configuration.accepts(Feature::VoidExpressionReturn), gnu);
            for (feature, since) in [
                (Feature::OmittedVariadicArguments, CStandard::C23),
                (Feature::ArrayParameterSyntax, CStandard::C99),
                (Feature::TypedefRedefinition, CStandard::C11),
                (Feature::ForNonVariableDeclarations, CStandard::C23),
            ] {
                assert!(configuration.accepts(feature));
                assert_eq!(configuration.is_native(feature), standard >= since);
            }
            assert!(configuration.accepts(Feature::AlignofExpression));
            assert_eq!(configuration.is_native(Feature::AlignofExpression), gnu);
            assert!(!configuration.accepts(Feature::MsSeh));
        }
    }
}
#[test]
fn msvc_groups_are_independent_and_recomputed() {
    let all = CompilerConfiguration::default().with_msvc_extensions(true);
    for feature in MsvcFeature::ALL {
        assert!(all.msvc_feature(feature));
    }
    let without_seh = all.with_msvc_feature(MsvcFeature::Seh, false);
    assert!(!without_seh.accepts(Feature::MsSeh));
    assert!(without_seh.accepts(Feature::MsDeclspec));
    assert!(!without_seh.gnu_extensions());
    assert_eq!(without_seh.standard(), CStandard::C99);
    assert!(!all.with_msvc_extensions(false).accepts(Feature::MsDeclspec));
}
/// The `_FloatN` keywords come with a GCC identity, as GCC 7 and later
/// provide them, and `-f[no-]floatn-types` overrides the identity whichever
/// comes first.
#[test]
fn interchange_float_types_follow_the_identity_unless_chosen() {
    let base = CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Allow);
    for (identity, expected) in [
        (CompilerIdentity::Clang, false),
        (CompilerIdentity::Gcc, true),
        (CompilerIdentity::Msvc, false),
        (CompilerIdentity::ClangCl, false),
        (CompilerIdentity::Tcc, false),
    ] {
        let configuration = base.with_compiler_identity(identity);
        assert_eq!(
            configuration.interchange_float_types(),
            expected,
            "{identity:?}"
        );
        assert_eq!(
            configuration.accepts(Feature::InterchangeFloatTypes),
            expected,
            "{identity:?}"
        );
        for chosen in [false, true] {
            for configuration in [
                base.with_interchange_float_types(chosen)
                    .with_compiler_identity(identity),
                base.with_compiler_identity(identity)
                    .with_interchange_float_types(chosen),
            ] {
                assert_eq!(
                    configuration.accepts(Feature::InterchangeFloatTypes),
                    chosen,
                    "{identity:?}"
                );
            }
        }
    }
}

#[test]
fn dialect_aliases_are_complete() {
    for (alias, standard, gnu) in [
        ("c89", CStandard::C89, false),
        ("c90", CStandard::C89, false),
        ("iso9899:1990", CStandard::C89, false),
        ("iso9899:199409", CStandard::C95, false),
        ("gnu89", CStandard::C89, true),
        ("gnu90", CStandard::C89, true),
        ("c99", CStandard::C99, false),
        ("c9x", CStandard::C99, false),
        ("iso9899:1999", CStandard::C99, false),
        ("iso9899:199x", CStandard::C99, false),
        ("gnu99", CStandard::C99, true),
        ("gnu9x", CStandard::C99, true),
        ("c11", CStandard::C11, false),
        ("c1x", CStandard::C11, false),
        ("iso9899:2011", CStandard::C11, false),
        ("gnu11", CStandard::C11, true),
        ("gnu1x", CStandard::C11, true),
        ("c17", CStandard::C17, false),
        ("c18", CStandard::C17, false),
        ("iso9899:2017", CStandard::C17, false),
        ("iso9899:2018", CStandard::C17, false),
        ("gnu17", CStandard::C17, true),
        ("gnu18", CStandard::C17, true),
        ("c23", CStandard::C23, false),
        ("c2x", CStandard::C23, false),
        ("iso9899:2024", CStandard::C23, false),
        ("gnu23", CStandard::C23, true),
        ("gnu2x", CStandard::C23, true),
        ("c2y", CStandard::C2y, false),
        ("gnu2y", CStandard::C2y, true),
    ] {
        assert_eq!(
            LanguageMode::parse(alias),
            Some(LanguageMode { standard, gnu }),
            "{alias}"
        );
    }
    assert_eq!(LanguageMode::parse("c98"), None);
}

/// `derive_features` walks `Feature::ALL`, so a variant missing from it
/// would never be accepted. Each entry sits at its discriminant.
#[test]
fn feature_lists_name_every_variant_in_order() {
    for (index, feature) in Feature::ALL.iter().enumerate() {
        assert_eq!(*feature as usize, index, "{feature:?}");
    }
    assert_eq!(
        Feature::ALL.last().map(|feature| *feature as usize),
        Some(Feature::GccOnlyBuiltins as usize)
    );
    for (index, feature) in MsvcFeature::ALL.iter().enumerate() {
        assert_eq!(*feature as usize, index, "{feature:?}");
    }
    assert_eq!(MsvcFeature::ALL.last(), Some(&MsvcFeature::VaArgs));
}

/// The identity policy: GCC and TCC follow GCC's rules and Clang and
/// clang-cl Clang's; MSVC follows GCC's, or Clang's where `cl` is known to
/// match Clang's model of it. The rules chosen per identity before the
/// policy agree with it.
#[test]
fn identities_follow_the_rule_families() {
    let base = CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Allow);
    for (identity, rules, ms_compatible) in [
        (CompilerIdentity::Clang, RuleSet::Clang, RuleSet::Clang),
        (CompilerIdentity::Gcc, RuleSet::Gcc, RuleSet::Gcc),
        (CompilerIdentity::Msvc, RuleSet::Gcc, RuleSet::Clang),
        (CompilerIdentity::ClangCl, RuleSet::Clang, RuleSet::Clang),
        (CompilerIdentity::Tcc, RuleSet::Gcc, RuleSet::Gcc),
    ] {
        assert_eq!(identity.rules(), rules, "{identity:?}");
        assert_eq!(
            identity.ms_compatible_rules(),
            ms_compatible,
            "{identity:?}"
        );
        let configuration = base.with_compiler_identity(identity);
        assert_eq!(
            configuration.folds_compound_literals(),
            rules == RuleSet::Clang,
            "{identity:?}"
        );
        if configuration.half_precision_types() {
            assert_eq!(
                configuration.half_in_float(),
                rules == RuleSet::Gcc,
                "{identity:?}"
            );
        }
    }
}

/// One option sequence's severity for `group` under `identity` and
/// `standard`, each option a `-W` group option or `-W[no-]all` (`None`).
fn permerror_severity(
    identity: CompilerIdentity,
    standard: CStandard,
    group: DiagnosticGroup,
    options: &[(Option<DiagnosticGroup>, bool)],
) -> Option<crate::translation_phases::ErrorSeverity> {
    options
        .iter()
        .fold(
            CompilerConfiguration::new(standard, ExtensionPolicy::Allow)
                .with_gnu_extensions(true)
                .with_compiler_identity(identity),
            |configuration, &(option, enabled)| match option {
                | Some(group) =>
                    configuration.with_warning_option(group, WarningOption::Enable(enabled)),
                | None => configuration.with_all_warnings(enabled),
            },
        )
        .diagnostic_severity(group)
}

/// Re-enabling a disabled group gives Clang a warning and GCC its default
/// error back; `-Wall` yields to a group's own option in GCC whatever the
/// order and only to a later one in Clang; `-Wno-all` silences the implicit
/// groups in both (and Clang's `-Wreturn-mismatch`), while any `-W[no-]all`
/// makes GCC 14 warn about C89's `return;` (probed with GCC 14.2's `cc1` and
/// Clang 23).
#[test]
fn permerror_options_follow_each_compiler() {
    use CompilerIdentity::{
        Clang,
        Gcc,
    };
    use DiagnosticGroup as G;

    use crate::translation_phases::ErrorSeverity::{
        Error,
        Warning,
    };
    let implicit = Some(G::ImplicitFunctionDeclaration);
    let all = None;
    for (identity, standard, group, options, expected) in [
        (
            Gcc,
            CStandard::C17,
            G::ImplicitFunctionDeclaration,
            &[(implicit, false), (implicit, true)][..],
            Some(Error),
        ),
        (
            Clang,
            CStandard::C17,
            G::ImplicitFunctionDeclaration,
            &[(implicit, false), (implicit, true)],
            Some(Warning),
        ),
        (
            Gcc,
            CStandard::C17,
            G::ImplicitFunctionDeclaration,
            &[(implicit, false), (all, true)],
            None,
        ),
        (
            Clang,
            CStandard::C17,
            G::ImplicitFunctionDeclaration,
            &[(implicit, false), (all, true)],
            Some(Warning),
        ),
        (
            Gcc,
            CStandard::C17,
            G::ImplicitFunctionDeclaration,
            &[(all, true), (implicit, false)],
            None,
        ),
        (
            Clang,
            CStandard::C17,
            G::ImplicitFunctionDeclaration,
            &[(all, true), (implicit, false)],
            None,
        ),
        (
            Gcc,
            CStandard::C17,
            G::ImplicitFunctionDeclaration,
            &[(all, false)],
            None,
        ),
        (
            Clang,
            CStandard::C17,
            G::ImplicitFunctionDeclaration,
            &[(all, false)],
            None,
        ),
        (
            Gcc,
            CStandard::C17,
            G::ImplicitFunctionDeclaration,
            &[(all, false), (all, true)],
            Some(Error),
        ),
        (
            Clang,
            CStandard::C17,
            G::ImplicitFunctionDeclaration,
            &[(all, false), (all, true)],
            Some(Warning),
        ),
        (
            Gcc,
            CStandard::C89,
            G::ImplicitFunctionDeclaration,
            &[(all, true)],
            Some(Warning),
        ),
        (
            Gcc,
            CStandard::C89,
            G::ImplicitFunctionDeclaration,
            &[(all, true), (all, false)],
            None,
        ),
        (
            Clang,
            CStandard::C89,
            G::ImplicitFunctionDeclaration,
            &[(all, true), (all, false)],
            None,
        ),
        (
            Gcc,
            CStandard::C89,
            G::ImplicitInt,
            &[(all, true)],
            Some(Warning),
        ),
        (Gcc, CStandard::C89, G::ReturnMismatch, &[], None),
        (
            Gcc,
            CStandard::C89,
            G::ReturnMismatch,
            &[(all, false)],
            Some(Warning),
        ),
        (
            Gcc,
            CStandard::C17,
            G::ReturnMismatch,
            &[(all, false)],
            Some(Error),
        ),
        (
            Clang,
            CStandard::C17,
            G::ReturnMismatch,
            &[(all, false)],
            None,
        ),
        (
            Gcc,
            CStandard::C17,
            G::VoidReturnMismatch,
            &[(all, false)],
            Some(Error),
        ),
        (
            Clang,
            CStandard::C17,
            G::VoidReturnMismatch,
            &[(all, false)],
            None,
        ),
    ] {
        assert_eq!(
            permerror_severity(identity, standard, group, options),
            expected,
            "{identity:?} {standard:?} {group:?} {options:?}"
        );
    }
}

/// An explicit `-fno-permissive` makes GCC 14's C89 permerror warnings
/// errors, leaving what C89 accepts silently, an implicit library
/// function and a conditional's pointer mismatch as they were; a later
/// `-fpermissive` wins.
#[test]
fn explicit_no_permissive_makes_gcc_c89_permerrors_errors() {
    use DiagnosticGroup as G;

    use crate::translation_phases::ErrorSeverity::{
        Error,
        Warning,
    };
    let gnu89 = CompilerConfiguration::new(CStandard::C89, ExtensionPolicy::Allow)
        .with_gnu_extensions(true)
        .with_compiler_identity(CompilerIdentity::Gcc);
    for (group, expected) in [
        (G::VoidReturnMismatch, Some(Error)),
        (G::IntConversion, Some(Error)),
        (G::IncompatiblePointerTypes, Some(Error)),
        (G::IncompatibleFunctionPointerTypes, Some(Error)),
        (G::DeclarationMissingParameterType, Some(Error)),
        (G::ImplicitFunctionDeclaration, None),
        (G::ImplicitInt, None),
        (G::ReturnMismatch, None),
        (G::ImplicitLibraryFunction, Some(Warning)),
        (G::GccConditionalPointerMismatch, Some(Warning)),
    ] {
        let strict = gnu89.with_permissive(false);
        assert_eq!(strict.diagnostic_severity(group), expected, "{group:?}");
        assert_eq!(
            strict.with_permissive(true).diagnostic_severity(group),
            gnu89.diagnostic_severity(group),
            "{group:?}"
        );
    }
    let gnu17 = CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Allow)
        .with_gnu_extensions(true)
        .with_compiler_identity(CompilerIdentity::Gcc);
    assert_eq!(
        gnu17
            .with_permissive(true)
            .diagnostic_severity(G::GccConditionalPointerMismatch),
        Some(Warning)
    );
    assert_eq!(
        gnu17.diagnostic_severity(G::GccConditionalPointerMismatch),
        Some(Error)
    );
}

/// Clang's options for `-Wconversion`, `-Wimplicit-int-float-conversion`
/// and `-Wimplicit-const-int-float-conversion` apply in order, a parent's to
/// each group it holds, so the later of a parent's and a child's option
/// wins (probed with Clang 23).
#[test]
fn clang_conversion_options_apply_in_order_to_their_children() {
    use DiagnosticGroup::{
        ConstantConversion,
        Conversion,
        ImplicitConstIntFloatConversion as Child,
        ImplicitIntFloatConversion as Parent,
    };
    use WarningOption::{
        Enable,
        Error as Werror,
    };

    use crate::translation_phases::ErrorSeverity::{
        Error,
        Warning,
    };
    let severity = |group, options: &[(DiagnosticGroup, WarningOption)]| {
        options
            .iter()
            .fold(
                CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Allow)
                    .with_compiler_identity(CompilerIdentity::Clang),
                |configuration, &(group, option)| configuration.with_warning_option(group, option),
            )
            .diagnostic_severity(group)
    };
    for (case, (options, expected)) in [
        (&[][..], Some(Warning)),
        (
            &[(Child, Enable(false)), (Parent, Werror(true))][..],
            Some(Error),
        ),
        (&[(Parent, Werror(true)), (Child, Enable(false))][..], None),
        (
            &[(Child, Enable(false)), (Parent, Enable(true))][..],
            Some(Warning),
        ),
        (
            &[(Child, Enable(false)), (Conversion, Werror(true))][..],
            Some(Error),
        ),
        (
            &[(Conversion, Enable(true)), (Parent, Enable(false))][..],
            None,
        ),
        (
            &[(Conversion, Enable(false)), (Child, Enable(true))][..],
            Some(Warning),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(severity(Child, options), expected, "case {case}");
    }
    assert_eq!(
        severity(
            ConstantConversion,
            &[
                (ConstantConversion, Enable(false)),
                (Conversion, Enable(true))
            ]
        ),
        Some(Warning)
    );
}

/// Clang's `-Wtautological-compare` holds its constant and pointer groups,
/// so `-Wno-tautological-compare` silences both and a later child's option
/// turns that one on again (probed with Clang 23.1.5). GCC's group of that
/// name holds neither.
#[test]
fn clang_tautological_compare_holds_its_groups() {
    use DiagnosticGroup::{
        TautologicalCompare as Parent,
        TautologicalConstantOutOfRangeCompare as Constant,
        TautologicalPointerCompare as Pointer,
    };
    use WarningOption::Enable;

    use crate::translation_phases::ErrorSeverity::Warning;
    let severity = |identity, group, options: &[(DiagnosticGroup, WarningOption)]| {
        options
            .iter()
            .fold(
                CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Allow)
                    .with_compiler_identity(identity),
                |configuration, &(group, option)| configuration.with_warning_option(group, option),
            )
            .diagnostic_severity(group)
    };
    let clang = CompilerIdentity::Clang;
    assert_eq!(severity(clang, Pointer, &[]), Some(Warning));
    assert_eq!(severity(clang, Pointer, &[(Parent, Enable(false))]), None);
    assert_eq!(severity(clang, Constant, &[(Parent, Enable(false))]), None);
    assert_eq!(
        severity(
            clang,
            Pointer,
            &[(Parent, Enable(false)), (Pointer, Enable(true))]
        ),
        Some(Warning)
    );
    assert_eq!(
        severity(CompilerIdentity::Gcc, Pointer, &[(Parent, Enable(true))]),
        None
    );
}

/// Clang's mapping of an extension diagnostic, set by its options in argv
/// order: a command-line `-WX` keeps an earlier `-Werror=X`, `-Wno-error=X`
/// caps `-pedantic-errors` at a warning and keeps `-Werror` from it for good,
/// `-Wno-X` outranks `-pedantic-errors` wherever it appears, an option for
/// `-Wpedantic` leaves what Clang warns of by default alone, and `-w`
/// silences every one (Clang 23.1.5, measured with `-std=c99`).
#[test]
fn clang_extension_mapping_follows_the_options_in_order() {
    use ExtensionClass::{
        C11,
        C23ByDefault,
        Pedantic,
    };
    use ExtensionOptionName as Name;
    use WarningOption::{
        Enable,
        Error,
    };

    use crate::translation_phases::ErrorSeverity::{
        Error as E,
        Warning as W,
    };
    let base = CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Allow)
        .with_compiler_identity(CompilerIdentity::Clang);
    let errors = base.with_pedantic_errors();
    for (configuration, class, expected) in [
        (base, Pedantic, None),
        (base, C23ByDefault, Some(W)),
        (base.with_pedantic_flag(), Pedantic, Some(W)),
        (errors, C23ByDefault, Some(E)),
        (errors.with_suppress_warnings(true), Pedantic, None),
        (
            base.with_extension_option(Name::Pedantic, Error(true))
                .with_extension_option(Name::Pedantic, Enable(true)),
            Pedantic,
            Some(E),
        ),
        (
            errors.with_extension_option(Name::Pedantic, Error(false)),
            Pedantic,
            Some(W),
        ),
        (
            errors.with_extension_option(Name::Pedantic, Error(false)),
            C23ByDefault,
            Some(E),
        ),
        (
            errors.with_extension_option(Name::Pedantic, Enable(false)),
            C23ByDefault,
            Some(E),
        ),
        (
            errors.with_extension_option(Name::C23Extensions, Enable(false)),
            C23ByDefault,
            None,
        ),
        (
            base.with_extension_option(Name::C11Extensions, Enable(false))
                .with_extension_option(Name::Pedantic, Enable(true)),
            C11,
            Some(W),
        ),
        (
            base.with_extension_option(Name::Pedantic, Enable(true))
                .with_extension_option(Name::C11Extensions, Enable(false)),
            C11,
            None,
        ),
        (
            base.with_warnings_as_errors(true)
                .with_extension_option(Name::C23Extensions, Error(false))
                .with_extension_option(Name::C23Extensions, Error(true))
                .with_extension_option(Name::C23Extensions, Enable(false))
                .with_extension_option(Name::C23Extensions, Enable(true)),
            C23ByDefault,
            Some(W),
        ),
    ] {
        assert_eq!(
            configuration.clang_extension_severity(class),
            expected,
            "{class:?}"
        );
    }
}

/// The extensions each compiler files under a group of its own, which
/// decides the options that reach them and the group their tags name. Clang
/// warns of a typedef redefinition by default in `-Wtypedef-redefinition`
/// alone, which neither `-Werror=pedantic` nor `-Wno-error=pedantic` reaches;
/// it holds an omitted variadic argument in `-Wc23-extensions` too, and
/// `#warning` in `-Wpedantic` alone (Clang 23.1.5, `-std=c99`). GCC 14 files
/// `long long` and mixed declarations before C99 under options `-Wpedantic`
/// does not classify, and variadic macros under one it does.
#[test]
fn extensions_take_the_groups_their_compilers_file_them_under() {
    use ExtensionClass as C;
    use ExtensionOptionName as Name;
    use WarningOption::{
        Enable,
        Error,
    };

    use crate::translation_phases::ErrorSeverity::{
        Error as E,
        Warning as W,
    };
    let class = |feature: Feature, by_default| {
        ExtensionClass::of(feature.origin(), Some(feature), by_default)
    };
    assert_eq!(
        class(Feature::TypedefRedefinition, true),
        C::TypedefRedefinition
    );
    assert_eq!(
        class(Feature::OmittedVariadicArguments, false),
        C::OmittedVariadicArguments
    );
    assert_eq!(class(Feature::WarningDirective, false), C::Pedantic);
    assert_eq!(class(Feature::StaticAssert, false), C::C11);
    for (class, group) in [
        (C::Pedantic, "pedantic"),
        (C::C11, "c11-extensions"),
        (C::C23, "c23-extensions"),
        (C::C23ByDefault, "c23-extensions"),
        (
            C::OmittedVariadicArguments,
            "variadic-macro-arguments-omitted",
        ),
        (C::TypedefRedefinition, "typedef-redefinition"),
        (C::ByDefault, "pedantic"),
    ] {
        assert_eq!(class.clang_group().name(), group, "{class:?}");
    }
    let base = CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Allow)
        .with_compiler_identity(CompilerIdentity::Clang);
    for (case, (configuration, expected)) in [
        (base, Some(W)),
        (
            base.with_extension_option(Name::Pedantic, Error(true)),
            Some(W),
        ),
        (
            base.with_pedantic(true)
                .with_pedantic_flag()
                .with_warnings_as_errors(true)
                .with_extension_option(Name::Pedantic, Error(false)),
            Some(E),
        ),
        (
            base.with_extension_option(Name::TypedefRedefinition, Enable(false))
                .with_pedantic(true)
                .with_pedantic_flag(),
            None,
        ),
        (
            base.with_extension_option(Name::TypedefRedefinition, Error(true)),
            Some(E),
        ),
        (
            base.with_warnings_as_errors(true)
                .with_extension_option(Name::TypedefRedefinition, Error(false)),
            Some(W),
        ),
        (
            base.with_extension_option(Name::Everything, Enable(false)),
            None,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            configuration.clang_extension_severity(C::TypedefRedefinition),
            expected,
            "case {case}"
        );
    }
    let omitted = C::OmittedVariadicArguments;
    assert_eq!(
        base.with_extension_option(Name::C23Extensions, Error(true))
            .clang_extension_severity(omitted),
        Some(E)
    );
    assert_eq!(
        base.with_extension_option(Name::Pedantic, Error(true))
            .with_extension_option(Name::C23Extensions, Enable(false))
            .clang_extension_severity(omitted),
        None
    );
    let gnu = ExtensionOptionName::parse("gnu").unwrap();
    assert_eq!(
        base.with_extension_option(Name::C23Extensions, Error(true))
            .with_extension_option(gnu, Enable(false))
            .clang_extension_severity(omitted),
        None
    );
    assert_eq!(
        base.with_extension_option(gnu, Enable(false))
            .with_extension_option(Name::C23Extensions, Error(true))
            .clang_extension_severity(omitted),
        Some(E)
    );
    assert_eq!(
        base.with_pedantic(true)
            .with_pedantic_flag()
            .with_extension_option(gnu, Enable(false))
            .clang_extension_severity(C::C23),
        Some(W)
    );
    for (feature, filing, group) in [
        (
            Feature::VoidPointerArithmetic,
            Some(GccFiling::Enabled),
            Some(ExtensionGroup::PointerArith),
        ),
        (
            Feature::LongLong,
            Some(GccFiling::Unclassified),
            Some(ExtensionGroup::LongLong),
        ),
        (
            Feature::MixedDeclarations,
            Some(GccFiling::Unclassified),
            Some(ExtensionGroup::DeclarationAfterStatement),
        ),
        (
            Feature::VariadicMacros,
            Some(GccFiling::Enabled),
            Some(ExtensionGroup::VariadicMacros),
        ),
        (
            Feature::NamedVariadicMacros,
            Some(GccFiling::Enabled),
            Some(ExtensionGroup::VariadicMacros),
        ),
        (
            Feature::OmittedVariadicArguments,
            Some(GccFiling::Unclassified),
            None,
        ),
        (Feature::TypedefRedefinition, None, None),
    ] {
        assert_eq!(feature.gcc_filing(), filing, "{feature:?}");
        assert_eq!(feature.gcc_group(), group, "{feature:?}");
    }
}
