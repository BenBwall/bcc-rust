//! Resource header lookup maps a requested filename to embedded C header
//! text. When resource headers are enabled,
//! [`Context::set_header_search`](crate::translation_phases::Context::set_header_search)
//! places the built-in directory after the system directories and before the
//! C library's directories and the `-idirafter` ones, so ordinary include
//! handling finds these headers and a resource header such as `<limits.h>` can
//! `#include_next` the library's header of the same name.
//!
//! The WebAssembly WASI target also gets bcc's bundled C library headers,
//! embedded in a second directory, [`WASI_DIRECTORY`], which takes the C
//! library's place in the search; their implementation is the WebAssembly
//! back end's runtime (`backend/wasm/runtime.rs`).
//!
//! Read [`text`], [`HeaderSearch`], and [`DIRECTORY`].
//!
//! Files by role:
//! - Search configuration: `headers/search.rs`.
//! - Embedded resource text: the C headers under `headers/`; the WASI C
//!   library's under `headers/wasi/`.
//!
//! C99: §4 paragraph 6, p. 7; PDF p. 19. Implementation-defined include
//! lookup: §6.10.2 paragraphs 2-3, pp. 149-150; PDF pp. 161-162.

// Header search
mod search;

use std::path::Path;

pub(crate) use search::{
    DIRECTORY,
    HeaderSearch,
    WASI_DIRECTORY,
};

/// Whether `directory` is one of the embedded header directories.
pub(crate) fn is_embedded(directory: &Path) -> bool {
    directory == Path::new(DIRECTORY) || directory == Path::new(WASI_DIRECTORY)
}

/// The text of the header `name` in the embedded `directory`, if it has
/// one.
pub(crate) fn embedded(directory: &Path, name: &Path) -> Option<&'static str> {
    if directory == Path::new(DIRECTORY) {
        text(name)
    } else if directory == Path::new(WASI_DIRECTORY) {
        wasi_text(name)
    } else {
        None
    }
}

/// Whether the embedded `directory` holds the header `name` for a
/// compilation as `identity`: every embedded header, except that the
/// resource directory has no `<stdbit.h>` for the Clang identities, as
/// Clang ships none, so their lookup finds only a C library's (bcc's WASI
/// C library has one).
pub(crate) fn provides(
    directory: &Path,
    name: &Path,
    identity: crate::configuration::CompilerIdentity,
) -> bool {
    use crate::configuration::CompilerIdentity;
    embedded(directory, name).is_some()
        && !(directory == Path::new(DIRECTORY)
            && name == Path::new("stdbit.h")
            && matches!(
                identity,
                CompilerIdentity::Clang | CompilerIdentity::ClangCl
            ))
}

/// The text of an embedded header's path, `DIRECTORY/name` or
/// `WASI_DIRECTORY/name`.
pub(crate) fn embedded_path(path: &Path) -> Option<&'static str> {
    if let Ok(name) = path.strip_prefix(DIRECTORY) {
        text(name)
    } else if let Ok(name) = path.strip_prefix(WASI_DIRECTORY) {
        wasi_text(name)
    } else {
        None
    }
}

/// The bundled C library headers of the WASI target. Each names its own
/// implementation in the WebAssembly back end's runtime.
/// C99: §7.1.2, pp. 165-166; PDF pp. 177-178 (standard headers).
pub(crate) fn wasi_text(name: &Path) -> Option<&'static str> {
    Some(match name.to_str()? {
        | "assert.h" => include_str!("headers/wasi/assert.h"),
        | "ctype.h" => include_str!("headers/wasi/ctype.h"),
        | "errno.h" => include_str!("headers/wasi/errno.h"),
        | "inttypes.h" => include_str!("headers/wasi/inttypes.h"),
        | "math.h" => include_str!("headers/wasi/math.h"),
        | "setjmp.h" => include_str!("headers/wasi/setjmp.h"),
        // The resource header, which is also the C library's here.
        | "stdbit.h" => include_str!("headers/stdbit.h"),
        | "stdio.h" => include_str!("headers/wasi/stdio.h"),
        | "stdlib.h" => include_str!("headers/wasi/stdlib.h"),
        | "string.h" => include_str!("headers/wasi/string.h"),
        | "strings.h" => include_str!("headers/wasi/strings.h"),
        | "time.h" => include_str!("headers/wasi/time.h"),
        | "unistd.h" => include_str!("headers/wasi/unistd.h"),
        | _ => return None,
    })
}

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
        | "stdbit.h" => include_str!("headers/stdbit.h"),
        | "tgmath.h" => include_str!("headers/tgmath.h"),
        | "mmintrin.h" => include_str!("headers/mmintrin.h"),
        | "xmmintrin.h" => include_str!("headers/xmmintrin.h"),
        | "emmintrin.h" => include_str!("headers/emmintrin.h"),
        | "pmmintrin.h" => include_str!("headers/pmmintrin.h"),
        | "tmmintrin.h" => include_str!("headers/tmmintrin.h"),
        | "smmintrin.h" => include_str!("headers/smmintrin.h"),
        | "nmmintrin.h" => include_str!("headers/nmmintrin.h"),
        | "wmmintrin.h" => include_str!("headers/wmmintrin.h"),
        | "mm3dnow.h" => include_str!("headers/mm3dnow.h"),
        | "avxintrin.h" => include_str!("headers/avxintrin.h"),
        | "avx2intrin.h" => include_str!("headers/avx2intrin.h"),
        | "popcntintrin.h" => include_str!("headers/popcntintrin.h"),
        | "crc32intrin.h" => include_str!("headers/crc32intrin.h"),
        | "cpuid.h" => include_str!("headers/cpuid.h"),
        | "immintrin.h" => include_str!("headers/immintrin.h"),
        | "x86intrin.h" => include_str!("headers/x86intrin.h"),
        | "__wmmintrin_aes.h" => include_str!("headers/__wmmintrin_aes.h"),
        | "__wmmintrin_pclmul.h" => include_str!("headers/__wmmintrin_pclmul.h"),
        | "prfchwintrin.h" => include_str!("headers/prfchwintrin.h"),
        | "intrin.h" => include_str!("headers/intrin.h"),
        | "intrin0.h" => include_str!("headers/intrin0.h"),
        | "adcintrin.h" => include_str!("headers/adcintrin.h"),
        | "__bcc_msvc_intrinsics.h" => include_str!("headers/__bcc_msvc_intrinsics.h"),
        | _ => return None,
    })
}
