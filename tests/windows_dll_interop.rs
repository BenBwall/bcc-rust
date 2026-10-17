//! Real DLL boundaries and repeated Microsoft inline bodies must link with
//! Clang in both directions. Windows also executes every linked client.
#![expect(unused_crate_dependencies, reason = "Tests run compiler processes.")]
#![expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Tests own fixtures and captured process output."
)]

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{
            Path,
            PathBuf,
        },
        process::Command,
    };

    const EXPORTER: &str = r#"
struct Data { int first, values[2]; };
__declspec(dllexport) int exported=7;
__declspec(dllexport) struct Data state={3,{5,11}};
__declspec(dllexport) int answer(int value){return exported+state.values[1]+value;}
__declspec(dllexport) int selected(int value) __attribute__((overloadable)){return value+40;}
__declspec(dllexport) int selected(long long value) __attribute__((overloadable)){return (int)value+50;}
int implementation(int value){return value+9;}
__declspec(dllexport) int exported_alias(int) __attribute__((alias("implementation")));
"#;
    const IMPORTER: &str = r#"
struct Data { int first, values[2]; };
__declspec(dllimport) extern int exported;
__declspec(dllimport) extern struct Data state;
__declspec(dllimport) int answer(int);
__declspec(dllimport) int selected(int) __attribute__((overloadable));
__declspec(dllimport) int selected(long long) __attribute__((overloadable));
__declspec(dllimport) int exported_alias(int);
int entry(void) {
    int (*function)(int)=answer;
    int *object=&exported, *member=&state.values[1];
    if (*object!=7 || *member!=11 || function(2)!=20 || exported_alias(1)!=10) return 1;
    if (selected(2)!=42 || selected(3LL)!=53) return 3;
    *object=9;
    *member=13;
    return answer(1)==23 ? 0 : 2;
}
"#;

    #[test]
    fn dll_functions_data_addresses_and_aliases_interoperate_with_clang() {
        for target in ["x86_64-w64-windows-gnu", "x86_64-pc-windows-msvc"] {
            for backend in ["llvm", "native"] {
                for bcc_exports in [true, false] {
                    let root = fixture_root(&format!("dll-{target}-{backend}-{bcc_exports}"));
                    fs::write(root.join("exporter.c"), EXPORTER).unwrap();
                    fs::write(root.join("importer.c"), IMPORTER).unwrap();
                    compile(&root, "exporter", target, backend, bcc_exports);
                    compile(&root, "importer", target, backend, !bcc_exports);
                    let dll = root.join("boundary.dll");
                    let import_library = root.join("boundary.lib");
                    checked(
                        linker()
                            .args(["/dll", "/noentry", "/nodefaultlib", "/machine:x64"])
                            .arg(format!("/out:{}", dll.display()))
                            .arg(format!("/implib:{}", import_library.display()))
                            .arg(root.join("exporter.obj")),
                    );
                    let executable = root.join("client.exe");
                    link_client(&executable, &[root.join("importer.obj"), import_library]);
                    execute_on_windows(&executable);
                    fs::remove_dir_all(root).unwrap();
                }
            }
        }
    }

    #[test]
    fn microsoft_inline_definitions_coalesce_across_compilers() {
        for (mode, prefix) in [
            ("ordinary", "inline"),
            ("extern", "extern inline"),
            ("exported", "__declspec(dllexport) inline"),
        ] {
            for backend in ["llvm", "native"] {
                for bcc_left in [true, false] {
                    let root = fixture_root(&format!("comdat-{mode}-{backend}-{bcc_left}"));
                    let definition = format!("{prefix} int shared(int value){{return value+4;}}\n");
                    fs::write(
                        root.join("left.c"),
                        format!(
                            "{definition} int left(void){{return shared(1);}} int \
                             (*left_address(void))(int){{return shared;}}"
                        ),
                    )
                    .unwrap();
                    fs::write(
                        root.join("right.c"),
                        format!(
                            "{definition} int left(void); int (*left_address(void))(int); int \
                             entry(void){{return left()==5 && shared(2)==6 && \
                             left_address()==shared ? 0 : 1;}}"
                        ),
                    )
                    .unwrap();
                    compile(&root, "left", "x86_64-pc-windows-msvc", backend, bcc_left);
                    compile(&root, "right", "x86_64-pc-windows-msvc", backend, !bcc_left);
                    let executable = root.join("coalesced.exe");
                    link_client(
                        &executable,
                        &[root.join("left.obj"), root.join("right.obj")],
                    );
                    execute_on_windows(&executable);
                    fs::remove_dir_all(root).unwrap();
                }
            }
        }
    }

    fn fixture_root(label: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("bcc-windows-link-{}-{label}", std::process::id()));
        fs::create_dir_all(root.join("include")).unwrap();
        fs::write(root.join("include/windows.h"), "").unwrap();
        root
    }

    fn compile(root: &Path, unit: &str, target: &str, backend: &str, bcc: bool) {
        let compiler = if bcc {
            PathBuf::from(env!("CARGO_BIN_EXE_bcc-rust"))
        } else {
            tool("clang")
        };
        let mut command = Command::new(compiler);
        _ = command.args([
            &format!("--target={target}"),
            "-std=c17",
            "-fms-extensions",
            "-c",
        ]);
        if bcc {
            _ = command.arg(format!("--backend={backend}"));
            if !cfg!(windows) && target.ends_with("windows-gnu") {
                // These freestanding fixtures use no system headers. Native
                // object assembly still needs a MinGW sysroot selector.
                _ = command.env("MINGW_ROOT", root);
            }
        }
        checked(
            command
                .arg(root.join(format!("{unit}.c")))
                .arg("-o")
                .arg(root.join(format!("{unit}.obj"))),
        );
    }

    fn link_client(executable: &Path, objects: &[PathBuf]) {
        checked(
            linker()
                .args([
                    "/entry:entry",
                    "/subsystem:console",
                    "/nodefaultlib",
                    "/machine:x64",
                ])
                .arg(format!("/out:{}", executable.display()))
                .args(objects),
        );
    }

    fn linker() -> Command {
        let mut command = Command::new(tool(if cfg!(windows) { "lld" } else { "ld.lld" }));
        _ = command.args(["-flavor", "link"]);
        command
    }

    fn tool(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/llvm/bin")
            .join(if cfg!(windows) {
                format!("{name}.exe")
            } else {
                name.to_owned()
            })
    }

    fn execute_on_windows(executable: &Path) {
        if cfg!(windows) {
            checked(
                Command::new("python")
                    .arg("scripts/cargo_runner.py")
                    .arg(executable),
            );
        }
    }

    fn checked(command: &mut Command) {
        let output = command
            .env("NO_COLOR", "1")
            .env_remove("CPATH")
            .env_remove("C_INCLUDE_PATH")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{command:?}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
