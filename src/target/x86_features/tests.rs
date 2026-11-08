//! The feature options and `target` attribute texts as GCC 14.2 and Clang 23
//! read them, measured with the probes of
//! `scripts/measure_x86_target_attribute.py`.
#![expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "Tests compare collected lists and rendered texts."
)]

use super::{
    CodeFeatures,
    TargetFault as F,
    X86Features,
    target_attribute_faults,
};

fn command_line(options: &[&str], clang: bool) -> X86Features {
    X86Features::from_options(&options.iter().copied(), clang)
}

fn faults(text: &str, clang: bool) -> Vec<F<'_>> {
    target_attribute_faults(text, clang).collect()
}

fn llvm(code: &CodeFeatures) -> String {
    code.llvm_features()
        .map(|(on, name)| format!("{}{name}", if on { '+' } else { '-' }))
        .collect::<Vec<_>>()
        .join(",")
}

#[test]
fn command_line_features_keep_their_macros_and_llvm_string() {
    let mut buffer = [0; 256];
    let features = command_line(&["-mavx2", "-mno-popcnt"], true);
    assert_eq!(
        features.llvm_features(&mut buffer).to_str().unwrap(),
        "+sse3,+ssse3,+sse4.1,+sse4.2,-popcnt,+crc32,+avx,+avx2,+xsave"
    );
    assert_eq!(
        features.macros().collect::<Vec<_>>(),
        [
            "__SSE3__",
            "__SSSE3__",
            "__SSE4_1__",
            "__SSE4_2__",
            "__CRC32__",
            "__AVX__",
            "__AVX2__",
            "__XSAVE__"
        ]
    );
    // GCC's AVX needs XSAVE; Clang's only brings it by default.
    assert!(!command_line(&["-mavx", "-mno-xsave"], false).avx());
    assert!(command_line(&["-mavx", "-mno-xsave"], true).avx());
    // An option outside the 18 the command line implements stays unknown.
    assert!(!super::is_feature_option("-mavx512f"));
    assert_eq!(
        command_line(&["-mavx512f"], true).unknown_vector_abi(),
        Some(32)
    );
}

#[test]
fn clang_ignores_the_attribute_for_its_first_fault() {
    let none: [F<'_>; 0] = [];
    assert_eq!(faults("avx2,no-bmi,arch=haswell,tune=znver3", true), none);
    // Clang trims each part and the processor's name.
    assert_eq!(faults("  avx2 , bmi2,arch= haswell", true), none);
    assert_eq!(
        faults("sse4,no-sse4,x87,general-regs-only,prefer-256-bit", true),
        none
    );
    assert_eq!(faults("arch=,tune=", true), none);
    assert_eq!(faults("foo", true), [F::Unsupported("foo")]);
    assert_eq!(faults("no-foo", true), [F::Unsupported("foo")]);
    assert_eq!(faults("", true), [F::Unsupported("")]);
    assert_eq!(faults("avx2,,bmi", true), [F::Unsupported("")]);
    assert_eq!(faults("AVX2", true), [F::Unsupported("AVX2")]);
    assert_eq!(faults("3dnow", true), [F::Unsupported("3dnow")]);
    assert_eq!(
        faults("no-arch=haswell", true),
        [F::Unsupported("arch=haswell")]
    );
    assert_eq!(faults("arch=foo", true), [F::UnknownCpu("foo")]);
    assert_eq!(faults("arch=native", true), [F::UnknownCpu("native")]);
    assert_eq!(faults("tune=foo", true), [F::UnknownTune("foo")]);
    assert_eq!(
        faults("arch=haswell,arch=skylake", true),
        [F::Duplicate("arch=")]
    );
    // `fpmath=` comes first, then the processors, then a repeat, then the
    // features in order.
    assert_eq!(
        faults("foo,arch=bar,fpmath=sse", true),
        [F::Unsupported("fpmath=")]
    );
    assert_eq!(
        faults("foo,tune=bar,arch=baz", true),
        [F::UnknownCpu("baz")]
    );
    assert_eq!(
        faults("foo,arch=x86-64,arch=haswell", true),
        [F::Duplicate("arch=")]
    );
    assert_eq!(
        faults("foo", true)[0].to_string(),
        "unsupported 'foo' in the 'target' attribute string; 'target' attribute ignored"
    );
    assert!(!F::UnknownCpu("foo").is_error());
}

#[test]
fn gcc_refuses_every_fault_as_written() {
    let none: [F<'_>; 0] = [];
    assert_eq!(faults("avx2,no-bmi,arch=haswell,tune=znver3", false), none);
    assert_eq!(
        faults(
            "fpmath=sse+387,prefer-vector-width=256,cld,no-recip,3dnow,avx2,",
            false
        ),
        none
    );
    assert_eq!(
        faults("tune=generic,tune=", false),
        [F::UnknownArgument("tune=")]
    );
    assert_eq!(faults(" avx2", false), [F::UnknownArgument(" avx2")]);
    assert_eq!(faults("avx2,,bmi", false), [F::UnknownArgument("")]);
    assert_eq!(
        faults("x87,no-foo", false),
        [F::UnknownArgument("x87"), F::UnknownArgument("no-foo")]
    );
    assert_eq!(faults("arch=foo", false), [F::BadValue("arch", "foo")]);
    assert_eq!(
        faults("arch=native", false),
        [F::BadValue("arch", "native")]
    );
    assert_eq!(
        faults("arch=foo,arch=haswell", false),
        [
            F::AlreadySpecified("arch=haswell"),
            F::BadValue("arch", "foo")
        ]
    );
    assert_eq!(
        faults("no-general-regs-only", false),
        [F::NegatedForm("general-regs-only")]
    );
    assert_eq!(faults("fpmath=foo", false), [F::UnknownValue("fpmath=foo")]);
    assert_eq!(
        faults("arch=foo", false)[0].to_string(),
        "bad value 'foo' for 'target(\"arch=\")' attribute"
    );
    assert!(F::UnknownArgument("foo").is_error());
}

#[test]
fn an_attribute_reads_over_the_command_line() {
    // Clang: AVX2 brings what it needs and, by default, POPCNT, CRC32 and
    // XSAVE; BMI2 needs nothing.
    let code = command_line(&[], true).target_attribute("avx2,bmi2").code();
    assert_eq!(
        llvm(&code),
        "+avx,+avx2,+bmi2,+crc32,+popcnt,+sse3,+sse4.1,+sse4.2,+ssse3,+xsave"
    );
    assert_eq!(code.llvm_cpu(), None);
    // Turning a feature off takes what needs it along and says so to LLVM.
    // Without AVX, Clang's XSAVE no longer comes by default.
    let text = llvm(
        &command_line(&["-mavx2"], true)
            .target_attribute("no-avx")
            .code(),
    );
    assert!(
        text.contains("-avx,") && text.contains("-avx2,") && text.contains("-avx512f,"),
        "{text}"
    );
    assert!(
        text.contains("+sse4.2") && !text.contains("xsave"),
        "{text}"
    );
    // `arch=` sets the processor's features as defaults. Clang keeps what
    // came before; GCC forgets it.
    let clang = command_line(&[], true)
        .target_attribute("avx2,arch=core2")
        .code();
    let gcc = command_line(&[], false)
        .target_attribute("avx2,arch=core2")
        .code();
    assert!(llvm(&clang).contains("+avx2"));
    assert!(!llvm(&gcc).contains("avx2"));
    assert_eq!(gcc.llvm_cpu(), Some("core2"));
    let gcc = command_line(&[], false)
        .target_attribute("arch=core2,avx2")
        .code();
    assert!(llvm(&gcc).contains("+avx2"));
    // A processor LLVM does not know stands as x86-64 with its features.
    let nano = command_line(&[], false)
        .target_attribute("arch=nano")
        .code();
    assert_eq!(
        (nano.llvm_cpu(), llvm(&nano).as_str()),
        (Some("x86-64"), "+sahf,+sse3,+ssse3")
    );
    let tuned = command_line(&[], true)
        .target_attribute(" tune=znver3")
        .code();
    assert_eq!(
        (tuned.llvm_tune(), llvm(&tuned).as_str()),
        (Some("znver3"), "")
    );
    // GCC's `abm` has no LLVM name but brings LZCNT and POPCNT.
    let abm = command_line(&[], false).target_attribute("abm").code();
    assert_eq!(llvm(&abm), "+lzcnt,+popcnt");
}

#[test]
fn baseline_features_cannot_be_turned_off() {
    let features = command_line(&[], true);
    assert!(features.target_attribute("no-sse2").refused());
    assert!(features.target_attribute("general-regs-only").refused());
    assert!(
        !features
            .target_attribute("no-general-regs-only,sse2,x87")
            .refused()
    );
}

#[test]
fn inlining_needs_every_feature_of_the_callee() {
    let plain = command_line(&[], true).target_attribute("");
    let avx2 = command_line(&[], true).target_attribute("avx2");
    let both = command_line(&[], true).target_attribute("avx2,bmi2");
    let haswell = command_line(&[], true).target_attribute("arch=haswell");
    assert_eq!(plain.missing(&avx2), Some("avx2"));
    assert_eq!(avx2.missing(&both), Some("bmi2"));
    assert_eq!(both.missing(&avx2), None);
    assert!(plain.missing(&haswell).is_some());
    assert!(haswell.missing(&both).is_none());
    assert!(both.code().includes(&avx2.code()));
    assert!(!avx2.code().includes(&both.code()));
}

#[test]
fn vector_conventions_change_with_avx_and_avx512() {
    let none = command_line(&[], true);
    let avx = command_line(&["-mavx"], true);
    assert!(none.target_changes_vector_abi(&none.target_attribute("avx")));
    assert!(!none.target_changes_vector_abi(&none.target_attribute("bmi2,arch=core2")));
    assert!(avx.target_changes_vector_abi(&avx.target_attribute("avx512f")));
    assert!(avx.target_changes_vector_abi(&avx.target_attribute("no-avx")));
    assert!(!avx.target_changes_vector_abi(&avx.target_attribute("avx2")));
}

#[test]
fn ir_text_names_round_trip() {
    let code = command_line(&["-mavx2"], false)
        .target_attribute("no-avx2,bmi2,arch=haswell,tune=generic")
        .code();
    let mut read = CodeFeatures::default();
    for (sign, name) in code.parts() {
        assert!(read.add_part(sign, name), "{sign}{name}");
    }
    assert_eq!(read, code);
    assert!(!read.add_part("+", "foo"));
    assert!(!read.add_part("arch=", "foo"));
}
