//! `-g`: the DWARF `readelf`, `addr2line` and `objdump` read back, which the
//! tests skip when a tool is missing or cannot read the object, and the code
//! it must leave alone. What needs no tool, such as which sections exist, also
//! runs where the host's binutils do not read ELF.

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "Tests own temporary paths, process output and expected strings."
)]
mod tests {
    use std::{
        path::{
            Path,
            PathBuf,
        },
        process::{
            Command,
            Output,
        },
        sync::atomic::{
            AtomicU32,
            Ordering,
        },
    };

    const LINUX: &str = "--target=x86_64-unknown-linux-gnu";

    /// A header's function, a macro of two lines used on one, `#line` moving
    /// code to another file, and a loop calling all of them.
    const PROGRAM: &str = "\
#include \"inc.h\"
#define BUMP(x) ((x) + \\
   1)
int g;
int add(int a, int b)
{
  int r = a + b;
  r = BUMP(r);
  return r;
}
#line 100 \"other.c\"
int moved(int a)
{
  g = a;
  return helper(a);
}
#line 19 \"a.c\"
int main(void)
{
  int s = 0;
  for (int i = 0; i < 3; i++)
    s += add(i, moved(i));
  return s != 12;
}
";

    const HEADER: &str = "\
static int helper(int x)
{
  return x * 2;
}
";

    struct Directory(PathBuf);

    impl Drop for Directory {
        fn drop(&mut self) {
            drop(std::fs::remove_dir_all(&self.0));
        }
    }

    impl Directory {
        /// A fresh directory holding the program as `a.c` and `inc.h`.
        fn new() -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let directory = std::env::temp_dir().join(format!(
                "bcc-debug-info-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&directory).unwrap();
            std::fs::write(directory.join("a.c"), PROGRAM).unwrap();
            std::fs::write(directory.join("inc.h"), HEADER).unwrap();
            Self(directory)
        }

        fn bcc(&self, args: &[&str]) -> Output {
            Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
                .args(args)
                .current_dir(&self.0)
                .env("NO_COLOR", "1")
                .output()
                .unwrap()
        }

        /// Runs bcc, which must succeed, and returns its stderr.
        fn built(&self, args: &[&str]) -> String {
            let output = self.bcc(args);
            assert!(
                output.status.success(),
                "{args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8_lossy(&output.stderr).into_owned()
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }

        /// The stdout of `tool` on the file `name` here, or `None` if the tool
        /// is missing or cannot read the file.
        fn tool(&self, tool: &str, args: &[&str], name: &str) -> Option<String> {
            let output = Command::new(tool)
                .args(args)
                .arg(self.path(name))
                .current_dir(&self.0)
                .output()
                .ok()?;
            if !output.status.success() {
                return None;
            }
            Some(String::from_utf8_lossy(&output.stdout).into_owned())
        }
    }

    fn holds(file: &Path, name: &[u8]) -> bool {
        std::fs::read(file)
            .unwrap()
            .windows(name.len())
            .any(|window| window == name)
    }

    /// The `(file, line)` rows of `readelf`'s decoded line table.
    fn rows(table: &str) -> Vec<(String, u32)> {
        table
            .lines()
            .filter_map(|row| {
                let mut words = row.split_whitespace();
                let file = words.next()?;
                let line = words.next()?.parse().ok()?;
                let address = words.next()?;
                (address == "0" || address.starts_with("0x")).then(|| (file.to_owned(), line))
            })
            .collect()
    }

    #[test]
    fn without_g_or_with_g0_the_object_has_no_debug_section() {
        let directory = Directory::new();
        for flags in [&[][..], &["-g0"], &["-g", "-g0"], &["-gdwarf-4", "-g0"]] {
            let mut args = vec!["--gcc-mode", LINUX, "-c", "a.c", "-o", "a.o"];
            args.extend_from_slice(flags);
            drop(directory.built(&args));
            assert!(!holds(&directory.path("a.o"), b".debug_"), "{flags:?}");
        }
    }

    #[test]
    fn the_compile_unit_is_honest_about_its_producer_and_language() {
        let directory = Directory::new();
        drop(directory.built(&[
            "--gcc-mode",
            LINUX,
            "-g",
            "-std=c99",
            "-c",
            "a.c",
            "-o",
            "a.o",
        ]));
        let Some(info) = directory.tool("readelf", &["--debug-dump=info"], "a.o") else {
            return;
        };
        assert!(
            info.contains(concat!("bcc-rust ", env!("CARGO_PKG_VERSION"))),
            "{info}"
        );
        assert!(info.contains("(ANSI C99)"), "{info}");
        assert!(info.contains("a.c"), "{info}");
        assert!(info.contains("Version:       5"), "{info}");
        for function in ["helper", "add", "moved", "main"] {
            assert!(info.contains(function), "{function}: {info}");
        }
    }

    /// `#line` and `#include` name the file and line, and a macro the line of
    /// its use, at both levels.
    #[test]
    fn the_line_table_follows_logical_positions() {
        let directory = Directory::new();
        for level in ["-O0", "-O2"] {
            drop(directory.built(&["--gcc-mode", LINUX, "-g", level, "-c", "a.c", "-o", "a.o"]));
            let Some(table) = directory.tool("readelf", &["--debug-dump=decodedline"], "a.o")
            else {
                return;
            };
            let rows = rows(&table);
            let has = |file: &str, line| rows.iter().any(|row| row.0 == file && row.1 == line);
            assert!(has("other.c", 102), "{level}: {table}");
            assert!(has("inc.h", 3), "{level}: {table}");
            // The macro's use, not its definition's lines 2 and 3.
            assert!(has("a.c", 8), "{level}: {table}");
            assert!(!has("a.c", 2) && !has("a.c", 3), "{level}: {table}");
        }
    }

    /// Each function's address gives its name, file and line, in a linked
    /// program that still runs.
    #[cfg(target_os = "linux")]
    #[test]
    fn addr2line_finds_the_function_file_and_line() {
        let directory = Directory::new();
        for (level, expected) in [
            (
                "-O0",
                &[
                    ("add", "a.c", 6),
                    ("moved", "other.c", 101),
                    ("main", "a.c", 20),
                ][..],
            ),
            // Without a prologue the entry is the first statement, as with
            // Clang.
            ("-O2", &[("add", "a.c", 8), ("moved", "other.c", 102)]),
        ] {
            drop(directory.built(&["--gcc-mode", "-g", level, "a.c", "-o", "a"]));
            assert!(
                Command::new(directory.path("a"))
                    .status()
                    .unwrap()
                    .success(),
                "{level}"
            );
            let Some(symbols) = directory.tool("nm", &[], "a") else {
                return;
            };
            for &(function, file, line) in expected {
                let address = symbols
                    .lines()
                    .find(|symbol| symbol.ends_with(&format!(" T {function}")))
                    .and_then(|symbol| symbol.split(' ').next())
                    .unwrap();
                let Some(found) =
                    directory.tool("addr2line", &["-f", &format!("0x{address}"), "-e"], "a")
                else {
                    return;
                };
                let mut found = found.lines();
                assert_eq!(found.next(), Some(function), "{level}");
                let place = found.next().unwrap();
                assert!(
                    place.ends_with(&format!("{file}:{line}")),
                    "{level} {function}: {place}"
                );
            }
        }
    }

    /// Inlined code names its own function and the call, as `addr2line -i`
    /// walks them.
    #[test]
    fn an_inlined_call_has_its_subroutine() {
        let directory = Directory::new();
        drop(directory.built(&["--gcc-mode", LINUX, "-g", "-O2", "-c", "a.c", "-o", "a.o"]));
        let Some(info) = directory.tool("readelf", &["--debug-dump=info"], "a.o") else {
            return;
        };
        assert!(info.contains("DW_TAG_inlined_subroutine"), "{info}");
    }

    /// `-g` must not change code generation at any level.
    #[test]
    fn the_machine_code_is_the_same_with_and_without_g() {
        let directory = Directory::new();
        std::fs::write(directory.path("code.c"), CODE).unwrap();
        for level in ["-O0", "-O1", "-O2", "-O3", "-Os", "-Og"] {
            drop(directory.built(&["--gcc-mode", level, "-c", "code.c", "-o", "same.o"]));
            let Some(plain) = directory.tool("objdump", &["-d"], "same.o") else {
                return;
            };
            drop(directory.built(&["--gcc-mode", level, "-g", "-c", "code.c", "-o", "same.o"]));
            let debug = directory.tool("objdump", &["-d"], "same.o").unwrap();
            assert_eq!(plain, debug, "{level}");
        }
    }

    /// A little of what lowering and the passes treat specially.
    const CODE: &str = "\
struct pair { int a; long b; };
static inline int twice(int x) { return x + x; }
static struct pair make(int a, long b) { struct pair p = { a, b }; return p; }
extern int printf(const char *, ...);
int sum(int n, const int *v)
{
  int total = 0;
  for (int i = 0; i < n; i++) {
    switch (v[i] & 3) {
    case 0: total += twice(v[i]); break;
    case 1: total -= v[i]; break;
    default: continue;
    }
    if (total > 100)
      goto done;
  }
done:
  return total;
}
long use(int n)
{
  int vla[n + 1];
  for (int i = 0; i <= n; i++)
    vla[i] = i;
  struct pair p = make(sum(n, vla), n);
  printf(\"%d %ld\\n\", p.a, p.b);
  return p.a ? p.b : 0;
}
";

    #[test]
    fn the_dwarf_version_is_the_last_asked_for() {
        let directory = Directory::new();
        for (flags, version) in [
            (&["-gdwarf-2"][..], 2),
            (&["-gdwarf-3"], 3),
            (&["-gdwarf-4"], 4),
            (&["-gdwarf-5"], 5),
            (&["-gdwarf-4", "-gdwarf-5"], 5),
            (&["-g", "-gdwarf-4"], 4),
            (&["-gdwarf-4", "-g"], 4),
            (&["-g0", "-gdwarf-3"], 3),
        ] {
            let mut args = vec!["--gcc-mode", LINUX, "-c", "a.c", "-o", "a.o"];
            args.extend_from_slice(flags);
            drop(directory.built(&args));
            let Some(info) = directory.tool("readelf", &["--debug-dump=info"], "a.o") else {
                return;
            };
            assert!(
                info.contains(&format!("Version:       {version}\n")),
                "{flags:?}: {info}"
            );
        }
    }

    /// CodeView for an MSVC target unless DWARF is asked for, and DWARF for
    /// MinGW, as with Clang. Not run on Windows here; the bytes are the
    /// object's whatever the host.
    #[test]
    fn windows_targets_choose_their_debug_format() {
        let directory = Directory::new();
        for (target, flags, section) in [
            (
                "--target=x86_64-pc-windows-msvc",
                &["-g"][..],
                &b".debug$S"[..],
            ),
            (
                "--target=x86_64-pc-windows-msvc",
                &["-gdwarf-4"],
                b".debug_info",
            ),
            ("--target=x86_64-w64-windows-gnu", &["-g"], b".debug_info"),
        ] {
            let mut args = vec!["--clang-mode", target, "-c", "t.c", "-o", "t.o"];
            args.extend_from_slice(flags);
            std::fs::write(directory.path("t.c"), "int t(int a) { return a + 1; }\n").unwrap();
            drop(directory.built(&args));
            assert!(holds(&directory.path("t.o"), section), "{target} {flags:?}");
        }
    }

    #[test]
    fn the_back_ends_without_debug_information_say_so_or_stay_quiet() {
        let directory = Directory::new();
        std::fs::write(directory.path("t.c"), "int main(void) { return 3; }\n").unwrap();
        let stderr = directory.built(&["-g", "--backend=native", LINUX, "-c", "t.c", "-o", "t.o"]);
        assert!(stderr.contains("writes no debug information"), "{stderr}");
        assert!(!holds(&directory.path("t.o"), b".debug_info"));
        let stderr = directory.built(&["--target=wasm32-wasip1", "-g", "t.c", "-o", "t.wasm"]);
        assert!(stderr.contains("writes no debug information"), "{stderr}");
        let run = directory.bcc(&["-g", "--interpret", "t.c"]);
        assert_eq!(run.status.code(), Some(3));
        assert!(
            run.stderr.is_empty(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
    }

    /// The text of `--emit=ir` is the same with `-g`; only the option that
    /// asks for the locations prints them, and `-gno-column-info` has none.
    #[test]
    fn the_ir_prints_locations_only_when_asked() {
        let directory = Directory::new();
        let ir = |flags: &[&str]| {
            let mut args = vec![LINUX, "--emit=ir", "a.c"];
            args.extend_from_slice(flags);
            let output = directory.bcc(&args);
            assert!(output.status.success(), "{flags:?}");
            String::from_utf8(output.stdout).unwrap()
        };
        assert_eq!(ir(&[]), ir(&["-g"]));
        let located = ir(&["-g", "--ir-debug-locations"]);
        assert!(located.contains(" dbg loc"), "{located}");
        assert!(located.contains("debug unit file0 language"), "{located}");
        assert!(located.contains("@other.c"), "{located}");
        assert!(!ir(&["--ir-debug-locations"]).contains(" dbg loc"));
        let columns = ir(&["-g", "-gno-column-info", "--ir-debug-locations"]);
        assert!(columns.contains("column 0"), "{columns}");
        assert!(!columns.contains("column 3"), "{columns}");
    }

    /// An assembly input is described too, as GCC and Clang pass `-g` on.
    #[test]
    fn an_assembly_input_gets_a_line_table() {
        let directory = Directory::new();
        std::fs::write(directory.path("s.s"), ".text\n.globl s\ns:\n  ret\n").unwrap();
        drop(directory.built(&["--gcc-mode", LINUX, "-g", "-c", "s.s", "-o", "s.o"]));
        assert!(holds(&directory.path("s.o"), b".debug_line"));
        drop(directory.built(&["--gcc-mode", LINUX, "-c", "s.s", "-o", "s.o"]));
        assert!(!holds(&directory.path("s.o"), b".debug_line"));
    }
}
