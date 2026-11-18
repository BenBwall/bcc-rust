use super::*;
use crate::snapshot::Position;

/// Every snapshot of a run of the first lesson, "Objects, loops and calls".
#[cfg(feature = "worker")]
fn tour() -> (Vec<Snapshot>, &'static str) {
    let source = crate::demos::find("tour").unwrap().source;
    let mut machine = crate::machine::Machine::default();
    let mut states = vec![machine.load(source.to_owned())];
    while states.last().is_some_and(Snapshot::can_step) {
        states.push(machine.step().expect("the lesson steps"));
    }
    (states, source)
}

#[cfg(feature = "worker")]
fn at(states: &[Snapshot], title: &str) -> usize {
    states
        .iter()
        .position(|state| state.event.title.replace('`', "") == title)
        .unwrap_or_else(|| panic!("no step titled {title}"))
}

#[cfg(feature = "worker")]
fn titles(group: &Group) -> Vec<String> {
    group
        .parts
        .iter()
        .map(|part| match part {
            | Part::Step { title, .. } => title.clone(),
            | Part::Call { function, .. } => format!("call {function}"),
        })
        .collect()
}

#[cfg(feature = "worker")]
#[test]
fn an_initialization_is_its_steps_up_to_the_sequence_point() {
    let (states, source) = tour();
    let cursor = at(&states, "Initialize total = 0");
    let shown = group(&states, cursor, source).unwrap();
    assert_eq!(shown.text, "total = 0");
    assert_eq!(shown.function, "main");
    assert_eq!(
        titles(&shown),
        [
            "total receives its initial value",
            "Evaluate the constant 0",
            "Initialize total = 0"
        ]
    );
    assert!(shown.complete);
    assert_eq!((shown.place(), shown.total()), (Some(3), 3));
    assert!(matches!(
        shown.parts.last(),
        Some(Part::Step {
            sequence_point: true,
            current: true,
            ..
        })
    ));
    // Back at its first step, the later two are after the one shown.
    let first = group(&states, cursor - 2, source).unwrap();
    assert_eq!(first.place(), Some(1));
    assert!(matches!(first.parts[2], Part::Step { after: true, .. }));
}

#[cfg(feature = "worker")]
#[test]
fn a_declaration_creating_its_object_stands_alone() {
    let (states, source) = tour();
    assert_eq!(
        group(&states, at(&states, "Create total (int)"), source),
        None
    );
    assert_eq!(group(&states, at(&states, "Enter a block"), source), None);
}

#[cfg(feature = "worker")]
#[test]
fn a_call_folds_into_the_expression_that_made_it() {
    let (states, source) = tour();
    let call = at(&states, "Call bump (frame #2)");
    let group = group(&states, call, source).unwrap();
    assert_eq!(group.text, "next = bump(cursor)");
    assert_eq!(
        titles(&group),
        [
            "next receives its initial value",
            "cursor names an object to read",
            "Read cursor → &values[0]",
            "call bump",
            "Return 3 from bump",
            "Initialize next = 3"
        ]
    );
    assert_eq!(group.place(), None);
    assert_eq!(group.calling(), Some("bump"));
    let Part::Call {
        index, first, last, ..
    } = group.parts[3]
    else {
        panic!("the fourth part is the call");
    };
    assert_eq!(index, call);
    assert_eq!(first, states[call].step);
    // The call runs up to the step before its return, which is main's.
    assert_eq!(states[last + 1].event.title, "Return 3 from bump");
    // A step inside bump belongs to bump's own expression.
    let inside = group_text(&states, at(&states, "Initialize before = 2"), source);
    assert_eq!(inside.as_deref(), Some("before = *value"));
}

#[cfg(feature = "worker")]
fn group_text(states: &[Snapshot], cursor: usize, source: &str) -> Option<String> {
    group(states, cursor, source).map(|group| group.text)
}

#[cfg(feature = "worker")]
#[test]
fn a_run_cut_short_says_what_comes_next() {
    let (states, source) = tour();
    let cursor = at(&states, "Evaluate the constant 0");
    let group = group(&states[..=cursor], cursor, source).unwrap();
    assert!(!group.complete);
    assert_eq!(group.next.as_deref(), Some("store into total"));
    assert_eq!((group.place(), group.total()), (Some(2), 2));
}

#[test]
fn source_text_joins_lines_and_cuts_long_text() {
    let range = |start: (usize, usize), end: (usize, usize)| Range {
        start: Position {
            line:   start.0,
            column: start.1,
        },
        end:   Position {
            line:   end.0,
            column: end.1,
        },
    };
    let source = "int x = a +\n    b;\n";
    assert_eq!(source_text(source, range((1, 5), (2, 6))), "x = a + b");
    let long = format!("int y = {};", "1 + ".repeat(20));
    assert_eq!(
        source_text(&long, range((1, 5), (1, 80))).chars().count(),
        48
    );
    assert!(source_text(&long, range((1, 5), (1, 80))).ends_with('…'));
}
