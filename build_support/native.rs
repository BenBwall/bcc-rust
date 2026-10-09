use std::{
    env::{
        self,
        var,
        var_os,
    },
    path::{
        Path,
        PathBuf,
    },
    process::Command,
};

fn output(command: &mut Command) -> String {
    let result = command.output().expect("Unable to run LLVM tool");
    assert!(
        result.status.success(),
        "LLVM tool failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).expect("LLVM tool output is UTF-8")
}

fn llvm_version(text: &str) -> Option<&str> {
    text.split_whitespace()
        .find(|word| word.as_bytes().first().is_some_and(u8::is_ascii_digit))
}

fn find_tool(directory: &Path, name: &str, version: &str) -> PathBuf {
    let executable = format!("{name}{}", env::consts::EXE_SUFFIX);
    let path = directory.join(executable);
    let tool_version = output(Command::new(&path).arg("--version"));
    assert_eq!(
        llvm_version(&tool_version),
        Some(version),
        "{} must use rustc's LLVM {version}; rebuild the pinned LLVM sources",
        path.display()
    );
    path
}

fn mingw_root() -> PathBuf {
    if let Some(root) = var_os("MINGW_ROOT") {
        return PathBuf::from(root);
    }
    for directory in env::split_paths(&var_os("PATH").unwrap_or_default()) {
        if directory.join("gcc.exe").is_file()
            && let Some(root) = directory.parent()
            && (root.join("include/windows.h").is_file()
                || root.join("x86_64-w64-mingw32/include/windows.h").is_file())
        {
            return root.to_owned();
        }
    }
    panic!("Set MINGW_ROOT to a MinGW installation containing include/windows.h");
}

pub(super) fn compile(out_dir: &str) {
    for name in ["RUSTC", "MINGW_ROOT", "PATH"] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    let target = var("TARGET").unwrap();
    assert_eq!(
        target,
        var("HOST").unwrap(),
        "The pinned LLVM build supports native targets"
    );
    let windows_gnu = target.ends_with("windows-gnu");
    let windows_msvc = target.ends_with("windows-msvc");
    let linux = var("CARGO_CFG_TARGET_OS").unwrap() == "linux";
    assert!(
        windows_gnu || windows_msvc || linux,
        "Configure a static LLVM linker and cross-language LTO for target {target}"
    );
    let flags = var("CARGO_ENCODED_RUSTFLAGS").unwrap_or_default();
    assert!(
        flags.contains("linker-plugin-lto") && flags.contains("lto=fat"),
        "Use the repository Cargo configuration; custom RUSTFLAGS must retain linker-plugin-lto \
         and lto=fat"
    );
    let rust_info = output(Command::new(var_os("RUSTC").unwrap()).arg("-vV"));
    let version = rust_info
        .lines()
        .find_map(|line| line.strip_prefix("LLVM version: "))
        .unwrap();
    let prefix = super::llvm::build(version);
    let directory = prefix.join("bin");
    let clang = find_tool(
        &directory,
        if windows_msvc { "clang-cl" } else { "clang" },
        version,
    );
    let archiver = find_tool(&directory, "llvm-ar", version);
    let resource_dir =
        output(Command::new(find_tool(&directory, "clang", version)).arg("-print-resource-dir"));
    let mut clang_args = vec![
        format!("--target={target}"),
        format!("-resource-dir={}", resource_dir.trim()),
    ];
    if windows_gnu {
        clang_args.push(format!("--sysroot={}", mingw_root().display()));
    }
    let library_dir = if cfg!(windows) {
        directory
    } else {
        prefix.join("lib")
    };
    // SAFETY: No threads or bindgen have started in this build script.
    // Select the pinned libclang before its loader reads the environment.
    unsafe {
        env::set_var("LIBCLANG_PATH", library_dir);
    }
    let bindings_version = bindgen::clang_version();
    assert_eq!(
        llvm_version(&bindings_version.full),
        Some(version),
        "bindgen's libclang must also use LLVM {version}"
    );
    println!(
        "cargo:warning=Native C and bindgen use LLVM {version}; static C archive and Rust/C fat \
         LTO enabled"
    );
    let mut native = cc::Build::new();
    _ = native
        .compiler(&clang)
        .archiver(&archiver)
        .file("build_support/float_parsing.c")
        .opt_level(3)
        .debug(true)
        .inherit_rustflags(false)
        .static_crt(true)
        .out_dir(out_dir);
    if windows_msvc {
        _ = native
            .flags(clang_args.iter().map(|arg| format!("/clang:{arg}")))
            .flag("/clang:-flto=full");
    } else {
        _ = native.flags(&clang_args).flag("-flto=full");
    }
    native.compile("float_parsing");
    bindgen::Builder::default()
        .header("build_support/float_parsing.h")
        .clang_args(&clang_args)
        .allowlist_file("build_support/float_parsing.h")
        .allowlist_item("ERANGE")
        .rust_edition(bindgen::RustEdition::Edition2024)
        .generate()
        .expect("Unable to generate bindings")
        .write_to_file(Path::new(out_dir).join("bindings.rs"))
        .expect("Couldn't write bindings");
}
