//! The arguments `cl` and clang-cl read besides `argv`: the `CL`
//! environment variable before them, `_CL_` after them, and the contents of
//! each command file (`@file`) in its place. Both are split into arguments
//! as the Microsoft C runtime splits a command line.
//!
//! `cl` passes the rest of a command file's line, or of an environment
//! variable, after `/link` to the linker and reads its later lines and the
//! rest of the command line as compiler options again; a command file
//! cannot name another. clang-cl reads a command file as one stream of
//! arguments, which may name further command files (LLVM's
//! `ExpandResponseFiles` with its Windows tokenizer), and reads `CL` and
//! `_CL_` as `cl` does (`clang/tools/driver/driver.cpp`).
//!
//! Microsoft Learn: "CL environment variables", "CL Command Files", "@
//! (Specify a Compiler Response File)", "Parsing C command-line
//! arguments".

use std::ffi::OsString;

/// How deeply clang-cl's command files may name each other before bcc
/// gives up, as LLVM gives up on a cycle.
const MAX_NESTING: usize = 32;

/// `arguments` with `CL` before them, `_CL_` after them and every
/// `@file` replaced by its contents; linker options that a command file
/// line or an environment variable gave after `/link` come last, after a
/// `/link` of their own unless the arguments already hold one. A command
/// file that cannot be read is `Err` with its name.
pub(super) fn expand(arguments: &[OsString], clang_cl: bool) -> Result<Vec<OsString>, OsString> {
    let mut expansion = Expansion {
        clang_cl,
        arguments: Vec::new(),
        link: Vec::new(),
    };
    let variable = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
    if let Some(prepended) = variable("CL") {
        expansion.line(&split(&prepended.to_string_lossy()), 0)?;
    }
    for argument in arguments {
        // An argument after `/link` on the command line is the linker's,
        // whatever it looks like.
        if expansion.linking() {
            expansion.arguments.push(argument.clone());
        } else {
            expansion.argument(argument.clone(), 0)?;
        }
    }
    if let Some(appended) = variable("_CL_") {
        expansion.line(&split(&appended.to_string_lossy()), 0)?;
    }
    let Expansion {
        mut arguments,
        link,
        ..
    } = expansion;
    if !link.is_empty() {
        if !arguments.iter().any(|argument| is_link(argument)) {
            arguments.push("/link".into());
        }
        arguments.extend(link);
    }
    Ok(arguments)
}

/// The arguments read so far.
struct Expansion {
    clang_cl:  bool,
    arguments: Vec<OsString>,
    /// What lines of command files and the environment variables gave
    /// after their `/link`.
    link:      Vec<OsString>,
}

impl Expansion {
    /// Reads one argument at nesting `depth`: a command file's contents,
    /// or the argument itself.
    fn argument(&mut self, argument: OsString, depth: usize) -> Result<(), OsString> {
        let Some(file) = argument
            .to_str()
            .and_then(|text| text.strip_prefix('@'))
            .filter(|file| !file.is_empty() && (depth == 0 || self.clang_cl))
        else {
            self.arguments.push(argument);
            return Ok(());
        };
        let text = match std::fs::read(file) {
            | Ok(bytes) if depth < MAX_NESTING => decode(&bytes),
            // clang-cl leaves a command file it cannot read on the
            // command line, as an input it then reports missing.
            | _ if self.clang_cl => {
                self.arguments.push(argument);
                return Ok(());
            },
            | _ => return Err(file.into()),
        };
        if self.clang_cl {
            for word in split(&text) {
                self.argument(word.into(), depth + 1)?;
            }
        } else {
            for line in text.lines() {
                self.line(&split(line), depth + 1)?;
            }
        }
        Ok(())
    }

    /// Reads the words of one line of a command file or environment
    /// variable, whose words after `/link` are the linker's; clang-cl
    /// reads a `/link` there as on the command line.
    fn line(&mut self, words: &[String], depth: usize) -> Result<(), OsString> {
        let mut words = words.iter();
        while let Some(word) = words.next() {
            if !self.clang_cl && is_link(word.as_ref()) {
                self.link.extend(words.by_ref().map(OsString::from));
                break;
            }
            if self.linking() {
                self.arguments.push(word.into());
            } else {
                self.argument(word.into(), depth)?;
            }
        }
        Ok(())
    }

    /// Whether the arguments so far end in a command-line `/link`, after
    /// which every argument is the linker's.
    fn linking(&self) -> bool {
        self.arguments.iter().any(|argument| is_link(argument))
    }
}

fn is_link(argument: &std::ffi::OsStr) -> bool {
    argument == "/link" || argument == "-link"
}

/// A command file's text: UTF-16 after its byte order mark, as MSBuild
/// writes them, otherwise UTF-8 without one.
fn decode(bytes: &[u8]) -> String {
    let utf16 = |bytes: &[u8], from: fn([u8; 2]) -> u16| {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|pair| from([pair[0], pair[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    };
    match bytes {
        | [0xFF, 0xFE, rest @ ..] => utf16(rest, u16::from_le_bytes),
        | [0xFE, 0xFF, rest @ ..] => utf16(rest, u16::from_be_bytes),
        | [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        | _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// Splits `text` into arguments as the Microsoft C runtime does: spaces,
/// tabs and line ends separate them outside double quotes; `2n`
/// backslashes before a quote are `n` and the quote opens or closes a
/// quoted part, `2n + 1` are `n` and a literal quote; `""` inside a quoted
/// part is a literal quote; other backslashes are literal.
/// Microsoft Learn: "Parsing C command-line arguments".
pub(super) fn split(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    // Whether a word has begun, so `""` gives an empty argument.
    let mut started = false;
    let mut quoted = false;
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            | '\\' => {
                let mut backslashes = 1;
                while characters.next_if_eq(&'\\').is_some() {
                    backslashes += 1;
                }
                started = true;
                if characters.peek() == Some(&'"') {
                    word.extend(std::iter::repeat_n('\\', backslashes / 2));
                    if backslashes % 2 == 1 {
                        word.push('"');
                        _ = characters.next();
                    }
                } else {
                    word.extend(std::iter::repeat_n('\\', backslashes));
                }
            },
            | '"' => {
                started = true;
                if quoted && characters.next_if_eq(&'"').is_some() {
                    word.push('"');
                } else {
                    quoted = !quoted;
                }
            },
            | ' ' | '\t' | '\r' | '\n' if !quoted => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            },
            | _ => {
                started = true;
                word.push(character);
            },
        }
    }
    if started {
        words.push(word);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::split;

    #[test]
    fn splits_as_the_microsoft_c_runtime_does() {
        for (text, words) in [
            ("/c  a.c\tb.c", &["/c", "a.c", "b.c"][..]),
            (r#""a b.c" /DX="1 2""#, &["a b.c", "/DX=1 2"]),
            (r#"a\\b \"q\" \\"x y""#, &[r"a\\b", r#""q""#, r"\x y"]),
            (r#""say ""hi""" """#, &[r#"say "hi""#, ""]),
            ("one\r\ntwo\n", &["one", "two"]),
        ] {
            assert_eq!(split(text), words, "{text:?}");
        }
    }
}
