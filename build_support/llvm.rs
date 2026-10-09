use std::{
    env,
    fs,
    path::PathBuf,
};

/// Builds and installs the pinned LLVM tools unless an installation for
/// `version` already exists, returning the installation prefix.
pub(super) fn build(version: &str) -> PathBuf {
    println!("cargo:rerun-if-changed=vendor/rust");
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let prefix = root.join("target/llvm");
    // A completed installation records its version last, so a restored cache
    // or an unchanged checkout skips reconfiguring the LLVM build tree. This
    // first check takes no lock and writes nothing, so a read-only cache works:
    // a shared installation reached through a link, or a sandbox that may not
    // write outside the checkout.
    let stamp = prefix.join(".installed-version");
    let installed = || fs::read_to_string(&stamp).is_ok_and(|installed| installed == version);
    if installed() {
        return prefix;
    }
    fs::create_dir_all(&prefix).unwrap();
    // Cargo profiles share this cache; hold the lock until installation
    // finishes, and check again under it in case another build just did.
    let lock = fs::File::options()
        .write(true)
        .create(true)
        .truncate(false)
        .open(prefix.join(".build-lock"))
        .unwrap();
    lock.lock().unwrap();
    if installed() {
        return prefix;
    }

    let host = env::var("HOST").unwrap();
    let msvc_host = host.replace("windows-gnu", "windows-msvc");
    let bootstrap_host = if host.ends_with("windows-gnu")
        && cc::windows_registry::find_tool(&msvc_host, "cl.exe").is_some()
    {
        &msvc_host
    } else {
        &host
    };
    let jobs = env::var("NUM_JOBS")
        .unwrap()
        .parse::<usize>()
        .unwrap()
        .min(8);
    let mut compiler = cc::Build::new();
    _ = compiler.inherit_rustflags(false);
    let mut config = cmake::Config::new(root.join("vendor/rust/src/llvm-project/llvm"));
    _ = config
        .host(bootstrap_host)
        .target(bootstrap_host)
        .init_c_cfg(compiler.clone())
        .init_cxx_cfg(compiler)
        .generator("Ninja")
        .profile("Release")
        .out_dir(prefix)
        .define("LLVM_ENABLE_PROJECTS", "clang;lld")
        .define("LLVM_TARGETS_TO_BUILD", "Native")
        .define("LLVM_DEFAULT_TARGET_TRIPLE", &host)
        .define("LLVM_VERSION_SUFFIX", "")
        .define(
            "LLVM_DISTRIBUTION_COMPONENTS",
            "clang;libclang;llvm-ar;lld;clang-resource-headers",
        )
        .define("LLVM_UNREACHABLE_OPTIMIZE", "OFF")
        .define("LLVM_USE_SYMLINKS", "OFF")
        .define("LLVM_PARALLEL_LINK_JOBS", "1")
        .define("CLANG_PLUGIN_SUPPORT", "OFF")
        .define("CLANG_ENABLE_OBJC_REWRITER", "OFF")
        .define("CLANG_ENABLE_STATIC_ANALYZER", "OFF")
        .build_target("install-distribution")
        .build_arg(format!("-j{jobs}"));
    for option in ["TESTS", "EXAMPLES", "DOCS", "BENCHMARKS"] {
        _ = config.define(format!("LLVM_INCLUDE_{option}"), "OFF");
    }
    for option in [
        "BINDINGS",
        "LIBEDIT",
        "LIBXML2",
        "ZLIB",
        "ZSTD",
        "Z3_SOLVER",
    ] {
        _ = config.define(format!("LLVM_ENABLE_{option}"), "OFF");
    }
    let prefix = config.build();
    fs::write(stamp, version).unwrap();
    prefix
}
