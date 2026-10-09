//! Embedded freestanding resource directory.
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
        | "stdarg.h" => include_str!("headers/stdarg.h"),
        | "stdbool.h" => include_str!("headers/stdbool.h"),
        | "stddef.h" => include_str!("headers/stddef.h"),
        | "stdint.h" => include_str!("headers/stdint.h"),
        | "stdalign.h" => include_str!("headers/stdalign.h"),
        | "stdnoreturn.h" => include_str!("headers/stdnoreturn.h"),
        | _ => return None,
    })
}
