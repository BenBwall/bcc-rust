//! `@FILE` on a GCC or Clang command line: the arguments in FILE stand where
//! the `@FILE` stood, and FILE may name further ones. An `@FILE` that cannot
//! be read stays on the command line as an input, which the compiler then
//! reports missing.
//!
//! Both compilers split a file the same way: at white space, except inside
//! single or double quotes, with a backslash taking the next character
//! literally in either. They differ in four ways, followed here. GCC
//! (libiberty's `expandargv` and `buildargv`) resolves a nested name against
//! the working directory, keeps an empty `""`, and stops after 2000 `@`
//! arguments. Clang (LLVM's `ExpandResponseFiles`, whose
//! `TokenizeGNUCommandLine` it selects) resolves a nested name against the
//! file naming it, drops an empty `""`, and refuses a file that names itself.

use std::{
    ffi::OsString,
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
        expansion.argument(argument.clone(), Path::new(""))?;
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
    /// Reads one argument, which came from a file in `directory`.
    fn argument(&mut self, argument: OsString, directory: &Path) -> Result<(), ResponseFileError> {
        let Some(file) = argument
            .to_str()
            .and_then(|text| text.strip_prefix('@'))
            .filter(|file| !file.is_empty())
        else {
            self.arguments.push(argument);
            return Ok(());
        };
        let clang = self.compiler == Compiler::Clang;
        if !clang {
            self.read += 1;
            if self.read >= GCC_LIMIT {
                return Err(ResponseFileError::TooMany);
            }
        }
        let path = if clang {
            directory.join(file)
        } else {
            PathBuf::from(file)
        };
        let Ok(bytes) = std::fs::read(&path) else {
            self.arguments.push(argument);
            return Ok(());
        };
        if clang {
            let full = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            if self.open.contains(&full) {
                return Err(ResponseFileError::Recursive(full));
            }
            self.open.push(full);
        }
        let parent = path.parent().unwrap_or(Path::new("")).to_owned();
        for word in split(&String::from_utf8_lossy(&bytes), !clang) {
            self.argument(word.into(), &parent)?;
        }
        if clang {
            drop(self.open.pop());
        }
        Ok(())
    }
}

/// The arguments in `text`. An empty pair of quotes is an argument only when
/// `keep_empty`.
fn split(text: &str, keep_empty: bool) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote = None;
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        match (character, quote) {
            | ('\\', _) =>
                if let Some(next) = characters.next() {
                    word.push(next);
                    started = true;
                },
            | (_, Some(open)) if character == open => quote = None,
            | ('\'' | '"', None) => {
                quote = Some(character);
                started |= keep_empty;
            },
            | (_, None) if character.is_ascii_whitespace() || character == '\x0b' => {
                if started || !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
                started = false;
            },
            | (_, Some(_) | None) => word.push(character),
        }
    }
    if started || !word.is_empty() {
        words.push(word);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::split;

    #[test]
    fn splits_at_white_space_outside_quotes() {
        assert_eq!(split("-c  a.c\n\tb.c\n", true), ["-c", "a.c", "b.c"]);
        assert_eq!(split("'a b' \"c d\" e' 'f", true), ["a b", "c d", "e f"]);
        assert_eq!(
            split("a\\ b 'c\\'d' \"e\\\"f\"", true),
            ["a b", "c'd", "e\"f"]
        );
        assert_eq!(split("", true), Vec::<String>::new());
    }

    #[test]
    fn only_gcc_keeps_an_empty_argument() {
        assert_eq!(split("a \"\" b", true), ["a", "", "b"]);
        assert_eq!(split("a \"\" b", false), ["a", "b"]);
    }
}
