//! Resource header lookup maps a requested filename to embedded C header
//! text. When resource headers are enabled,
//! [`Context::set_header_search`](crate::translation_phases::Context::set_header_search)
//! places the built-in directory after the system directories and before the
//! C library's directories and the `-idirafter` ones, so ordinary include
//! handling finds these headers and a resource header such as `<limits.h>` can
//! `#include_next` the library's header of the same name.
//!
//! Read [`text`], [`HeaderSearch`], and [`DIRECTORY`].
//!
//! Files by role:
//! - Search configuration: `headers/search.rs`.
//! - Embedded resource text: the C headers under `headers/`.
//!
//! C99: §4 paragraph 6, p. 7; PDF p. 19. Implementation-defined include
//! lookup: §6.10.2 paragraphs 2-3, pp. 149-150; PDF pp. 161-162.

// Header search
mod search;

use std::path::Path;

pub(crate) use search::{
    DIRECTORY,
    HeaderSearch,
};

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
