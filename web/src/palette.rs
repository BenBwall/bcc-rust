//! Colour coding shared by every view: each variable name gets one of eight
//! hues (`--obj-0` … `--obj-7` in `style.css`), used for its object in the
//! memory diagram, its identifiers in the source, and its leaves in the
//! expression tree.

/// The palette slot for the object named `name`. A stable hash keeps a name's
/// colour the same across steps, demos and reloads.
pub fn slot(name: &str) -> u8 {
    let mut hash = 0x811C_9DC5_u32;
    for byte in name.bytes() {
        hash = (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193);
    }
    (hash % 8) as u8
}

/// The CSS class that selects `name`'s colour.
pub fn class(name: &str) -> String {
    format!("obj-c{}", slot(name))
}

/// Byte ranges of the identifiers in C `source`, skipping comments, literals
/// and keywords.
pub fn identifiers(source: &str) -> Vec<(usize, usize)> {
    let bytes = source.as_bytes();
    let mut found = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let rest = &bytes[index..];
        if rest.starts_with(b"//") {
            index += rest
                .iter()
                .position(|&byte| byte == b'\n')
                .unwrap_or(rest.len());
        } else if rest.starts_with(b"/*") {
            index += rest
                .windows(2)
                .skip(2)
                .position(|pair| pair == b"*/")
                .map_or(rest.len(), |end| end + 4);
        } else if matches!(rest[0], b'"' | b'\'') {
            let quote = rest[0];
            let mut end = 1;
            while end < rest.len() && rest[end] != quote && rest[end] != b'\n' {
                end += if rest[end] == b'\\' { 2 } else { 1 };
            }
            index += end + 1;
        } else if rest[0].is_ascii_alphabetic() || rest[0] == b'_' {
            let length = rest
                .iter()
                .position(|&byte| !(byte.is_ascii_alphanumeric() || byte == b'_'))
                .unwrap_or(rest.len());
            found.push((index, index + length));
            index += length;
        } else if rest[0].is_ascii_digit() {
            index += rest
                .iter()
                .position(|&byte| !(byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'.'))
                .unwrap_or(rest.len());
        } else {
            index += 1;
        }
    }
    found
}

/// The 1-based line and UTF-16 column of byte `offset` in `source`.
pub fn position(source: &str, offset: usize) -> (usize, usize) {
    let prefix = &source[..offset.min(source.len())];
    let start = prefix.rfind('\n').map_or(0, |newline| newline + 1);
    (
        prefix.matches('\n').count() + 1,
        prefix[start..].encode_utf16().count() + 1,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_identifiers_outside_comments_and_literals() {
        let source = "int x = y; // z\n/* w */ \"q\" x1 12u";
        let names: Vec<&str> = identifiers(source)
            .into_iter()
            .map(|(start, end)| &source[start..end])
            .collect();
        assert_eq!(names, ["int", "x", "y", "x1"]);
        assert_eq!(position(source, source.find("x1").unwrap()), (2, 13));
    }

    #[test]
    fn slots_are_stable_and_in_range() {
        assert_eq!(slot("total"), slot("total"));
        assert!(
            ["a", "b", "total", "values", "cursor"]
                .iter()
                .all(|name| slot(name) < 8)
        );
    }
}
