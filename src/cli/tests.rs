use std::{
    fmt::Write as _,
    io::{
        self,
        Write,
    },
    path::Path,
};

use super::{
    output::{
        TokenOutput,
        print_preprocessor_output_in,
    },
    preprocessed::print_preprocessed,
    *,
};
use crate::{
    configuration::{
        ExtensionPolicy,
        MsvcFeature,
    },
    diagnostics::ColorChoice as RenderColor,
    headers::HeaderSearch,
    pipeline::TextOptions,
    test_support::{
        committed_peak,
        macro_heavy_source,
    },
    translation_phases::Context,
    util::bump::{
        ArenaString,
        Bump,
    },
};

#[test]
#[expect(
    clippy::disallowed_types,
    reason = "Startup argv tests use owned OS strings as clap does."
)]
fn language_flags_apply_in_order_and_input_values_stay_opaque() {
    for (flags, seh) in [
        (["-fms-extensions", "-fno-ms-seh", "-fms-seh"], true),
        (["-fms-seh", "-fms-extensions", "-fno-ms-seh"], false),
    ] {
        let arguments = [
            "bcc-rust",
            "-std=c89",
            flags[0],
            flags[1],
            flags[2],
            "-pedantic-errors",
            "--input",
            "-std=c98",
        ];
        let cli = Cli::try_parse_from(
            normalize_language_arguments(arguments.map(std::ffi::OsString::from)).unwrap(),
        )
        .unwrap();
        let configuration = cli.configuration();
        assert_eq!(
            configuration.standard(),
            crate::configuration::CStandard::C89
        );
        assert_eq!(configuration.extension_policy(), ExtensionPolicy::Deny);
        assert_eq!(configuration.msvc_feature(MsvcFeature::Seh), seh);
        assert!(configuration.msvc_feature(MsvcFeature::Declspec));
        assert_eq!(cli.input.input.as_deref(), Some("-std=c98"));
    }
    let repeated =
        Cli::try_parse_from(["bcc-rust", "--std=c89", "--std=gnu23", "--input", "int x;"]).unwrap();
    assert_eq!(
        repeated.configuration().standard(),
        crate::configuration::CStandard::C23
    );
    assert!(repeated.configuration().gnu_extensions());
    let cli = Cli::try_parse_from(["bcc-rust", "--input", "int x;"]).unwrap();
    assert_eq!(
        cli.configuration().standard(),
        crate::configuration::CStandard::C17
    );
    assert!(cli.configuration().gnu_extensions());
}

struct CountDiagnostics(usize);

#[test]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    reason = "Startup argv tests own clap argument strings."
)]
fn char_signedness_flags_override_the_target_and_each_other() {
    for (flags, signed) in [
        (vec![], true),
        (vec!["-funsigned-char"], false),
        (vec!["-fno-signed-char"], false),
        (vec!["-funsigned-char", "-fsigned-char"], true),
        (
            vec!["-fsigned-char", "-fno-unsigned-char", "-funsigned-char"],
            false,
        ),
    ] {
        let args = [vec!["bcc-rust", "--input", "int x;"], flags].concat();
        let cli = Cli::try_parse_from(
            normalize_language_arguments(args.iter().map(std::ffi::OsString::from)).unwrap(),
        )
        .unwrap();
        assert_eq!(cli.configuration().layout().char_signed, signed, "{args:?}");
    }
}

#[test]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    reason = "Startup argv tests own clap argument strings."
)]
fn inline_flags_override_the_final_standard_and_each_other() {
    for (flags, expected) in [
        (vec!["-std=gnu89"], true),
        (vec!["-std=c89"], true),
        (vec!["-std=c90"], true),
        (vec!["-std=gnu17"], false),
        (vec!["-fgnu89-inline", "-std=c99"], true),
        (
            vec!["-std=gnu17", "-fgnu89-inline", "-fno-gnu89-inline"],
            false,
        ),
        (
            vec!["-std=c99", "-fno-gnu89-inline", "-fgnu89-inline"],
            true,
        ),
        (vec!["-std=gnu89", "-fno-gnu89-inline"], false),
    ] {
        let args = [vec!["bcc-rust", "--input", "int x;"], flags].concat();
        let cli = Cli::try_parse_from(
            normalize_language_arguments(args.iter().map(std::ffi::OsString::from)).unwrap(),
        )
        .unwrap();
        assert_eq!(cli.configuration().gnu89_inline(), expected, "{args:?}");
    }
}

impl Write for CountDiagnostics {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.starts_with(b"error: #error bad") {
            self.0 += 1;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn token_reporter_high_water(repetitions: usize) -> usize {
    let tu = Bump::new();
    let mut source = ArenaString::new_in(&tu);
    for number in 0..repetitions {
        _ = write!(source, "#error bad\nint x{number};\n");
    }
    let source = source.into_str();
    let mut context = Context::new(&tu);
    let mut reporter_arena = Bump::new();
    let mut out = CountDiagnostics(0);
    let errors = print_preprocessor_output_in(
        &mut context,
        Path::new("<input>"),
        source,
        HeaderSearch::default(),
        TokenOutput {
            out:            &mut out,
            reporter_arena: &mut reporter_arena,
            color:          RenderColor::Plain,
        },
    );
    assert_eq!(out.0, repetitions);
    assert_eq!(errors, repetitions);
    reporter_arena.high_water()
}

#[test]
fn token_reporter_arena_stays_bounded_across_diagnostic_batches() {
    let once = token_reporter_high_water(1_000);
    let twice = token_reporter_high_water(2_000);
    assert_eq!(once, twice);
}

/// The bytes `-E` commits at its peak for `lines` macro-heavy declarations,
/// and its output tokens.
fn preprocessed_text_peak(lines: usize) -> (usize, usize) {
    let source = macro_heavy_source(lines);
    committed_peak(|| {
        let tu = Bump::new();
        let mut context = Context::new(&tu);
        let source = tu.alloc_str(&source);
        let failed = print_preprocessed(
            &mut context,
            Path::new("<input>"),
            source,
            HeaderSearch::default(),
            TextOptions::default(),
            &mut io::sink(),
        )
        .expect("the text is written");
        assert!(!failed);
        context.preprocessed_tokens
    })
}

/// `-E` formats each token's text as the token is produced, so what it
/// keeps per token is about the text alone, between 400 and 800 lines.
#[test]
fn preprocessed_text_commits_little_more_than_its_text_per_token() {
    let (small, small_tokens) = preprocessed_text_peak(400);
    let (large, large_tokens) = preprocessed_text_peak(800);
    let per_token = (large - small) / (large_tokens - small_tokens);
    assert!(
        per_token <= MAX_TEXT_BYTES_PER_TOKEN,
        "{per_token} bytes per token ({small} for {small_tokens} tokens, {large} for \
         {large_tokens})"
    );
}

/// Bytes per token `-E` may commit: the text takes under two for this input.
/// Keeping every token, its position and its provenance until the end took
/// 141.
const MAX_TEXT_BYTES_PER_TOKEN: usize = 16;

/// Clang's front end words what it says about each warning option as
/// `ProcessWarningOptions` does, with `getNearestOption`'s suggestion
/// (checked against the pinned Clang 23.1.5 on 2026-11-19).
#[test]
fn clang_complains_about_warning_options_in_its_own_words() {
    let arena = Bump::new();
    for (option, complaint) in [
        ("-Wshadow", None),
        ("-Wno-shadow", None),
        ("-W", None),
        ("-Wno-", None),
        ("-Werror", None),
        ("-Werror=", None),
        ("-Werror=shadow", None),
        ("-Werror-implicit-function-declaration", None),
        ("-Wformat=0", None),
        ("-Wformat=2", None),
        ("-Wno-format=2", None),
        ("-Wsystem-headers", None),
        ("-Weverything", None),
        ("-Wno-everything", None),
        ("-Wfatal-errors", None),
        ("-Wfatal-errors=shadow", None),
        ("-Wfatal-errors-shadow", None),
        ("-W#warnings", None),
        ("-Wfoo", Some("unknown warning option '-Wfoo'")),
        (
            "-Wshadwo",
            Some("unknown warning option '-Wshadwo'; did you mean '-Wshadow'?"),
        ),
        (
            "-Wno-no-shadow",
            Some("unknown warning option '-Wno-no-shadow'; did you mean '-Wno-shadow'?"),
        ),
        (
            "-Wno-error=shadwo",
            Some("unknown warning option '-Werror=shadwo'; did you mean '-Werror=shadow'?"),
        ),
        (
            "-Wfatal-errors=pass",
            Some(
                "unknown warning option '-Wfatal-errors=pass'; did you mean '-Wfatal-errors=asm'?",
            ),
        ),
        (
            "-Wfatal-errors-=x",
            Some("unknown warning option '-Wfatal-errors==x'"),
        ),
        (
            "-Wformat=1",
            Some("unknown warning option '-Wformat=1'; did you mean '-Wformat=2'?"),
        ),
        ("-Wformat=", Some("unknown warning option '-Wformat='")),
        ("-Weverythin", Some("unknown warning option '-Weverythin'")),
        (
            "-Wno-everything=",
            Some("unknown warning option '-Wno-everything='"),
        ),
        (
            "-Werror-",
            Some("unknown -Werror warning specifier: '-Werror-'"),
        ),
        (
            "-Wno-error-x",
            Some("unknown -Werror warning specifier: '-Wno-error-x'"),
        ),
        (
            "-Werror-implicit-function-declarationx",
            Some("unknown -Werror warning specifier: '-Werror-implicit-function-declarationx'"),
        ),
        (
            "-Wfatal-errors=",
            Some("unknown -Wfatal-errors warning specifier: '-Wfatal-errors='"),
        ),
        // A group of remarks alone is no warning option.
        (
            "-Wpass",
            Some("unknown warning option '-Wpass'; did you mean '-Wasm'?"),
        ),
        ("-Wl,", Some("unknown warning option '-Wl,'")),
    ] {
        assert_eq!(
            warning_options::clang_complaint(option, &arena),
            complaint,
            "{option}"
        );
        assert_eq!(
            warning_options::clang_takes(option),
            complaint.is_none(),
            "{option}"
        );
    }
}

#[test]
fn personalities_learn_how_bccs_command_line_reads_an_option() {
    for (options, stage, reading) in [
        (&["-O2"][..], "-c", OptionReading::Read),
        (&["-o", "x.obj"], "-c", OptionReading::Read),
        (&["-D", "X"], "-c", OptionReading::Read),
        (&["-funsigned-char"], "-c", OptionReading::Read),
        // bcc's own option, which writes what `-c` would.
        (&["--emit=ir"], "-c", OptionReading::Conflicts),
        (&["-dM"], "-E", OptionReading::Read),
        (&["-dM"], "-c", OptionReading::Inert),
        (&["-shared"], "-c", OptionReading::Conflicts),
        (&["-ffoo"], "-c", OptionReading::Refused),
        (&["--bogus"], "-c", OptionReading::Refused),
        (&["-Xclang", "-O2"], "-c", OptionReading::Refused),
    ] {
        assert_eq!(read_options(options, stage), reading, "{options:?} {stage}");
    }
}

#[test]
#[expect(
    clippy::disallowed_types,
    reason = "Startup argv tests use owned OS strings as clap does."
)]
fn a_value_that_starts_with_a_dash_stays_a_value() {
    let normalized = |arguments: &[&str]| {
        normalize_language_arguments(arguments.iter().map(std::ffi::OsString::from))
            .unwrap()
            .into_iter()
            .map(|argument| argument.into_string().unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        normalized(&[
            "bcc-rust", "-I", "-mfoo", "-x", "-c", "-B", "-T", "--", "-a.c"
        ]),
        ["bcc-rust", "-I-mfoo", "--language=-c", "-B-T", "--", "-a.c"]
    );
    assert_eq!(
        normalized(&["bcc-rust", "-iquote", "-W", "-o", "out", "-I", "dir"]),
        ["bcc-rust", "--iquote=-W", "-o", "out", "-I", "dir"]
    );
    for arguments in [
        &["bcc-rust", "-I", "-mfoo", "--input", "int x;"][..],
        &["bcc-rust", "-isystem", "-x", "--input", "int x;"],
        &["bcc-rust", "-B", "-ffoo", "--input", "int x;"],
    ] {
        assert!(
            Cli::try_parse_from(normalized(arguments)).is_ok(),
            "{arguments:?}"
        );
    }
}
