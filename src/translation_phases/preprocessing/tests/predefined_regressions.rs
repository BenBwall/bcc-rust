//! Required C99 predefined macros.

use std::path::PathBuf;

use crate::{
    configuration::CompilerConfiguration,
    translation_phases::{
        Context,
        SourceVector,
        preprocessing::{
            IntegerTokenType,
            Preprocessor,
            Token,
            TokenType,
        },
    },
    util::{
        packed::Packed,
        shared::SharedVec,
    },
};

#[derive(Debug, Default)]
struct Observation {
    kinds:     Vec<TokenType>,
    spellings: Vec<String>,
    sources:   Vec<Vec<SourceVector>>,
    errors:    Vec<String>,
}

fn record_token(token: Token, context: &Context<'_>, observation: &mut Observation) {
    observation.kinds.push(token.kind);
    observation.spellings.push(
        context
            .string_cache
            .at(token.contents)
            .trim_end_matches('\0')
            .to_owned(),
    );
    observation
        .sources
        .push(context.get_source_vectors(token.source_vectors).to_vec());
}

fn record_errors(context: &mut Context<'_>, observation: &mut Observation) {
    observation.errors.extend(
        context
            .take_pending_errors()
            .into_iter()
            .map(|error| error.to_string()),
    );
}

fn observe(source: &str) -> Observation {
    observe_with(source, CompilerConfiguration::default())
}

#[test]
fn freestanding_resources_and_suffix_helpers_expand() {
    let observation = observe(
        "#include <iso646.h>\n#include <stdbool.h>\n#include <limits.h>\n#include \
         <stdint.h>\n#include <float.h>\nINT8_C(1) UINT32_C(2) INT64_C(3) UINT64_C(4) INTMAX_C(5) \
         UINTMAX_C(6)\nCHAR_BIT MB_LEN_MAX FLT_MANT_DIG DBL_MANT_DIG LDBL_MANT_DIG\ntrue false \
         not and or xor\n",
    );
    assert!(observation.errors.is_empty(), "{:?}", observation.errors);
    assert!(observation.spellings.iter().any(|s| s == "3L"));
    assert!(observation.spellings.iter().any(|s| s == "4UL"));
    assert!(observation.spellings.iter().any(|s| s == "!"));
    assert!(observation.spellings.iter().any(|s| s == "&&"));
    assert!(
        observation
            .sources
            .iter()
            .flatten()
            .any(|s| s.source_file_index != 0)
    );
}

#[test]
fn resource_queries_and_target_macro_override_follow_normal_lookup() {
    let observation = observe_with(
        "#if !__has_include(<stddef.h>) || !__has_include(<stdarg.h>)\n#error missing \
         resource\n#endif\n#if __has_include(<absent-resource.h>)\n#error unexpected \
         resource\n#endif\n#undef __CHAR_BIT__\n#define __CHAR_BIT__ 16\n__CHAR_BIT__\n",
        CompilerConfiguration::default().with_gnu_extensions(true),
    );
    assert!(observation.errors.is_empty(), "{:?}", observation.errors);
    assert_eq!(observation.spellings, ["16"]);
}

fn observe_with(source: &str, configuration: CompilerConfiguration) -> Observation {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::with_configuration(&tu, configuration);
    let preprocess_arena = crate::util::bump::Bump::new();
    let mut preprocessor = Preprocessor::new(
        &preprocess_arena,
        &mut context,
        PathBuf::from("<predefined regressions>").into_boxed_path(),
        source,
        SharedVec::default(),
        SharedVec::default(),
    );
    let mut observation = Observation::default();
    for token in preprocessor.preprocess_all(&mut context) {
        record_token(token, &context, &mut observation);
    }
    record_errors(&mut context, &mut observation);
    observation
}

fn identifier_spellings(observation: &Observation) -> Vec<&str> {
    observation
        .kinds
        .iter()
        .zip(&observation.spellings)
        .filter_map(|(kind, spelling)| {
            matches!(kind, TokenType::Identifier).then_some(spelling.as_str())
        })
        .collect()
}

fn integer_kinds(observation: &Observation) -> Vec<IntegerTokenType> {
    observation
        .kinds
        .iter()
        .filter_map(|kind| match kind {
            | TokenType::Integer(value) => Some(*value),
            | _ => None,
        })
        .collect()
}

#[test]
fn standard_macros_expand_to_required_numeric_types_and_values() {
    let source = "__STDC__; __STDC_VERSION__; __STDC_HOSTED__; __STDC_MB_MIGHT_NEQ_WC__; after\n";
    let observation = observe(source);
    assert!(observation.errors.is_empty(), "{observation:#?}");
    assert_eq!(
        integer_kinds(&observation),
        [
            IntegerTokenType::Int(1),
            IntegerTokenType::Long(Packed::new(199_901)),
            IntegerTokenType::Int(1),
            IntegerTokenType::Int(1),
        ]
    );
    assert_eq!(
        observation.spellings,
        ["1", ";", "199901L", ";", "1", ";", "1", ";", "after"]
    );
    for (index, column, length) in [(0, 1, 8), (2, 11, 16), (4, 29, 15), (6, 46, 24)] {
        let source = &observation.sources[index][0];
        assert_eq!(
            (source.line, source.column, source.length),
            (1, column, length)
        );
    }
}

#[test]
fn required_standard_macros_are_available_to_both_defined_forms() {
    let source = "#if defined __STDC__ && defined(__STDC_VERSION__) && defined(__STDC_HOSTED__) \
                  && defined(__STDC_MB_MIGHT_NEQ_WC__)\nall_defined\n#endif\nafter\n";
    let observation = observe(source);
    assert_eq!(identifier_spellings(&observation), ["all_defined", "after"]);
    assert!(observation.errors.is_empty(), "{observation:#?}");
}

#[test]
fn required_standard_macros_are_available_to_ifdef() {
    let source = "#ifdef __STDC__\nstdc_defined\n#endif\n#ifdef \
                  __STDC_VERSION__\nversion_defined\n#endif\n#ifdef \
                  __STDC_HOSTED__\nhosted_defined\n#endif\n#ifdef \
                  __STDC_MB_MIGHT_NEQ_WC__\nencoding_defined\n#endif\nafter\n";
    let observation = observe(source);
    assert_eq!(
        identifier_spellings(&observation),
        [
            "stdc_defined",
            "version_defined",
            "hosted_defined",
            "encoding_defined",
            "after"
        ]
    );
    assert!(observation.errors.is_empty(), "{observation:#?}");
}

#[test]
fn standard_macro_values_select_the_configured_execution_environment() {
    for (hosted, expected) in [(true, 1), (false, 0)] {
        let source = format!(
            "#if __STDC__ == 1 && __STDC_VERSION__ == 199901L && __STDC_HOSTED__ == {expected} && \
             __STDC_MB_MIGHT_NEQ_WC__ == 1\nright_values\n#else\nwrong_values\n#endif\nafter\n"
        );
        let observation = observe_with(
            &source,
            CompilerConfiguration::default().with_hosted(hosted),
        );
        assert_eq!(
            identifier_spellings(&observation),
            ["right_values", "after"]
        );
        assert!(observation.errors.is_empty(), "{observation:#?}");
    }
}

#[test]
fn standard_version_expands_inside_an_ordinary_macro_alias() {
    let source = "#define VERSION_ALIAS __STDC_VERSION__\n#define ENCODING_ALIAS \
                  __STDC_MB_MIGHT_NEQ_WC__\nVERSION_ALIAS; ENCODING_ALIAS; after\n";
    let observation = observe(source);
    assert_eq!(
        integer_kinds(&observation),
        [
            IntegerTokenType::Long(Packed::new(199_901)),
            IntegerTokenType::Int(1)
        ]
    );
    assert_eq!(identifier_spellings(&observation), ["after"]);
    assert!(observation.errors.is_empty(), "{observation:#?}");
}

#[test]
fn ordinary_macro_definitions_and_undef_remain_usable() {
    let source = "#define STDC 7\n#define STDC_VERSION 8\n#define STDC_HOSTED 9\n#define \
                  STDC_MB_MIGHT_NEQ_WC 10\n#if defined(STDC) && STDC == 7 && STDC_VERSION == 8 && \
                  STDC_HOSTED == 9 && STDC_MB_MIGHT_NEQ_WC == 10\nnormal_defined\n#endif\nSTDC; \
                  STDC_VERSION; STDC_HOSTED; STDC_MB_MIGHT_NEQ_WC;\n#undef STDC\n#ifdef \
                  STDC\nunreachable\n#else\nnormal_undef\n#endif\nSTDC after\n";
    let observation = observe(source);
    assert_eq!(
        integer_kinds(&observation),
        [
            IntegerTokenType::Int(7),
            IntegerTokenType::Int(8),
            IntegerTokenType::Int(9),
            IntegerTokenType::Int(10)
        ]
    );
    assert_eq!(
        identifier_spellings(&observation),
        ["normal_defined", "normal_undef", "STDC", "after"]
    );
    assert!(observation.errors.is_empty(), "{observation:#?}");
}

/// The spellings of `__DATE__` and `__TIME__` under `source_date_epoch`.
fn translation_timestamp(source_date_epoch: Option<i64>) -> (String, String) {
    let configuration = CompilerConfiguration::default().with_source_date_epoch(source_date_epoch);
    let observation = observe_with("__DATE__, __TIME__; __DATE__\n", configuration);
    assert!(observation.errors.is_empty(), "{observation:#?}");
    assert!(
        matches!(
            observation.kinds[..],
            [
                TokenType::String(_),
                TokenType::Operator(_),
                TokenType::String(_),
                TokenType::Operator(_),
                TokenType::String(_),
            ]
        ),
        "{observation:#?}"
    );
    let [date, _, time, _, again] =
        <[String; 5]>::try_from(observation.spellings).expect("five tokens were matched above");
    assert_eq!(date, again, "every __DATE__ in a unit agrees");
    (date, time)
}

#[test]
fn configured_source_date_epoch_spells_utc_date_and_time() {
    for (seconds, date, time) in [
        (0, "\"Jan  1 1970\"", "\"00:00:00\""),
        (1_700_000_000, "\"Nov 14 2023\"", "\"22:13:20\""),
        (-1, "\"Dec 31 1969\"", "\"23:59:59\""),
    ] {
        assert_eq!(
            translation_timestamp(Some(seconds)),
            (date.to_owned(), time.to_owned()),
            "{seconds}"
        );
    }
}

/// Whether `date` spells a `__DATE__` (`"Mmm dd yyyy"`, the day space-padded)
/// and `time` a `__TIME__` (`"hh:mm:ss"`).
fn is_timestamp_spelling(date: &str, time: &str) -> bool {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let digits = |text: &str| text.bytes().all(|byte| byte.is_ascii_digit());
    let date_parts = date
        .strip_prefix('"')
        .and_then(|date| date.strip_suffix('"'))
        .filter(|date| date.len() == 11)
        .is_some_and(|date| {
            MONTHS.contains(&&date[..3])
                && &date[3..4] == " "
                && (&date[4..5] == " " || digits(&date[4..5]))
                && digits(&date[5..6])
                && &date[6..7] == " "
                && digits(&date[7..])
        });
    let time_parts = time
        .strip_prefix('"')
        .and_then(|time| time.strip_suffix('"'))
        .filter(|time| time.len() == 8)
        .is_some_and(|time| {
            time.split(':').count() == 3
                && time.split(':').all(|part| part.len() == 2 && digits(part))
        });
    date_parts && time_parts
}

#[test]
fn translation_timestamp_without_a_representable_epoch_spells_the_local_time() {
    for source_date_epoch in [None, Some(i64::MAX), Some(i64::MIN)] {
        let (date, time) = translation_timestamp(source_date_epoch);
        assert!(
            is_timestamp_spelling(&date, &time),
            "{source_date_epoch:?}: {date} {time}"
        );
    }
}

#[test]
fn version_strict_ansi_and_identity_macros_follow_every_mode() {
    use crate::configuration::{
        CStandard,
        ExtensionPolicy,
    };
    for (standard, version) in [
        (CStandard::C89, None),
        (CStandard::C95, Some(199_409)),
        (CStandard::C99, Some(199_901)),
        (CStandard::C11, Some(201_112)),
        (CStandard::C17, Some(201_710)),
        (CStandard::C23, Some(202_311)),
        (CStandard::C2y, Some(202_400)),
    ] {
        for gnu in [false, true] {
            let configuration = CompilerConfiguration::new(standard, ExtensionPolicy::Allow)
                .with_gnu_extensions(gnu);
            let result = observe_with(
                "#ifdef __STDC_VERSION__\nversion __STDC_VERSION__\n#endif\n#ifdef \
                 __STRICT_ANSI__\nstrict __STRICT_ANSI__\n#endif\n#ifdef __GNUC__\ngnu __GNUC__ \
                 __GNUC_MINOR__ __GNUC_PATCHLEVEL__\n#endif\n#ifdef \
                 __GNUC_GNU_INLINE__\ngnu_inline\n#endif\n#ifdef \
                 __GNUC_STDC_INLINE__\nstdc_inline\n#endif\n#ifdef _MSC_VER\nbad_ms\n#endif\n#if \
                 defined(__clang__) || !defined(__bcc__)\nbad_identity\n#endif\n",
                configuration,
            );
            let mut expected = Vec::new();
            if let Some(version) = version {
                expected.extend(["version".to_owned(), format!("{version}L")]);
            }
            if !gnu {
                expected.extend(["strict".to_owned(), "1".to_owned()]);
            }
            expected.extend(["gnu", "4", "2", "1"].map(str::to_owned));
            expected.push(
                if standard < CStandard::C99 {
                    "gnu_inline"
                } else {
                    "stdc_inline"
                }
                .to_owned(),
            );
            assert_eq!(result.spellings, expected, "{standard:?}, gnu={gnu}");
            assert!(result.errors.is_empty(), "{:?}", result.errors);
        }
    }
}

#[test]
fn msvc_identity_follows_the_umbrella_flag_and_identity_macros_can_be_undefined() {
    let source = "_MSC_VER _MSC_FULL_VER _MSC_BUILD _MSC_EXTENSIONS\n#undef __GNUC__\n#undef \
                  __bcc__\n#if defined __GNUC__ || defined __bcc__\nbad\n#endif\n";
    let result = observe_with(
        source,
        CompilerConfiguration::default()
            .with_gnu_extensions(true)
            .with_msvc_extensions(true),
    );
    assert_eq!(result.spellings, ["1933", "193300000", "1", "1"]);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    // A single group is not the umbrella, and the umbrella's opposite
    // withdraws it.
    for configuration in [
        CompilerConfiguration::default()
            .with_msvc_feature(crate::configuration::MsvcFeature::Declspec, true),
        CompilerConfiguration::default()
            .with_msvc_extensions(true)
            .with_msvc_extensions(false),
    ] {
        let result = observe_with("#ifdef _MSC_VER\nbad\n#endif\n", configuration);
        assert!(result.spellings.is_empty(), "{:?}", result.spellings);
    }
}

#[test]
fn char8_atomic_macros_follow_c23_keywords_for_every_target_and_dialect() {
    use crate::{
        configuration::{
            CStandard,
            ExtensionPolicy,
        },
        target::Target,
    };
    for target in [
        Target::LinuxGnu,
        Target::LinuxMusl,
        Target::WindowsGnu,
        Target::WindowsMsvc,
    ] {
        for standard in [CStandard::C17, CStandard::C23, CStandard::C2y] {
            for gnu in [false, true] {
                let config = CompilerConfiguration::new(standard, ExtensionPolicy::Deny)
                    .with_target(target)
                    .with_gnu_extensions(gnu);
                let observed = observe_with(
                    "#ifdef __CLANG_ATOMIC_CHAR8_T_LOCK_FREE\nclang_char8 \
                     __CLANG_ATOMIC_CHAR8_T_LOCK_FREE\n#endif\n#ifdef \
                     __GCC_ATOMIC_CHAR8_T_LOCK_FREE\ngcc_char8 \
                     __GCC_ATOMIC_CHAR8_T_LOCK_FREE\n#endif\n",
                    config,
                );
                assert!(observed.errors.is_empty(), "{:?}", observed.errors);
                let expected = if standard < CStandard::C23 {
                    &[][..]
                } else if target == Target::WindowsMsvc {
                    &["clang_char8", "2"][..]
                } else {
                    &["clang_char8", "2", "gcc_char8", "2"][..]
                };
                assert_eq!(
                    observed.spellings, expected,
                    "{target:?} {standard:?} {gnu}"
                );
            }
        }
    }
}
