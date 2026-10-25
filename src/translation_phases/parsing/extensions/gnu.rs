//! Syntax of the GNU extensions to translation phase 7, parsed by
//! `grammar/gnu.rs`.
//!
//! C99: these extend expression, statement and declarator grammar (§6.5,
//! pp. 67-94; PDF pp. 79-106; §6.8, pp. 131-139; PDF pp. 143-151;
//! §6.7.5, p. 114; PDF p. 126). GCC's C Extensions and Extended Asm
//! manuals specify the vendor grammar. Types, constraints, assembly meaning,
//! builtin evaluation and label resolution belong to later analysis.

use crate::{
    translation_phases::{
        SourceVectors,
        parsing::{
            extensions::modern::SyntaxOperand,
            syntax::{
                Expression,
                Identifier,
            },
        },
        preprocessing::{
            KeywordTokenType,
            Token,
        },
    },
    util::arena_list::ArenaList,
};

/// GNU assembly grammar: `asm qualifiers ( template : outputs : inputs :
/// clobbers : labels )`, or a declarator's `asm ( template )` label. The
/// template, constraint and clobber string literals keep their provenance;
/// C operands are parsed syntax children. `sections` counts the colons.
/// C99: extension to §6.8, p. 131; PDF p. 143 and §6.7.5, p. 114; PDF p. 126.
/// GNU extension: GCC manual, "Extended Asm".
/// <https://gcc.gnu.org/onlinedocs/gcc/Extended-Asm.html>
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct Asm<'tu> {
    pub(crate) qualifiers:     AsmQualifiers,
    /// `None` when the template literal is missing.
    pub(crate) template:       Option<Token>,
    pub(crate) operands:       ArenaList<'tu, AsmOperand<'tu>>,
    pub(crate) clobbers:       ArenaList<'tu, Token>,
    pub(crate) labels:         ArenaList<'tu, Identifier>,
    pub(crate) sections:       u8,
    pub(crate) source_vectors: SourceVectors,
    pub(crate) recovered:      bool,
}

/// The qualifiers written before a GNU assembly statement's `(`.
/// GNU extension: GCC manual, "Extended Asm".
/// <https://gcc.gnu.org/onlinedocs/gcc/Extended-Asm.html>
#[derive(Debug, PartialEq, Eq, Clone, Copy, Default)]
pub(crate) struct AsmQualifiers {
    pub(crate) volatile: bool,
    pub(crate) inline:   bool,
    pub(crate) goto:     bool,
}

/// One GNU output/input operand with an optional symbolic name.
/// GNU extension: GCC manual, "Extended Asm".
/// <https://gcc.gnu.org/onlinedocs/gcc/Extended-Asm.html>
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct AsmOperand<'tu> {
    pub(crate) name:       Option<Identifier>,
    pub(crate) constraint: Token,
    pub(crate) expression: &'tu Expression<'tu>,
    pub(crate) output:     bool,
}

/// Typed GNU builtin operands. Offset member paths use their own namespace.
/// C99: extension to primary expressions §6.5.1, p. 69; PDF p. 81.
/// GNU extension: GCC manual, "Other Builtins".
/// <https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html>
/// GNU extension: GCC manual, "Offsetof".
/// <https://gcc.gnu.org/onlinedocs/gcc/Offsetof.html>
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) struct Builtin<'tu> {
    pub(crate) keyword:        KeywordTokenType,
    pub(crate) operands:       ArenaList<'tu, SyntaxOperand<'tu>>,
    pub(crate) members:        ArenaList<'tu, OffsetMember<'tu>>,
    pub(crate) source_vectors: SourceVectors,
    pub(crate) recovered:      bool,
}

/// A field or array-index suffix in an offsetof member designator.
/// GNU extension: GCC manual, "Offsetof".
/// <https://gcc.gnu.org/onlinedocs/gcc/Offsetof.html>
/// C99: §7.17 paragraph 3, p. 254; PDF p. 266.
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum OffsetMember<'tu> {
    Field(Identifier),
    Index(&'tu Expression<'tu>),
}
