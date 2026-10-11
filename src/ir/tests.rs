//! Tests for the IR: building, printing and parsing the textual form, and
//! the instruction record's size.

mod builder;
mod text;

use super::*;

/// Parses `text`, panicking with the error if it does not parse.
fn parse<'ir>(arena: &'ir Bump, text: &str) -> Module<'ir> {
    parse_module(arena, text).unwrap_or_else(|error| panic!("{error}\n{text}"))
}

/// Asserts that printing the parse of `text` gives `text` back.
fn assert_round_trip(text: &str) {
    let arena = Bump::new();
    let module = parse(&arena, text);
    pretty_assertions::assert_eq!(module.to_string(), text);
}

#[cfg(target_pointer_width = "64")]
#[test]
fn records_keep_their_sizes() {
    let sizes = [
        ("InstData", size_of::<InstData>(), 16),
        ("Option<AccessTag>", size_of::<Option<AccessTag>>(), 4),
        ("Option<Value>", size_of::<Option<Value>>(), 8),
        ("Type", size_of::<Type>(), 1),
        ("Opcode", size_of::<Opcode>(), 1),
        ("InstFlags", size_of::<InstFlags>(), 1),
        ("ValueData", size_of::<ValueData>(), 16),
    ];
    let changed: Vec<_> = sizes
        .into_iter()
        .filter(|&(_, actual, expected)| actual != expected)
        .collect();
    assert!(
        changed.is_empty(),
        "record sizes changed (record, bytes, expected bytes): {changed:?}"
    );
}
