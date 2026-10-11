use std::io::{
    self,
    Write,
};

use super::{
    output::{
        TokenOutput,
        print_preprocessor_output_in,
    },
    *,
};
use crate::{
    configuration::{
        ExtensionPolicy,
        MsvcFeature,
    },
    diagnostics::ColorChoice as RenderColor,
    util::bump::ArenaString,
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
        let cli = Cli::try_parse_from(normalize_language_arguments(
            arguments.map(std::ffi::OsString::from),
        ))
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
    print_preprocessor_output_in(
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
    reporter_arena.high_water()
}

#[test]
fn token_reporter_arena_stays_bounded_across_diagnostic_batches() {
    let once = token_reporter_high_water(1_000);
    let twice = token_reporter_high_water(2_000);
    assert_eq!(once, twice);
}
