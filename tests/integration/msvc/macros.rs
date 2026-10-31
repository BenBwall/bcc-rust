//! The macros `cl.exe` predefines against those `bcc --msvc-mode`
//! predefines, under the options that change them. Each name is
//! stringized before and after expansion, so an undefined macro reads as
//! its own name and an empty one as `""`.

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs,
};

use super::toolchain::{
    Differences,
    Msvc,
    msvc,
    scratch,
};
use crate::native::run_native;

/// Macros compared. `cl` defines the `_M_`, `_MSC_` and `_WIN` families and
/// `_MT`, `_DLL`, `_DEBUG` and `_CHAR_UNSIGNED` from its options; the
/// GNU and Clang names must stay undefined.
const NAMES: [&str; 33] = [
    "_MSC_VER",
    "_MSC_FULL_VER",
    "_MSC_BUILD",
    "_MSC_EXTENSIONS",
    "_MSVC_TRADITIONAL",
    "_WIN32",
    "_WIN64",
    "_M_X64",
    "_M_AMD64",
    "_M_IX86",
    "_M_ARM64",
    "_M_FP_PRECISE",
    "_M_FP_FAST",
    "_M_FP_STRICT",
    "_MT",
    "_DLL",
    "_DEBUG",
    "_CHAR_UNSIGNED",
    "_INTEGRAL_MAX_BITS",
    "_ISO_VOLATILE",
    "__STDC__",
    "__STDC_VERSION__",
    "__STDC_HOSTED__",
    "__STDC_NO_ATOMICS__",
    "__STDC_NO_COMPLEX__",
    "__STDC_NO_THREADS__",
    "__STDC_NO_VLA__",
    "__AVX__",
    "__SSE2__",
    "__GNUC__",
    "__clang__",
    "__x86_64__",
    "__bcc__",
];

/// The option sets compared.
const OPTIONS: &[&[&str]] = &[
    &[],
    &["/MD"],
    &["/MDd"],
    &["/MTd"],
    &["/LD"],
    &["/std:c11"],
    &["/std:c17"],
    &["/std:clatest"],
    &["/Za"],
    &["/J"],
    &["/O2"],
    &["/fp:fast"],
    &["/Zc:preprocessor"],
];

/// Names whose difference driver-modes.md records ("Predefined macros"
/// under MSVC).
const DOCUMENTED: [&str; 13] = [
    // bcc claims MSVC 19.33.
    "_MSC_VER",
    "_MSC_FULL_VER",
    // bcc's preprocessor is conforming, as `cl /Zc:preprocessor`'s is.
    "_MSVC_TRADITIONAL",
    // ISO C requires `__STDC__`, and bcc compiles C17 by default; `cl
    // /std:clatest` claims `202312L`.
    "__STDC__",
    "__STDC_VERSION__",
    // bcc has atomics, complex types and VLAs, and no `<threads.h>`.
    "__STDC_NO_ATOMICS__",
    "__STDC_NO_COMPLEX__",
    "__STDC_NO_THREADS__",
    "__STDC_NO_VLA__",
    // Clang's target macros stay.
    "__SSE2__",
    "__x86_64__",
    // bcc keeps IEC 60559 semantics under `/fp:fast`.
    "_M_FP_FAST",
    "__bcc__",
];

#[test]
fn predefined_macros_match_cl() {
    let Some(msvc) = msvc() else {
        eprintln!("skipping: no Visual Studio C compiler on this host");
        return;
    };
    let root = scratch("macros");
    let mut source = String::from("#define NAME(x) #x\n#define VALUE(x) NAME(x)\n");
    for name in NAMES {
        writeln!(source, "NAME({name})=VALUE({name});").unwrap();
    }
    fs::write(root.join("macros.c"), source).unwrap();
    let mut differences = Differences::default();
    for options in OPTIONS {
        let cl = macros(msvc, &root, true, options);
        let bcc = macros(msvc, &root, false, options);
        println!("\n==== {}", options.join(" "));
        for name in NAMES {
            let (cl, bcc) = (cl.get(name), bcc.get(name));
            let mark = if cl == bcc { " " } else { "*" };
            println!("{mark} {name:22} cl {cl:?} bcc {bcc:?}");
            if !DOCUMENTED.contains(&name) {
                differences.compare(&format!("{} {name}", options.join(" ")), &cl, &bcc);
            }
        }
    }
    drop(fs::remove_dir_all(root));
    differences.assert_none("predefined macro");
}

/// The value of each name under `options`, `None` where it is undefined,
/// or every name `None` when the compiler refuses the options.
fn macros(
    msvc: &Msvc,
    root: &std::path::Path,
    cl: bool,
    options: &[&str],
) -> BTreeMap<String, Option<String>> {
    let mut command = if cl { msvc.cl() } else { msvc.bcc() };
    let output = run_native(
        command
            .current_dir(root)
            .arg("/nologo")
            .args(options)
            .args(["/EP", "macros.c"]),
    )
    .unwrap();
    if !output.status.success() {
        println!(
            "{} refuses {options:?}: {}",
            if cl { "cl" } else { "bcc" },
            String::from_utf8_lossy(&output.stderr)
        );
    }
    // Whitespace is dropped, so each pair ends with `;` however `/EP` lays
    // the text out.
    let text: String = String::from_utf8_lossy(&output.stdout)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let mut values = BTreeMap::new();
    for pair in text.split(';') {
        let Some((name, value)) = pair.split_once('=') else {
            continue;
        };
        let name = name.trim_matches('"').to_owned();
        let value = value.trim_matches('"').to_owned();
        let value = (value != name).then_some(value);
        drop(values.insert(name, value));
    }
    values
}
