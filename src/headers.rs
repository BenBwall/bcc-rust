//! Embedded resource directory and the header search configuration.
//! C99: §4p6, p. 7; PDF p. 19; implementation-defined include lookup
//! §6.10.2p2-3, pp. 149-150; PDF pp. 161-162.

use std::path::Path;

pub(crate) const DIRECTORY: &str = "<built-in>";

/// Header names are exact, independent of host filesystem case folding.
pub(crate) fn text(name: &Path) -> Option<&'static str> {
    Some(match name.to_str()? {
        | "float.h" => include_str!("headers/float.h"),
        | "iso646.h" => include_str!("headers/iso646.h"),
        | "limits.h" => include_str!("headers/limits.h"),
        | "mm_malloc.h" => include_str!("headers/mm_malloc.h"),
        | "stdarg.h" => include_str!("headers/stdarg.h"),
        | "stdbool.h" => include_str!("headers/stdbool.h"),
        | "stddef.h" => include_str!("headers/stddef.h"),
        | "stdint.h" => include_str!("headers/stdint.h"),
        | "stdalign.h" => include_str!("headers/stdalign.h"),
        | "stdnoreturn.h" => include_str!("headers/stdnoreturn.h"),
        | "stdatomic.h" => include_str!("headers/stdatomic.h"),
        | "bits/floatn.h" => include_str!("headers/bits/floatn.h"),
        | _ => return None,
    })
}

/// The places a header is searched for, in Clang's order (C99 §6.10.2p2-3
/// leave them implementation-defined):
///
/// 1. for a `"…"` name only, the including file's directory and then
///    [`quote`](Self::quote);
/// 2. [`angled`](Self::angled);
/// 3. [`system`](Self::system);
/// 4. the embedded resource directory, when [`resource`](Self::resource) is
///    set;
/// 5. [`after`](Self::after).
///
/// The resource directory precedes the C library, so a resource header such
/// as `<limits.h>` can `#include_next` the library's header of the same name.
/// Every place from `system` on is a system directory, as in GCC: a header
/// found there is a system header.
///
/// C99: implementation-defined places, §6.10.2 paragraphs 2-3, pp. 149-150;
/// PDF pp. 161-162.
#[derive(Debug, Clone, Copy)]
pub(crate) struct HeaderSearch<'a> {
    /// `-iquote` directories, searched only for `"…"` names.
    pub(crate) quote:    &'a [&'a Path],
    /// `-I` and then `CPATH` directories.
    pub(crate) angled:   &'a [&'a Path],
    /// `-isystem` and then `C_INCLUDE_PATH` directories, the first system
    /// directories.
    pub(crate) system:   &'a [&'a Path],
    /// Whether the embedded resource directory follows `system`; cleared by
    /// `-nostdinc` and `-nobuiltininc`.
    pub(crate) resource: bool,
    /// The C library's directories (from `--sysroot`, unless `-nostdinc` or
    /// `-nostdlibinc` removed them), then the `-idirafter` directories.
    pub(crate) after:    &'a [&'a Path],
}

impl Default for HeaderSearch<'_> {
    /// No configured directories: a lookup searches only the including
    /// file's directory and the resource directory.
    fn default() -> Self {
        HeaderSearch {
            quote:    &[],
            angled:   &[],
            system:   &[],
            resource: true,
            after:    &[],
        }
    }
}
