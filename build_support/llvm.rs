use std::{
    env,
    fs,
    path::PathBuf,
};

#[path = "llvm/cache.rs"]
mod cache;
#[path = "llvm/tools.rs"]
mod tools;

pub(super) use tools::build;

const LINK_COMPONENTS: [&str; 4] = ["core", "passes", "x86codegen", "x86asmparser"];

/// Installs ABI-compatible static LLVM libraries in a separate cache and links
/// their transitive closure. llvm-sys supplies declarations only: dependency
/// build scripts run before this one, so it cannot bootstrap our installation.
/// Returns the prefix of the installation whose libraries are linked, whose
/// headers C++ code calling into them must be compiled against.
pub(super) fn link_libraries(version: &str, tools: &std::path::Path) -> PathBuf {
    use std::process::Command;
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let host = env::var("HOST").unwrap();
    let windows_gnu = host.ends_with("windows-gnu");
    let prefix = root.join("target/llvm-libs");
    let stamp = prefix.join(".installed-version");
    let identity = format!("{version}-{host}-static-x86-v1");
    let installed = || fs::read_to_string(&stamp).is_ok_and(|value| value == identity);
    let config_name = format!("llvm-config{}", env::consts::EXE_SUFFIX);
    // Linux distributions may already contain ABI-compatible static libraries.
    let prebuilt = !windows_gnu
        && tools.join("bin").join(&config_name).is_file()
        && tools.join("lib/libLLVMCore.a").is_file();
    let install = if prebuilt { tools } else { &prefix };
    let _lock = if prebuilt {
        None
    } else {
        cache::lock_for_install(&prefix, &stamp, &identity).unwrap()
    };
    if !prebuilt && !installed() {
        let jobs = env::var("NUM_JOBS")
            .unwrap()
            .parse::<usize>()
            .unwrap()
            .min(8);
        let mut config = cmake::Config::new(root.join("vendor/rust/src/llvm-project/llvm"));
        _ = config
            .host(&host)
            .target(&host)
            .generator("Ninja")
            .profile("Release")
            .out_dir(&prefix)
            .define("LLVM_ENABLE_PROJECTS", "")
            .define("LLVM_TARGETS_TO_BUILD", "X86")
            .define("LLVM_DEFAULT_TARGET_TRIPLE", &host)
            .define("LLVM_VERSION_SUFFIX", "")
            .define("LLVM_ENABLE_ASSERTIONS", "OFF")
            .define("LLVM_BUILD_LLVM_DYLIB", "OFF")
            .define("LLVM_BUILD_LLVM_C_DYLIB", "OFF")
            .define("LLVM_LINK_LLVM_DYLIB", "OFF")
            .define(
                "LLVM_DISTRIBUTION_COMPONENTS",
                "llvm-libraries;llvm-config;llvm-headers",
            )
            .define("LLVM_PARALLEL_LINK_JOBS", "1")
            .define("LLVM_USE_SYMLINKS", "OFF")
            .build_target("LLVMPasses")
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
        let mut compiler = cc::Build::new();
        _ = compiler.inherit_rustflags(false);
        _ = config.init_c_cfg(compiler.clone()).init_cxx_cfg(compiler);
        if windows_gnu {
            let sysroot = super::native::mingw_root();
            let mut sysroot_flag = sysroot.display().to_string();
            if sysroot_flag.contains(' ') {
                sysroot_flag = format!("\"{sysroot_flag}\"");
            }
            let flags = format!(
                "--target=x86_64-w64-windows-gnu --sysroot={sysroot_flag} -pthread -femulated-tls",
            );
            _ = config
                .define("CMAKE_C_COMPILER", tools.join("bin/clang.exe"))
                .define("CMAKE_CXX_COMPILER", tools.join("bin/clang++.exe"))
                .define("CMAKE_C_FLAGS", &flags)
                .define("CMAKE_CXX_FLAGS", &flags)
                .define("CMAKE_EXE_LINKER_FLAGS", "-static -pthread -fuse-ld=lld")
                .define("CMAKE_AR", tools.join("bin/llvm-ar.exe"))
                .define("CMAKE_RANLIB", sysroot.join("bin/ranlib.exe"));
        }
        drop(config.build());
        let tree = prefix.join("build");
        let run = |command: &mut Command| {
            let status = command
                .status()
                .expect("Unable to run LLVM build/install tool");
            assert!(status.success(), "LLVM build/install tool failed");
        };
        run(Command::new("cmake")
            .arg("--build")
            .arg(&tree)
            .args([
                "--target",
                "LLVMX86CodeGen",
                "LLVMX86AsmParser",
                "llvm-config",
                "--parallel",
            ])
            .arg(jobs.to_string()));
        let result = Command::new(tree.join("bin").join(&config_name))
            .args(["--link-static", "--libnames"])
            .args(LINK_COMPONENTS)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "LLVM link closure lookup failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let names = String::from_utf8(result.stdout).unwrap();
        // Install only the queried closure, configuration executable and
        // headers.
        for name in names
            .split_whitespace()
            .map(|name| {
                name.strip_prefix("lib")
                    .unwrap_or(name)
                    .strip_suffix(".a")
                    .or_else(|| name.strip_suffix(".lib"))
                    .unwrap()
            })
            .chain(["llvm-config", "llvm-headers"])
        {
            run(Command::new("cmake")
                .arg("--install")
                .arg(&tree)
                .arg("--component")
                .arg(name));
        }
        fs::write(&stamp, &identity).unwrap();
    }
    let llvm_config = install.join("bin").join(config_name);
    let query = |args: &[&str], components: &[&str]| {
        let result = Command::new(&llvm_config)
            .args(args)
            .args(components)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "llvm-config failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        String::from_utf8(result.stdout).unwrap()
    };
    assert_eq!(
        query(&["--version"], &[]).trim(),
        version,
        "LLVM libraries must match the pinned toolchain"
    );
    println!(
        "cargo:rustc-link-search=native={}",
        query(&["--libdir"], &[]).trim()
    );
    for name in query(&["--link-static", "--libnames"], &LINK_COMPONENTS).split_whitespace() {
        let name = name.strip_prefix("lib").unwrap_or(name);
        let name = name
            .strip_suffix(".a")
            .or_else(|| name.strip_suffix(".lib"))
            .unwrap();
        println!("cargo:rustc-link-lib=static={name}");
    }
    // An empty component list makes llvm-config validate every LLVM library,
    // including libraries outside the closure installed above.
    for name in query(&["--link-static", "--system-libs"], &LINK_COMPONENTS).split_whitespace() {
        if let Some(name) = name.strip_prefix("-l") {
            println!("cargo:rustc-link-lib={name}");
        }
    }
    if windows_gnu {
        let sysroot = super::native::mingw_root();
        let gcc = sysroot.join("bin/gcc.exe");
        for library in ["libstdc++.a", "libgcc_eh.a", "libwinpthread.a"] {
            let result = Command::new(&gcc)
                .arg(format!("-print-file-name={library}"))
                .output()
                .unwrap();
            assert!(result.status.success(), "MinGW library lookup failed");
            let path = PathBuf::from(String::from_utf8(result.stdout).unwrap().trim());
            assert!(
                path.is_file(),
                "MinGW static runtime missing: {}",
                path.display()
            );
            println!(
                "cargo:rustc-link-search=native={}",
                path.parent().unwrap().display()
            );
        }
        for library in ["stdc++", "gcc_eh", "winpthread"] {
            println!("cargo:rustc-link-lib=static={library}");
        }
    } else if !host.ends_with("windows-msvc") {
        // The static C++ runtime sits in the GCC installation's private
        // library directory, which the linker does not search by default.
        let path = ["g++", "gcc"]
            .into_iter()
            .find_map(|compiler| {
                let result = Command::new(compiler)
                    .arg("-print-file-name=libstdc++.a")
                    .output()
                    .ok()?;
                let path = PathBuf::from(String::from_utf8(result.stdout).ok()?.trim());
                path.is_file().then_some(path)
            })
            .expect("the static C++ runtime libstdc++.a is needed to link LLVM; install g++");
        println!(
            "cargo:rustc-link-search=native={}",
            path.parent().unwrap().display()
        );
        println!("cargo:rustc-link-lib=static=stdc++");
    }
    install.to_path_buf()
}
