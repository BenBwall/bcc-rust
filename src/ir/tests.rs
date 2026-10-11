//! Tests for the IR: building function bodies and the instruction record's
//! size.

mod builder;

use super::*;

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
