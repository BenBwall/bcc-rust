//! How a text of phase 4 (`-E`) spaces a token that macro replacement
//! produced: what GCC's padding tokens would have said about it, folded into
//! a byte on the token, so that no reader of the token stream has to skip
//! them.
//!
//! GCC's preprocessor puts a padding token before each expansion and each
//! substituted argument, naming the token whose white space stands for theirs
//! (the macro's name, the parameter), and one naming nothing after each. Its
//! printer keeps one `source` across a run of paddings (libcpp `macro.cc`
//! `enter_macro_context`, `replace_args`, `_cpp_pop_context`; `c-ppoutput.cc`
//! `scan_translation_unit`). [`TokenSpacing`] records a run of paddings as what
//! it does to that `source`, so that runs compose wherever a prescanned
//! argument's tokens are replayed.
//!
//! Where preprocessing tokens are separated is unspecified beyond not forming
//! other tokens; the text is the GCC and Clang `-E` convention.
//! C99: §5.1.1.2 paragraph 1 item 3, p. 10; PDF p. 22; §6.4 paragraph 4, p. 49;
//! PDF p. 61.

/// The token just read: whether white space came before it, and what the
/// paddings since the token before it do to the printer's `source`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct TokenSpacing(u8);

/// What the printer's `source` says of the white space before a token.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum Source {
    /// There is no source: the token speaks for itself.
    None     = 0,
    /// The source had white space before it.
    Spaced   = 1,
    /// The source had none.
    Unspaced = 2,
}

/// A padding was seen, so the printer checks that the tokens would not paste.
const AVOID_PASTE: u8 = 1;
/// The token itself has white space before it, GCC's `PREV_WHITE`.
const WHITE: u8 = 2;
/// The token is the right operand of a `##` that became a plain token: as for
/// the parameter of GCC's `, ## __VA_ARGS__`, its substitution has no padding
/// of its own.
const RIGHT_OPERAND: u8 = 64;
/// Where the result for a printer that held no source is.
const FROM_NONE: u8 = 2;
/// Where the result for a printer that held [`Source::Unspaced`] is. One that
/// held [`Source::Spaced`] keeps it, whatever follows.
const FROM_UNSPACED: u8 = 4;

impl TokenSpacing {
    /// No padding, as for a token read from a source file.
    pub(crate) const NONE: Self = Self::new(false, Source::None, Source::Unspaced);
    /// The padding with no source, after an expansion or an argument.
    pub(crate) const END: Self = Self::new(true, Source::None, Source::None);

    /// The padding whose source is a token with white space before it or
    /// without, before an expansion (the macro's name) or an argument (the
    /// parameter).
    pub(crate) const fn before(white: bool) -> Self {
        let source = if white {
            Source::Spaced
        } else {
            Source::Unspaced
        };
        Self::new(true, source, Source::Unspaced)
    }

    /// These paddings, and then `next`.
    pub(crate) const fn then(self, next: Self) -> Self {
        Self::new(
            self.0 & AVOID_PASTE != 0 || next.0 & AVOID_PASTE != 0,
            next.after(self.field(FROM_NONE)),
            next.after(self.field(FROM_UNSPACED)),
        )
    }

    /// This spacing, for a token with white space before it or without.
    pub(crate) const fn with_white(self, white: bool) -> Self {
        Self(if white {
            self.0 | WHITE
        } else {
            self.0 & !WHITE
        })
    }

    /// Whether the token itself has white space before it.
    pub(crate) const fn white(self) -> bool {
        self.0 & WHITE != 0
    }

    /// This spacing, for the right operand of a `##` in a replacement list.
    pub(crate) const fn right_operand(self) -> Self {
        Self(self.0 | RIGHT_OPERAND)
    }

    /// Whether the token is a right operand, which brings no padding.
    pub(crate) const fn is_right_operand(self) -> bool {
        self.0 & RIGHT_OPERAND != 0
    }

    /// What the printer does for this token.
    pub(crate) const fn output(self) -> OutputSpacing {
        let avoid_paste = self.0 & AVOID_PASTE != 0;
        OutputSpacing {
            space: match self.field(FROM_NONE) {
                | Source::Spaced if avoid_paste => true,
                | Source::Unspaced if avoid_paste => false,
                | _ => self.white(),
            },
            avoid_paste,
        }
    }

    const fn new(avoid_paste: bool, from_none: Source, from_unspaced: Source) -> Self {
        Self(
            (avoid_paste as u8)
                | ((from_none as u8) << FROM_NONE)
                | ((from_unspaced as u8) << FROM_UNSPACED),
        )
    }

    const fn field(self, at: u8) -> Source {
        match (self.0 >> at) & 3 {
            | 0 => Source::None,
            | 1 => Source::Spaced,
            | _ => Source::Unspaced,
        }
    }

    /// The printer's `source` after these paddings, from `source`.
    const fn after(self, source: Source) -> Source {
        match source {
            | Source::None => self.field(FROM_NONE),
            | Source::Spaced => Source::Spaced,
            | Source::Unspaced => self.field(FROM_UNSPACED),
        }
    }
}

/// How the printer separates a token from the one before it on its line.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct OutputSpacing {
    /// White space stands before the token: write a space.
    pub(crate) space:       bool,
    /// Write a space too if the two tokens could read as other tokens.
    pub(crate) avoid_paste: bool,
}

impl Default for TokenSpacing {
    fn default() -> Self {
        Self::NONE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The printer of `c-ppoutput.cc`, one padding at a time, from a printer
    /// that holds no source: its `avoid_paste` and `source`.
    fn printed(paddings: &[Padding]) -> (bool, Source) {
        let mut source = Source::None;
        for padding in paddings {
            let incoming = padding.source();
            if source == Source::None || (source != Source::Spaced && incoming == Source::None) {
                source = incoming;
            }
        }
        (!paddings.is_empty(), source)
    }

    #[derive(Clone, Copy)]
    enum Padding {
        Spaced,
        Unspaced,
        End,
    }

    impl Padding {
        const ALL: [Self; 3] = [Self::Spaced, Self::Unspaced, Self::End];

        fn source(self) -> Source {
            match self {
                | Self::Spaced => Source::Spaced,
                | Self::Unspaced => Source::Unspaced,
                | Self::End => Source::None,
            }
        }

        fn spacing(self) -> TokenSpacing {
            match self {
                | Self::Spaced => TokenSpacing::before(true),
                | Self::Unspaced => TokenSpacing::before(false),
                | Self::End => TokenSpacing::END,
            }
        }
    }

    /// Every run of paddings, however it is split where a replay's tokens
    /// were kept, tells the printer what the run itself would.
    #[test]
    fn composed_paddings_print_as_the_run_of_them() {
        let mut runs: Vec<Vec<Padding>> = vec![Vec::new()];
        for _ in 0..5 {
            let longer: Vec<Vec<Padding>> = runs
                .iter()
                .flat_map(|run| {
                    Padding::ALL.map(|padding| {
                        let mut run = run.clone();
                        run.push(padding);
                        run
                    })
                })
                .collect();
            runs.extend(longer);
        }
        for run in &runs {
            for split in 0..=run.len() {
                let fold = |paddings: &[Padding]| {
                    paddings
                        .iter()
                        .fold(TokenSpacing::NONE, |spacing, padding| {
                            spacing.then(padding.spacing())
                        })
                };
                let composed = fold(&run[..split]).then(fold(&run[split..]));
                let (avoid_paste, source) = printed(run);
                for white in [false, true] {
                    let output = composed.with_white(white).output();
                    assert_eq!(output.avoid_paste, avoid_paste);
                    let space = match source {
                        | Source::Spaced => true,
                        | Source::Unspaced => false,
                        | Source::None => white,
                    };
                    assert_eq!(output.space, space);
                }
            }
        }
    }

    /// The spacing rides in the padding of the tokens it is on.
    #[test]
    fn spacing_does_not_grow_tokens() {
        use super::super::PreprocessorToken;
        assert_eq!(size_of::<TokenSpacing>(), 1);
        assert_eq!(size_of::<PreprocessorToken>(), 16);
    }
}
