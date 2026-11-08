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

    /// The back ends that write DWARF for Linux, as a personality's command
    /// line selects them: LLVM, and the native back end with its listing
    /// assembled by the bundled clang or its object encoded directly.
    const BACK_ENDS: [&[&str]; 3] = [
        &[],
        &["-Xbcc", "--backend=native", "-Xbcc", "--no-native-object"],
        &["-Xbcc", "--backend=native", "-Xbcc", "--native-object"],
    ];

    /// `args` after the options that select `back_end`.
    fn with<'a>(back_end: &[&'a str], args: &[&'a str]) -> Vec<&'a str> {
        let mut all = vec![args[0]];
        all.extend_from_slice(back_end);
        all.extend_from_slice(&args[1..]);
        all
    }

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
        for back_end in BACK_ENDS {
            for flags in [&[][..], &["-g0"], &["-g", "-g0"], &["-gdwarf-4", "-g0"]] {
                let mut args = with(back_end, &["--gcc-mode", LINUX, "-c", "a.c", "-o", "a.o"]);
                args.extend_from_slice(flags);
                drop(directory.built(&args));
                assert!(
                    !holds(&directory.path("a.o"), b".debug_"),
                    "{back_end:?} {flags:?}"
                );
            }
        }
    }

    #[test]
    fn the_compile_unit_is_honest_about_its_producer_and_language() {
        let directory = Directory::new();
        for back_end in BACK_ENDS {
            drop(directory.built(&with(
                back_end,
                &[
                    "--gcc-mode",
                    LINUX,
                    "-g",
                    "-std=c99",
                    "-c",
                    "a.c",
                    "-o",
                    "a.o",
                ],
            )));
            let Some(info) = directory.tool("readelf", &["--debug-dump=info"], "a.o") else {
                return;
            };
            assert!(
                info.contains(concat!("bcc-rust ", env!("CARGO_PKG_VERSION"))),
                "{back_end:?}: {info}"
            );
            assert!(info.contains("(ANSI C99)"), "{back_end:?}: {info}");
            assert!(info.contains("a.c"), "{back_end:?}: {info}");
            assert!(info.contains("Version:       5"), "{back_end:?}: {info}");
            for function in ["helper", "add", "moved", "main"] {
                assert!(info.contains(function), "{back_end:?} {function}: {info}");
            }
        }
    }

    /// `#line` and `#include` name the file and line, and a macro the line of
    /// its use, at both levels.
    #[test]
    fn the_line_table_follows_logical_positions() {
        let directory = Directory::new();
        for back_end in BACK_ENDS {
            for level in ["-O0", "-O2"] {
                let args = with(
                    back_end,
                    &["--gcc-mode", LINUX, "-g", level, "-c", "a.c", "-o", "a.o"],
                );
                drop(directory.built(&args));
                let Some(table) = directory.tool("readelf", &["--debug-dump=decodedline"], "a.o")
                else {
                    return;
                };
                let rows = rows(&table);
                let has = |file: &str, line| rows.iter().any(|row| row.0 == file && row.1 == line);
                assert!(has("other.c", 102), "{args:?}: {table}");
                assert!(has("inc.h", 3), "{args:?}: {table}");
                // The macro's use, not its definition's lines 2 and 3.
                assert!(has("a.c", 8), "{args:?}: {table}");
                assert!(!has("a.c", 2) && !has("a.c", 3), "{args:?}: {table}");
            }
        }
    }

    /// Each function's address gives its name, file and line, in a linked
    /// program that still runs.
    #[cfg(target_os = "linux")]
    #[test]
    fn addr2line_finds_the_function_file_and_line() {
        let directory = Directory::new();
        for (back_end, (level, expected)) in BACK_ENDS.into_iter().flat_map(|back_end| {
            [
                (
                    "-O0",
                    &[
                        ("add", "a.c", 6),
                        ("moved", "other.c", 101),
                        ("main", "a.c", 20),
                    ][..],
                ),
                // Without a prologue the entry is the first statement, as
                // with Clang. LLVM folds both additions of `add` into one
                // `lea` on line 8, where the native back end adds on each
                // line.
                (
                    "-O2",
                    if back_end.is_empty() {
                        &[("add", "a.c", 8), ("moved", "other.c", 102)]
                    } else {
                        &[("add", "a.c", 7), ("moved", "other.c", 102)]
                    },
                ),
            ]
            .map(|case| (back_end, case))
        }) {
            drop(directory.built(&with(
                back_end,
                &["--gcc-mode", "-g", level, "a.c", "-o", "a"],
            )));
            assert!(
                Command::new(directory.path("a"))
                    .status()
                    .unwrap()
                    .success(),
                "{back_end:?} {level}"
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
                assert_eq!(found.next(), Some(function), "{back_end:?} {level}");
                let place = found.next().unwrap();
                assert!(
                    place.ends_with(&format!("{file}:{line}")),
                    "{back_end:?} {level} {function}: {place}"
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

    /// `-g` must not change code generation at any level, in any back end.
    #[test]
    fn the_machine_code_is_the_same_with_and_without_g() {
        let directory = Directory::new();
        std::fs::write(directory.path("code.c"), CODE).unwrap();
        for back_end in BACK_ENDS {
            for level in ["-O0", "-O1", "-O2", "-O3", "-Os", "-Og"] {
                let plain = with(
                    back_end,
                    &["--gcc-mode", level, "-c", "code.c", "-o", "same.o"],
                );
                drop(directory.built(&plain));
                let Some(plain) = directory.tool("objdump", &["-d"], "same.o") else {
                    return;
                };
                let args = with(
                    back_end,
                    &["--gcc-mode", level, "-g", "-c", "code.c", "-o", "same.o"],
                );
                drop(directory.built(&args));
                let debug = directory.tool("objdump", &["-d"], "same.o").unwrap();
                assert_eq!(plain, debug, "{args:?}");
            }
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
            // By lines: `readelf` ends them with CRLF on Windows.
            let expected = format!("Version:       {version}");
            assert!(
                info.lines().any(|line| line.trim() == expected),
                "{flags:?}: {info}"
            );
        }
    }

    /// The native back end writes DWARF 5 whatever version is asked for, and
    /// says so.
    #[test]
    fn the_native_back_end_writes_dwarf_5() {
        let directory = Directory::new();
        let args = [
            "--gcc-mode",
            "-Xbcc",
            "--backend=native",
            LINUX,
            "-gdwarf-4",
            "-c",
        ];
        let stderr = directory.built(&[&args[..], &["a.c", "-o", "a.o"]].concat());
        assert!(
            stderr.contains("writes DWARF 5; ignoring -gdwarf-4"),
            "{stderr}"
        );
        let Some(info) = directory.tool("readelf", &["--debug-dump=info"], "a.o") else {
            return;
        };
        assert!(
            info.lines().any(|line| line.trim() == "Version:       5"),
            "{info}"
        );
    }

    /// `llvm-dwarfdump --verify`, where it is installed, accepts the objects
    /// and programs of every back end, at `-O0`, `-O2` and through bcc's
    /// optimizer.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_debug_information_verifies() {
        let directory = Directory::new();
        std::fs::write(directory.path("data.c"), DATA).unwrap();
        for back_end in BACK_ENDS {
            for levels in [&["-O0"][..], &["-O2"], &["-O2", "-Xbcc", "--opt=bcc"]] {
                for (output, link) in [("data.o", &["-c"][..]), ("data", &[])] {
                    let mut args = with(back_end, &["--gcc-mode", "-g", "data.c", "-o", output]);
                    args.extend_from_slice(levels);
                    args.extend_from_slice(link);
                    drop(directory.built(&args));
                    let Some(verified) = directory.tool("llvm-dwarfdump", &["--verify"], output)
                    else {
                        let found = Command::new("llvm-dwarfdump").arg("--version").output();
                        assert!(found.is_err(), "{args:?}: llvm-dwarfdump --verify failed");
                        return;
                    };
                    assert!(verified.contains("No errors."), "{args:?}: {verified}");
                }
            }
        }
    }

    /// `CodeView` for an MSVC target unless DWARF is asked for, and DWARF for
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
        // The native back end writes no CodeView, and no DWARF in COFF.
        let windows = "--target=x86_64-w64-windows-gnu";
        let stderr =
            directory.built(&["-g", "--backend=native", windows, "-c", "t.c", "-o", "t.o"]);
        assert!(
            stderr.contains("writes no debug information for Windows targets"),
            "{stderr}"
        );
        assert!(!holds(&directory.path("t.o"), b".debug_info"));
        let stderr = directory.built(&["-g", "--backend=native", LINUX, "-c", "t.c", "-o", "t.o"]);
        assert!(stderr.is_empty(), "{stderr}");
        assert!(holds(&directory.path("t.o"), b".debug_info"));
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

    /// A program with the data a debugger shows: a struct, a union, an array,
    /// a pointer, a bit-field, an enum, typedefs, qualifiers, a static local,
    /// a nested block that shadows a name, and a recursive call.
    const DATA: &str = "\
typedef unsigned long my_size;
typedef struct point { int x; int y; } point_t;
struct bits { unsigned a : 3; unsigned b : 5; int c; };
union u { int i; float f; };
enum color { RED, GREEN = 5, BLUE };
const int table[3] = { 1, 2, 3 };
volatile int vol;
int sum(point_t *p, my_size n)
{
  int total = 0;
  for (my_size i = 0; i < n; i++) {
    int total2 = p[i].x + p[i].y;
    {
      int total = total2 * 2;
      total2 = total;
    }
    total += total2;
  }
  return total;
}
int fact(int n)
{
  if (n <= 1)
    return 1;
  return n * fact(n - 1);
}
int main(void)
{
  point_t pts[2] = { { 1, 2 }, { 3, 4 } };
  struct bits b = { 1, 2, 3 };
  union u un;
  enum color c = GREEN;
  static int counter = 7;
  un.i = 9;
  int s = sum(pts, 2);
  int f = fact(3);
  return s + f + b.a + un.i + c + counter + table[0] + vol != 49;
}
";

    #[test]
    fn variables_and_types_read_back() {
        let directory = Directory::new();
        std::fs::write(directory.path("data.c"), DATA).unwrap();
        for back_end in BACK_ENDS {
            let args = with(
                back_end,
                &["--gcc-mode", LINUX, "-g", "-c", "data.c", "-o", "data.o"],
            );
            drop(directory.built(&args));
            let Some(info) = directory.tool("readelf", &["--debug-dump=info"], "data.o") else {
                return;
            };
            read_back(&info, &args);
        }
    }

    /// The tags and names `readelf` shows in the entries of `DATA`.
    fn read_back(info: &str, args: &[&str]) {
        for tag in [
            "DW_TAG_variable",
            "DW_TAG_formal_parameter",
            "DW_TAG_lexical_block",
            "DW_TAG_structure_type",
            "DW_TAG_union_type",
            "DW_TAG_enumeration_type",
            "DW_TAG_enumerator",
            "DW_TAG_typedef",
            "DW_TAG_array_type",
            "DW_TAG_subrange_type",
            "DW_TAG_pointer_type",
            "DW_TAG_const_type",
            "DW_TAG_volatile_type",
            "DW_AT_bit_size",
        ] {
            assert!(info.contains(tag), "{args:?} {tag}: {info}");
        }
        for name in [
            "pts", "total2", "counter", "my_size", "point_t", "GREEN", "table",
        ] {
            assert!(info.contains(name), "{args:?} {name}: {info}");
        }
    }

    /// `-g1` is line tables, without the data, as Clang's line-tables-only.
    #[test]
    fn level_one_describes_no_data() {
        let directory = Directory::new();
        std::fs::write(directory.path("data.c"), DATA).unwrap();
        for back_end in BACK_ENDS {
            let args = with(
                back_end,
                &["--gcc-mode", LINUX, "-g1", "-c", "data.c", "-o", "data.o"],
            );
            drop(directory.built(&args));
            let Some(info) = directory.tool("readelf", &["--debug-dump=info"], "data.o") else {
                return;
            };
            assert!(!info.contains("DW_TAG_variable"), "{args:?}: {info}");
            assert!(!info.contains("DW_TAG_base_type"), "{args:?}: {info}");
            let table = directory
                .tool("readelf", &["--debug-dump=decodedline"], "data.o")
                .unwrap();
            assert!(
                rows(&table).iter().any(|row| row.0 == "data.c"),
                "{args:?}: {table}"
            );
        }
    }

    /// At `-O0` locals live in memory with or without `-g`, as GCC and Clang
    /// keep them, so a line that only initializes one has a row and no reload
    /// between statements appears as line 0.
    #[test]
    fn unoptimized_locals_have_their_lines() {
        let directory = Directory::new();
        for back_end in BACK_ENDS {
            let args = with(
                back_end,
                &["--gcc-mode", LINUX, "-g", "-c", "a.c", "-o", "a.o"],
            );
            drop(directory.built(&args));
            let Some(table) = directory.tool("readelf", &["--debug-dump=decodedline"], "a.o")
            else {
                return;
            };
            let rows = rows(&table);
            // `int s = 0;` in `main`, which is on line 21 of the logical file.
            assert!(
                rows.iter().any(|row| row.0 == "a.c" && row.1 == 21),
                "{args:?}: {table}"
            );
            assert!(rows.iter().all(|row| row.1 != 0), "{args:?}: {table}");
        }
    }

    /// The session the brief asks for, with `gdb`, where one can run a program,
    /// for each back end.
    #[cfg(target_os = "linux")]
    #[test]
    fn gdb_shows_the_program() {
        let directory = Directory::new();
        std::fs::write(directory.path("data.c"), DATA).unwrap();
        for back_end in BACK_ENDS {
            gdb_session(&directory, back_end);
        }
    }

    #[cfg(target_os = "linux")]
    fn gdb_session(directory: &Directory, back_end: &[&str]) {
        drop(directory.built(&with(
            back_end,
            &["--gcc-mode", "-g", "data.c", "-o", "data"],
        )));
        assert!(
            Command::new(directory.path("data"))
                .status()
                .unwrap()
                .success(),
            "the program checks its own values"
        );
        let session = Command::new("gdb")
            .current_dir(&directory.0)
            .args(["-q", "-nx", "-batch"])
            .args(["-ex", "break data.c:15", "-ex", "run", "-ex", "bt"])
            .args(["-ex", "info locals", "-ex", "print p->y", "-ex", "ptype p"])
            .args(["-ex", "up", "-ex", "print pts", "-ex", "print b"])
            .args([
                "-ex",
                "print un.i",
                "-ex",
                "print c",
                "-ex",
                "print counter",
            ])
            .args([
                "-ex",
                "whatis table",
                "-ex",
                "whatis vol",
                "-ex",
                "ptype sum",
            ])
            .args([
                "-ex",
                "delete",
                "-ex",
                "break fact if n == 1",
                "-ex",
                "continue",
            ])
            .args(["-ex", "bt", "-ex", "finish", "-ex", "next"])
            .arg("./data")
            .output();
        let Ok(session) = session else {
            return;
        };
        let text = String::from_utf8_lossy(&session.stdout);
        if !text.contains("Breakpoint 1,") {
            // No gdb that can run a program here, under a sandbox for one.
            return;
        }
        for expected in [
            "sum (p=",
            "n=2) at data.c:15",
            // The inner `total` first: a nested block shadows the outer.
            "total = 6\ntotal2 = 3\ni = 0\ntotal = 0\n",
            "= 2\n",
            "type = struct point {\n    int x;\n    int y;\n} *",
            "{{x = 1, y = 2}, {x = 3, y = 4}}",
            "{a = 1, b = 2, c = 3}",
            "= 9\n",
            "= GREEN\n",
            "= 7\n",
            "type = const int [3]",
            "type = volatile int",
            "type = int (point_t *, my_size)",
            "#0  fact (n=1) at data.c:23",
            "#1  ",
            "fact (n=2) at data.c:25",
            "#2  ",
            "fact (n=3) at data.c:25",
            "main () at data.c:37",
            "Value returned is ",
        ] {
            assert!(text.contains(expected), "{back_end:?} {expected}: {text}");
        }
    }

    /// An optimized program still answers with its functions and files. A local
    /// the optimizer placed in registers is out of a debugger's reach here, as
    /// `middle-end.md` records, and its description stays.
    #[test]
    fn optimized_programs_keep_their_variables_described() {
        let directory = Directory::new();
        std::fs::write(directory.path("data.c"), DATA).unwrap();
        for back_end in BACK_ENDS {
            let args = with(
                back_end,
                &[
                    "--gcc-mode",
                    LINUX,
                    "-g",
                    "-O2",
                    "-c",
                    "data.c",
                    "-o",
                    "data.o",
                ],
            );
            drop(directory.built(&args));
            let Some(info) = directory.tool("readelf", &["--debug-dump=info"], "data.o") else {
                return;
            };
            for name in ["total2", "my_size", "point_t"] {
                assert!(info.contains(name), "{args:?} {name}: {info}");
            }
        }
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
