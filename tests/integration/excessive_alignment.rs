//! Alignment requests above 2^28 bytes, which GCC rejects and Clang ignores
//! (gcc.dg/pr39323-1.c, gcc.dg/attr-aligned.c). Honouring them made each
//! object file pad its sections to the requested alignment: 512 MiB for
//! one such object, over 1 GiB for attr-aligned.c, and twice that in
//! memory while the LLVM back end built the object.

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "Tests own process output and temporary files."
)]
mod tests {
    use std::{
        path::PathBuf,
        process::Command,
    };

    use crate::native::run_native;

    const SOURCE: &str =
        "int foo __attribute__((aligned(1 << 29))) = 20;\ntypedef int __attribute__((aligned(1 << \
         29))) int29;\nint29 bar;\n_Alignas(1 << 30) static char baz;\n__attribute__((aligned(1 \
         << 29))) int f(void) {\nstatic int s __attribute__((aligned(1 << 29)));\nint l \
         __attribute__((aligned(1 << 29))) = 1;\nreturn s + l + baz + *(volatile int *)&l;\n}\n";

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            for name in ["input.c", "output.o"] {
                drop(std::fs::remove_file(self.0.join(name)));
            }
            drop(std::fs::remove_dir(&self.0));
        }
    }

    fn compile(identity: &str, backend: &str, index: usize) -> (std::process::Output, u64) {
        let scratch = Scratch(std::env::temp_dir().join(format!(
            "bcc-excessive-alignment-{}-{index}",
            std::process::id()
        )));
        std::fs::create_dir_all(&scratch.0).unwrap();
        let input = scratch.0.join("input.c");
        let object = scratch.0.join("output.o");
        std::fs::write(&input, SOURCE).unwrap();
        let output = run_native(
            Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args([identity, backend, "-c"])
                .arg(&input)
                .arg("-o")
                .arg(&object)
                .env_remove("CPATH")
                .env_remove("C_INCLUDE_PATH"),
        )
        .unwrap();
        let size = std::fs::metadata(&object).map_or(0, |metadata| metadata.len());
        (output, size)
    }

    #[test]
    fn alignments_above_two_to_the_28_are_ignored_or_rejected_as_clang_and_gcc_do() {
        for (index, backend) in ["--backend=llvm", "--backend=native"]
            .into_iter()
            .enumerate()
        {
            let (output, size) = compile("--compiler-identity=clang", backend, 2 * index);
            assert!(
                output.status.success(),
                "{backend}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(size < 1 << 20, "{backend}: a {size}-byte object");
            let (output, size) = compile("--compiler-identity=gcc", backend, 2 * index + 1);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert_eq!(output.status.code(), Some(1), "{backend}: {stderr}");
            assert_eq!(
                stderr
                    .matches("error: requested alignment exceeds maximum 268435456")
                    .count(),
                6,
                "{backend}: {stderr}"
            );
            assert_eq!(size, 0, "{backend}");
        }
    }
}
