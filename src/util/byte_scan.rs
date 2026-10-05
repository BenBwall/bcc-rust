//! Byte-class scans for the lexer.
//!
//! Every scan answers "how many leading bytes belong to a class?" or "where
//! is the first byte of a class?". With the nightly-only `portable-simd`
//! feature, scans classify [`LANES`] bytes per step with `std::simd`, so one
//! comparison chain and one bitmask replace a branch per byte. Without it they
//! fall back to scalar loops with identical results, keeping the pinned
//! stable toolchain buildable.

#[cfg(feature = "portable-simd")]
use std::simd::{
    Mask,
    Simd,
    cmp::{
        SimdPartialEq,
        SimdPartialOrd,
    },
};

/// Bytes classified per vector step.
#[cfg(feature = "portable-simd")]
const LANES: usize = 32;

#[cfg(feature = "portable-simd")]
type Lanes = Simd<u8, LANES>;

#[cfg(feature = "portable-simd")]
type LaneMask = Mask<i8, LANES>;

/// A set of bytes that can be tested one byte or one vector at a time. Both
/// tests must agree.
trait ByteClass {
    fn contains(byte: u8) -> bool;
    #[cfg(feature = "portable-simd")]
    fn contains_lanes(bytes: Lanes) -> LaneMask;
}

#[cfg(feature = "portable-simd")]
#[inline(always)]
fn splat(byte: u8) -> Lanes {
    Lanes::splat(byte)
}

#[cfg(feature = "portable-simd")]
#[inline(always)]
fn in_range(bytes: Lanes, low: u8, high: u8) -> LaneMask {
    bytes.simd_ge(splat(low)) & bytes.simd_le(splat(high))
}

/// Declares a [`ByteClass`] from a scalar pattern and the equivalent lane
/// expression.
macro_rules! byte_class {
    ($(#[$attribute:meta])* $name:ident, $pattern:pat, | $lanes:ident | $vector:expr) => {
        $(#[$attribute])*
        struct $name;

        impl ByteClass for $name {
            #[inline(always)]
            fn contains(byte: u8) -> bool {
                matches!(byte, $pattern)
            }

            #[cfg(feature = "portable-simd")]
            #[inline(always)]
            fn contains_lanes($lanes: Lanes) -> LaneMask {
                $vector
            }
        }
    };
}

byte_class!(
    /// ASCII identifier characters: `[A-Za-z0-9_]`.
    Identifier,
    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_',
    |bytes| {
        let folded = bytes | splat(0x20);
        in_range(folded, b'a', b'z') | in_range(bytes, b'0', b'9') | bytes.simd_eq(splat(b'_'))
    }
);

byte_class!(
    /// ASCII preprocessing-number body characters other than exponent
    /// signs: `[A-Za-z0-9_.]`.
    NumberBody,
    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.',
    |bytes| {
        let folded = bytes | splat(0x20);
        in_range(folded, b'a', b'z')
            | in_range(bytes, b'0', b'9')
            | bytes.simd_eq(splat(b'_'))
            | bytes.simd_eq(splat(b'.'))
    }
);

byte_class!(
    /// Whitespace that does not end a line: space, tab, vertical tab, and
    /// form feed.
    HorizontalSpace,
    b' ' | b'\t' | b'\x0b' | b'\x0c',
    |bytes| bytes.simd_eq(splat(b' '))
        | (in_range(bytes, b'\t', b'\x0c') & bytes.simd_ne(splat(b'\n')))
);

byte_class!(
    /// Bytes that translation phases 1 and 2 may rewrite: the start of a
    /// trigraph, of a line splice, or of a carriage-return line ending.
    Phase2Special,
    b'\\' | b'?' | b'\r',
    |bytes| bytes.simd_eq(splat(b'\\')) | bytes.simd_eq(splat(b'?')) | bytes.simd_eq(splat(b'\r'))
);

byte_class!(
    /// Bytes that end a string or character literal body in spliced text.
    LiteralBreak,
    b'\\' | b'\n' | b'"' | b'\'',
    |bytes| {
        bytes.simd_eq(splat(b'\\'))
            | bytes.simd_eq(splat(b'\n'))
            | bytes.simd_eq(splat(b'"'))
            | bytes.simd_eq(splat(b'\''))
    }
);

byte_class!(
    /// Bytes that may end a block comment or a line in spliced text.
    BlockCommentBreak,
    b'*' | b'\n',
    |bytes| bytes.simd_eq(splat(b'*')) | bytes.simd_eq(splat(b'\n'))
);

byte_class!(
    /// Line feed.
    LineFeed,
    b'\n',
    |bytes| bytes.simd_eq(splat(b'\n'))
);

byte_class!(
    /// UTF-8 continuation bytes, which do not start a character.
    Continuation,
    0x80..=0xBF,
    |bytes| (bytes & splat(0xC0)).simd_eq(splat(0x80))
);

/// Returns the index of the first byte for which `class` is `wanted`, or the
/// length of `bytes` when there is none.
#[inline(always)]
fn find<C: ByteClass, const WANTED: bool>(bytes: &[u8]) -> usize {
    #[cfg(feature = "portable-simd")]
    {
        let (chunks, remainder) = bytes.as_chunks::<LANES>();
        for (index, chunk) in chunks.iter().enumerate() {
            let mask = C::contains_lanes(Lanes::from_array(*chunk));
            let bits = if WANTED { mask } else { !mask }.to_bitmask();
            if bits != 0 {
                return index * LANES + bits.trailing_zeros() as usize;
            }
        }
        chunks.len() * LANES
            + remainder
                .iter()
                .position(|&byte| C::contains(byte) == WANTED)
                .unwrap_or(remainder.len())
    }
    #[cfg(not(feature = "portable-simd"))]
    {
        bytes
            .iter()
            .position(|&byte| C::contains(byte) == WANTED)
            .unwrap_or(bytes.len())
    }
}

/// Counts the bytes of `bytes` in `class`.
#[inline(always)]
fn count<C: ByteClass>(bytes: &[u8]) -> usize {
    #[cfg(feature = "portable-simd")]
    {
        let (chunks, remainder) = bytes.as_chunks::<LANES>();
        chunks
            .iter()
            .map(|chunk| {
                C::contains_lanes(Lanes::from_array(*chunk))
                    .to_bitmask()
                    .count_ones() as usize
            })
            .sum::<usize>()
            + remainder.iter().filter(|&&byte| C::contains(byte)).count()
    }
    #[cfg(not(feature = "portable-simd"))]
    {
        bytes.iter().filter(|&&byte| C::contains(byte)).count()
    }
}

/// Length of the leading run of ASCII identifier characters.
#[inline(always)]
pub(crate) fn identifier_run(bytes: &[u8]) -> usize {
    find::<Identifier, false>(bytes)
}

/// Length of the leading run of ASCII preprocessing-number characters,
/// excluding exponent signs.
#[inline(always)]
pub(crate) fn number_run(bytes: &[u8]) -> usize {
    find::<NumberBody, false>(bytes)
}

/// Length of the leading run of space, tab, vertical tab, and form feed.
#[inline(always)]
pub(crate) fn horizontal_space_run(bytes: &[u8]) -> usize {
    find::<HorizontalSpace, false>(bytes)
}

/// Index of the first byte that translation phases 1-2 may rewrite.
#[inline(always)]
pub(crate) fn find_phase2_special(bytes: &[u8]) -> usize {
    find::<Phase2Special, true>(bytes)
}

/// Length of the leading run of a spliced literal body: no quote, escape,
/// or line feed.
#[inline(always)]
pub(crate) fn literal_run(bytes: &[u8]) -> usize {
    find::<LiteralBreak, true>(bytes)
}

/// Length of the leading run of spliced block-comment text before `*` or a
/// line feed.
#[inline(always)]
pub(crate) fn block_comment_run(bytes: &[u8]) -> usize {
    find::<BlockCommentBreak, true>(bytes)
}

/// Index of the first line feed.
#[inline(always)]
pub(crate) fn find_line_feed(bytes: &[u8]) -> usize {
    find::<LineFeed, true>(bytes)
}

/// Number of line feeds.
#[inline(always)]
pub(crate) fn count_line_feeds(bytes: &[u8]) -> usize {
    count::<LineFeed>(bytes)
}

/// Number of characters in valid UTF-8 `bytes`.
#[inline(always)]
pub(crate) fn count_chars(bytes: &[u8]) -> usize {
    if bytes.is_ascii() {
        bytes.len()
    } else {
        bytes.len() - count::<Continuation>(bytes)
    }
}

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    reason = "Tests build inputs and expected values with std types; the arena rule covers the \
              compiler, not its tests."
)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn scalar_find(bytes: &[u8], wanted: bool, class: impl Fn(u8) -> bool) -> usize {
        bytes
            .iter()
            .position(|&byte| class(byte) == wanted)
            .unwrap_or(bytes.len())
    }

    proptest! {
        #[test]
        fn vector_scans_agree_with_byte_tests(
            bytes in proptest::collection::vec(
                prop_oneof![
                    Just(b'a'), Just(b'Z'), Just(b'0'), Just(b'_'), Just(b'.'), Just(b' '),
                    Just(b'\t'), Just(b'\x0b'), Just(b'\x0c'), Just(b'\n'), Just(b'\r'),
                    Just(b'\\'), Just(b'?'), Just(b'*'), Just(b'"'), Just(b'\''), Just(b'+'),
                    Just(0xc3), Just(0xa9), any::<u8>(),
                ],
                0..100,
            )
        ) {
            prop_assert_eq!(
                identifier_run(&bytes),
                scalar_find(&bytes, false, |b| b.is_ascii_alphanumeric() || b == b'_')
            );
            prop_assert_eq!(
                number_run(&bytes),
                scalar_find(&bytes, false, |b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.')
            );
            prop_assert_eq!(
                horizontal_space_run(&bytes),
                scalar_find(&bytes, false, |b| matches!(b, b' ' | b'\t' | b'\x0b' | b'\x0c'))
            );
            prop_assert_eq!(
                find_phase2_special(&bytes),
                scalar_find(&bytes, true, |b| matches!(b, b'\\' | b'?' | b'\r'))
            );
            prop_assert_eq!(
                literal_run(&bytes),
                scalar_find(&bytes, true, |b| matches!(b, b'\\' | b'\n' | b'"' | b'\''))
            );
            prop_assert_eq!(
                block_comment_run(&bytes),
                scalar_find(&bytes, true, |b| matches!(b, b'*' | b'\n'))
            );
            prop_assert_eq!(
                count_line_feeds(&bytes),
                bytes.iter().map(|&b| usize::from(b == b'\n')).sum::<usize>()
            );
        }
    }

    #[test]
    fn character_counts_skip_continuation_bytes() {
        let text = "aé€😀b".repeat(20);
        assert_eq!(
            count_chars(text.as_bytes()),
            text.chars().count(),
            "UTF-8 character counts must match `str::chars`"
        );
    }
}
