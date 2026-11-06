//! `@FILE` on a GCC or Clang command line: the arguments in FILE stand where
//! the `@FILE` stood, and FILE may name further ones, against the working
//! directory. An `@FILE` that does not exist stays on the command line as an
//! input, which the compiler then reports missing; a directory is an error.
//!
//! Both compilers split a file the same way: at white space, except inside
//! single or double quotes, with a backslash taking the next byte literally in
//! either, and the bytes need not be UTF-8. They differ in what is left over.
//! GCC (libiberty's `expandargv` and `buildargv`) stops reading a file at a
//! NUL, keeps an empty `""`, drops a backslash that ends the file, reads `@`
//! alone as an input, and stops after 2000 `@` arguments. Clang (LLVM's
//! `ExpandResponseFiles`, whose `TokenizeGNUCommandLine` it selects) reads a
//! NUL as white space, drops an empty `""`, keeps a final backslash, reads `@`
//! alone as the working directory, and refuses a file that names itself.

use std::{
    ffi::{
        OsStr,
        OsString,
    },
    path::{
        Path,
        PathBuf,
    },
};

use super::Compiler;

/// How many `@` arguments GCC reads before it gives up.
const GCC_LIMIT: u32 = 2000;

/// Why the arguments could not be read.
pub(super) enum ResponseFileError {
    /// GCC met too many `@` arguments.
    TooMany,
    /// GCC was named a directory.
    GccDirectory,
    /// Clang was named a directory; by its full path.
    ClangDirectory(PathBuf),
    /// A file of Clang's names itself, directly or through others; the file
    /// by its full path.
    Recursive(PathBuf),
}

/// `arguments` with each readable `@FILE` replaced by the arguments it holds.
pub(super) fn expand(
    arguments: &[OsString],
    compiler: Compiler,
) -> Result<Vec<OsString>, ResponseFileError> {
    let mut expansion = Expansion {
        compiler,
        arguments: Vec::with_capacity(arguments.len()),
        read: 0,
        open: Vec::new(),
    };
    for argument in arguments {
        expansion.argument(argument.clone())?;
    }
    Ok(expansion.arguments)
}

struct Expansion {
    compiler:  Compiler,
    arguments: Vec<OsString>,
    /// The `@` arguments met so far, which GCC limits.
    read:      u32,
    /// The files being read, outermost first, each by its full path: naming
    /// one again would never end, which Clang refuses.
    open:      Vec<PathBuf>,
}

impl Expansion {
    /// Reads one argument.
    fn argument(&mut self, argument: OsString) -> Result<(), ResponseFileError> {
        let clang = self.compiler == Compiler::Clang;
        let Some(file) = name_after_at(&argument).filter(|file| clang || !file.is_empty()) else {
            self.arguments.push(argument);
            return Ok(());
        };
        if !clang {
            self.read += 1;
            if self.read >= GCC_LIMIT {
                return Err(ResponseFileError::TooMany);
            }
        }
        let path = if file.is_empty() {
            PathBuf::from(".")
        } else {
            PathBuf::from(&file)
        };
        if path.is_dir() {
            return Err(if clang {
                ResponseFileError::ClangDirectory(full_path(&path))
            } else {
                ResponseFileError::GccDirectory
            });
        }
        let Ok(bytes) = std::fs::read(&path) else {
            self.arguments.push(argument);
            return Ok(());
        };
        if clang {
            let full = full_path(&path);
            if self.open.contains(&full) {
                return Err(ResponseFileError::Recursive(full));
            }
            self.open.push(full);
        }
        for word in split(&bytes, self.compiler) {
            self.argument(os_string(word))?;
        }
        if clang {
            drop(self.open.pop());
        }
        Ok(())
    }
}

/// `path` from the root, as Clang names a file in its messages.
fn full_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned())
}

/// What follows the `@` that starts `argument`.
#[cfg(unix)]
fn name_after_at(argument: &OsStr) -> Option<OsString> {
    use std::os::unix::ffi::OsStrExt as _;
    argument
        .as_bytes()
        .strip_prefix(b"@")
        .map(|name| OsStr::from_bytes(name).to_owned())
}

/// What follows the `@` that starts `argument`.
#[cfg(not(unix))]
fn name_after_at(argument: &OsStr) -> Option<OsString> {
    argument
        .to_str()
        .and_then(|text| text.strip_prefix('@'))
        .map(OsString::from)
}

/// An argument read from a file, whose bytes need not be text.
#[cfg(unix)]
fn os_string(bytes: Vec<u8>) -> OsString {
    use std::os::unix::ffi::OsStringExt as _;
    OsString::from_vec(bytes)
}

/// An argument read from a file. Windows names files in UTF-16, so bytes that
/// are not UTF-8 have no faithful argument there.
#[cfg(not(unix))]
fn os_string(bytes: Vec<u8>) -> OsString {
    String::from_utf8_lossy(&bytes).into_owned().into()
}

/// The arguments in `text`, as `compiler` splits them.
fn split(text: &[u8], compiler: Compiler) -> Vec<Vec<u8>> {
    let clang = compiler == Compiler::Clang;
    let text = if clang {
        text
    } else {
        text.split(|&byte| byte == 0).next().unwrap_or_default()
    };
    let mut words = Vec::new();
    let mut word = Vec::new();
    let mut started = false;
    let mut quote = None;
    let mut bytes = text.iter().copied();
    while let Some(byte) = bytes.next() {
        match (byte, quote) {
            | (b'\\', _) => match bytes.next() {
                | Some(next) => {
                    word.push(next);
                    started = true;
                },
                | None if clang => word.push(byte),
                | None => (),
            },
            | (_, Some(open)) if byte == open => quote = None,
            | (b'\'' | b'"', None) => {
                quote = Some(byte);
                started |= !clang;
            },
            | (b' ' | b'\t' | b'\n' | b'\r' | 0x0B | 0x0C | 0, None) => {
                if started || !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
                started = false;
            },
            | (_, Some(_) | None) => word.push(byte),
        }
    }
    if started || !word.is_empty() {
        words.push(word);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::{
        Compiler,
        split,
    };

    fn words(text: &[u8], compiler: Compiler) -> Vec<String> {
        split(text, compiler)
            .into_iter()
            .map(|word| String::from_utf8_lossy(&word).into_owned())
            .collect()
    }

    #[test]
    fn splits_at_white_space_outside_quotes() {
        for compiler in [Compiler::Gcc, Compiler::Clang] {
            assert_eq!(words(b"-c  a.c\n\tb.c\n", compiler), ["-c", "a.c", "b.c"]);
            assert_eq!(
                words(b"'a b' \"c d\" e' 'f", compiler),
                ["a b", "c d", "e f"]
            );
            assert_eq!(
                words(b"a\\ b 'c\\'d' \"e\\\"f\"", compiler),
                ["a b", "c'd", "e\"f"]
            );
            assert_eq!(words(b"", compiler), Vec::<String>::new());
        }
    }

    #[test]
    fn keeps_bytes_that_are_not_text() {
        assert_eq!(split(b"-DA=\xff", Compiler::Clang), [b"-DA=\xff"]);
    }

    #[test]
    fn the_compilers_differ_in_what_is_left_over() {
        assert_eq!(words(b"a \"\" b", Compiler::Gcc), ["a", "", "b"]);
        assert_eq!(words(b"a \"\" b", Compiler::Clang), ["a", "b"]);
        assert_eq!(words(b"a\\", Compiler::Gcc), ["a"]);
        assert_eq!(words(b"a\\", Compiler::Clang), ["a\\"]);
        assert_eq!(words(b"a\0b", Compiler::Gcc), ["a"]);
        assert_eq!(words(b"a\0b", Compiler::Clang), ["a", "b"]);
    }
}
