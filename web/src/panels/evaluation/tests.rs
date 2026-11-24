use super::*;
use crate::snapshot::Position;

/// Every snapshot of a run of the first lesson, "Objects, loops and calls".
#[cfg(feature = "worker")]
fn tour() -> (Vec<Snapshot>, &'static str) {
    let source = crate::demos::find("tour").unwrap().source;
    (run(source), source)
}

/// Every snapshot of a run of `source`.
#[cfg(feature = "worker")]
fn run(source: &str) -> Vec<Snapshot> {
    let mut machine = crate::machine::Machine::default();
    let mut states = vec![machine.load(source.to_owned())];
    while states.last().is_some_and(Snapshot::can_step) {
        states.push(machine.step().expect("the program steps"));
    }
    assert_eq!(states.last().unwrap().status, "finished", "{source}");
    states
}

/// Where Over, pressed at `from`, stops.
#[cfg(feature = "worker")]
fn over(states: &[Snapshot], from: usize) -> usize {
    let mut over = Over::new(states, from);
    (from + 1..states.len())
        .find_map(|cursor| over.stepped(states, cursor))
        .unwrap_or(states.len() - 1)
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
fn a_block_entry_and_a_declaration_stand_alone() {
    let (states, source) = tour();
    assert_eq!(
        group(&states, at(&states, "Declare total (int)"), source),
        None
    );
    let entry = states
        .iter()
        .position(|state| state.event.kind == "enter_block")
        .expect("main's block");
    assert_eq!(group(&states, entry, source), None);
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
    // Figure 2 names the call main() waits on at that step.
    assert_eq!(
        states[call].stack[1].waiting.as_deref(),
        Some("bump(cursor)")
    );
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
    assert_eq!(group.next.as_deref(), Some("initialize total"));
    assert_eq!(group.over, Some(OverStop::End));
    assert_eq!((group.place(), group.total()), (Some(2), 2));
    // At its last step the engine's next operation is outside it: done,
    // though the run has recorded nothing after it.
    let last = at(&states, "Initialize total = 0");
    let done = super::group(&states[..=last], last, source).unwrap();
    assert!(done.complete);
    assert_eq!(done.next, None);
    assert_eq!(done.over, Some(OverStop::Next));
    // The call is not the end of the expression that made it.
    let call = at(&states, "Call bump (frame #2)");
    assert!(
        !super::group(&states[..=call], call, source)
            .unwrap()
            .complete
    );
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

/// Two calls to a function without parameters, spelled alike.
#[cfg(feature = "worker")]
const TWO_CALLS: &str = "int f(void) { return 2; }\nint main(void) { f(); f(); return 0; }\n";

#[cfg(feature = "worker")]
#[test]
fn a_call_without_arguments_is_its_own_expression() {
    let states = run(TWO_CALLS);
    // Nothing of the expression comes before the call: no argument, no
    // destination.
    let call = at(&states, "Call f (frame #2)");
    let shown = group(&states, call, TWO_CALLS).unwrap();
    assert_eq!(shown.text, "f()");
    assert_eq!(shown.function, "main");
    assert_eq!(
        titles(&shown),
        ["call f", "Return 2 from f", "Discard the result"]
    );
    assert!(matches!(shown.parts[0], Part::Call { index, current: true, .. } if index == call));
    assert!(shown.complete);
    // Over runs through f to the expression's last step.
    let last = at(&states, "Discard the result");
    assert_eq!(over(&states, call), last);
    // Inside f, a block's entry evaluates no expression: no panel, and Over
    // finishes f's next expression without leaving f.
    let entry = call + 1;
    assert_eq!(states[entry].event.title, "Enter a block");
    assert_eq!(group(&states, entry, TWO_CALLS), None);
    assert_eq!(states[over(&states, entry)].stack[0].name, "f");
}

#[cfg(feature = "worker")]
#[test]
fn calls_spelled_alike_are_two_expressions() {
    let states = run(TWO_CALLS);
    let first = at(&states, "Call f (frame #2)");
    let second = at(&states, "Call f (frame #3)");
    let indices = |group: &Group| {
        group
            .parts
            .iter()
            .map(|part| match part {
                | Part::Step { index, .. } | Part::Call { index, .. } => *index,
            })
            .collect::<Vec<_>>()
    };
    let earlier = group(&states, first, TWO_CALLS).unwrap();
    let later = group(&states, second, TWO_CALLS).unwrap();
    assert_eq!(later.text, "f()");
    assert_eq!(titles(&later), titles(&earlier));
    // The first expression ends before the second call; the second's steps
    // start at it.
    assert!(indices(&earlier).iter().all(|&index| index < second));
    assert_eq!(indices(&later)[0], second);
    // From the first expression's last step, Over finishes the second.
    let ends = |from: usize| {
        (from..states.len())
            .find(|&index| states[index].event.title == "Discard the result")
            .unwrap()
    };
    assert_eq!(over(&states, ends(first)), ends(second));
}

#[cfg(feature = "worker")]
#[test]
fn a_call_holds_the_calls_it_makes_in_turn() {
    let source = concat!(
        "int g(int x) { return x + 1; }\n",
        "int f(int x) { int y = g(x); return y; }\n",
        "int main(void) { int r = f(1); return r; }\n",
    );
    let states = run(source);
    let outer = at(&states, "Call f (frame #2)");
    let inner = at(&states, "Call g (frame #3)");
    // In main, the call to f folds every step of f and of g.
    let shown = group(&states, outer, source).unwrap();
    assert_eq!(shown.text, "r = f(1)");
    let calls = shown
        .parts
        .iter()
        .filter(|part| matches!(part, Part::Call { .. }))
        .count();
    assert_eq!(calls, 1);
    let Some(&Part::Call { last, .. }) = shown
        .parts
        .iter()
        .find(|part| matches!(part, Part::Call { .. }))
    else {
        panic!("the call to f is folded");
    };
    assert!(last >= states[inner].step);
    assert_eq!(states[last + 1].event.title, "Return 2 from f");
    // In f, the call to g is f's own expression's.
    let nested = group(&states, inner, source).unwrap();
    assert_eq!(
        (nested.text.as_str(), nested.function.as_str()),
        ("y = g(x)", "f")
    );
    assert_eq!(nested.calling(), Some("g"));
    assert_eq!(over(&states, inner), at(&states, "Initialize y = 2"));
    assert_eq!(over(&states, outer), at(&states, "Initialize r = 2"));
    // f's return lands in main's expression.
    let returned = group(&states, at(&states, "Return 2 from f"), source).unwrap();
    assert_eq!(returned.text, "r = f(1)");
}

#[cfg(feature = "worker")]
#[test]
fn a_call_a_return_statement_makes_shows_the_whole_statement() {
    let source = "int f(void) { return 2; }\nint main(void) { return f(); }\n";
    let states = run(source);
    let call = at(&states, "Call f (frame #2)");
    let shown = group(&states, call, source).unwrap();
    // The call's first step knows only the call; the steps after it hold
    // the whole statement.
    assert_eq!(shown.text, "return f()");
    assert_eq!(titles(&shown)[..2], ["call f", "Return 2 from f"]);
    let last = match shown.parts.last().unwrap() {
        | Part::Step { index, .. } | Part::Call { index, .. } => *index,
    };
    assert!(last > call + 1);
    assert_eq!(over(&states, call), last);
}

#[cfg(feature = "worker")]
#[test]
fn a_folded_call_keeps_the_sequence_point_before_it() {
    let (states, source) = tour();
    let call = at(&states, "Call bump (frame #2)");
    assert!(states[call].event.sequence_point);
    let done = group(&states, at(&states, "Initialize next = 3"), source).unwrap();
    // After the argument is read, before the call: not after the call's
    // folded body.
    let place = done
        .parts
        .iter()
        .position(|part| matches!(part, Part::Call { .. }))
        .unwrap();
    assert!(matches!(
        done.parts[place - 1],
        Part::Step {
            sequence_point: false,
            ..
        }
    ));
    assert!(matches!(
        done.parts[place],
        Part::Call {
            sequence_point: true,
            ..
        }
    ));
    let Part::Call {
        ref function,
        first,
        last,
        ..
    } = done.parts[place]
    else {
        unreachable!()
    };
    assert!(
        call_label(function, first, last, true, false)
            .starts_with("A sequence point, then the call to bump, steps ")
    );
}

#[cfg(feature = "worker")]
#[test]
fn the_panel_marks_where_over_stops() {
    for demo in crate::demos::DEMOS
        .iter()
        .filter(|demo| demo.stage.is_none() && !demo.faults)
    {
        let states = run(demo.source);
        for cursor in 0..states.len() - 1 {
            let Some(shown) = group(&states, cursor, demo.source) else {
                continue;
            };
            let stop = over(&states, cursor);
            match shown
                .over
                .expect("Over can run from every step but the last")
            {
                | OverStop::At(index) => assert_eq!(stop, index, "{} at {cursor}", demo.key),
                | OverStop::Next => assert!(
                    group(&states, stop, demo.source)
                        .is_none_or(|next| next.text != shown.text
                            || next.parts.first() != shown.parts.first()),
                    "{} at {cursor}",
                    demo.key
                ),
                | OverStop::End => panic!("a whole run records every step"),
            }
            // Cut short at the current step, the group's last is still to come.
            let cut = group(&states[..=cursor], cursor, demo.source).unwrap();
            if !cut.complete {
                assert_eq!(cut.over, Some(OverStop::End));
                assert!(cut.next.is_some(), "{} at {cursor}", demo.key);
            }
        }
    }
}

#[cfg(feature = "worker")]
#[test]
fn over_has_no_mark_where_the_run_ended() {
    let (states, source) = tour();
    let last = states.len() - 1;
    if let Some(shown) = group(&states, last, source) {
        assert_eq!(shown.over, None);
    }
    let source = crate::demos::find("overflow").unwrap().source;
    let mut machine = crate::machine::Machine::default();
    let mut states = vec![machine.load(source.to_owned())];
    while states.last().is_some_and(Snapshot::can_step) {
        states.push(machine.step().expect("the program steps"));
    }
    let stop = group(&states, states.len() - 1, source).expect("the stop is in an expression");
    assert_eq!(stop.over, None);
}
