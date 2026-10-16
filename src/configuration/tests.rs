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
        Some(Feature::UnnamedDefinitionParameters as usize)
    );
    for (index, feature) in MsvcFeature::ALL.iter().enumerate() {
        assert_eq!(*feature as usize, index, "{feature:?}");
    }
    assert_eq!(MsvcFeature::ALL.last(), Some(&MsvcFeature::VaArgs));
}
