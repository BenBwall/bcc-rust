use super::*;
use crate::pipeline::SourceSpan;

/// A compilation of `ir` whose one error is placed on `line`.
fn compiled(ir: &str, line: usize) -> Compilation {
    Compilation {
        tokens: Some(String::new()),
        ir: Some(ir.to_owned()),
        diagnostics: vec![CompileDiagnostic {
            severity: "error".to_owned(),
            message: format!("an error on line {line}"),
            range: Some(SourceSpan {
                line,
                column: 1,
                end_line: line,
                end_column: 2,
            }),
            ..CompileDiagnostic::default()
        }],
        ..Compilation::default()
    }
}

fn key(text: &str, optimize: bool) -> CompileKey {
    (text.to_owned(), optimize)
}

#[test]
fn edits_keep_the_output_but_not_its_markers_or_links() {
    let owner = Owner::new();
    owner.set();
    let editor_text = RwSignal::new("old".to_owned());
    let optimize = RwSignal::new(false);
    let cache = RwSignal::new(vec![(key("old", false), Ok(compiled("old ir", 3)))]);
    let in_flight = RwSignal::new(None);
    let pending = RwSignal::new(false);
    let starting = RwSignal::new(false);
    let live = LiveViews::new(editor_text, optimize, cache, in_flight, pending, starting);
    let marked = |live: &LiveViews| {
        live.markers.with_untracked(|markers| {
            markers
                .iter()
                .map(|marker| marker.start_line)
                .collect::<Vec<_>>()
        })
    };
    let shown_ir = |live: &LiveViews| {
        live.shown
            .with_untracked(|shown| shown.as_ref().and_then(|shown| shown.ir.clone()))
    };
    assert_eq!(marked(&live), [3]);
    assert_eq!(shown_ir(&live).as_deref(), Some("old ir"));

    // Typing: the old output stays on screen, its squiggles and links go.
    editor_text.set("new".to_owned());
    pending.set(true);
    assert_eq!(shown_ir(&live).as_deref(), Some("old ir"));
    assert_eq!(live.compilation.get_untracked(), None);
    assert_eq!(marked(&live), Vec::<usize>::new());

    // Compiling the new text at the other level changes nothing yet.
    pending.set(false);
    in_flight.set(Some(key("new", false)));
    optimize.set(true);
    in_flight.set(Some(key("new", true)));
    assert_eq!(shown_ir(&live).as_deref(), Some("old ir"));
    assert_eq!(marked(&live), Vec::<usize>::new());

    in_flight.set(None);
    cache.update(|cache| cache.push((key("new", true), Ok(compiled("new ir", 5)))));
    assert_eq!(shown_ir(&live).as_deref(), Some("new ir"));
    assert_eq!(marked(&live), [5]);
}

#[test]
fn held_output_takes_the_current_lessons_llvm_ir() {
    let previous = Compilation {
        llvm: Some("previous lesson".to_owned()),
        ..compiled("previous ir", 1)
    };
    let current = Compilation {
        llvm: Some("this lesson".to_owned()),
        ..Compilation::default()
    };
    let held = shown(Some(&current), Some(&previous), true).expect("held");
    assert_eq!(held.ir.as_deref(), Some("previous ir"));
    assert_eq!(held.llvm.as_deref(), Some("this lesson"));
    assert_eq!(
        shown(Some(&current), Some(&previous), false),
        Some(current.clone())
    );
    let live = compiled("live", 2);
    assert_eq!(
        shown(Some(&live), Some(&previous), true),
        Some(live.clone())
    );
}

#[test]
fn edited_text_is_compiled_at_the_selected_level_first() {
    let mut cache = Compiled::new();
    assert_eq!(wanted(&cache, "x", true, true), Some(key("x", true)));
    cache.push((key("x", true), Ok(compiled("", 1))));
    assert_eq!(wanted(&cache, "x", true, true), Some(key("x", false)));
    // The abstract machine view needs only the selected level.
    assert_eq!(wanted(&cache, "x", true, false), None);
    // A request captured before an edit is no longer wanted after it.
    assert_ne!(wanted(&cache, "xy", true, true), Some(key("x", false)));
}

#[test]
fn a_bundled_lesson_compiles_only_in_the_pipeline_view() {
    let lesson = DEMOS[0].source;
    assert_eq!(wanted(&Compiled::new(), lesson, false, false), None);
    assert_eq!(
        wanted(&Compiled::new(), lesson, false, true),
        Some(key(lesson, false))
    );
}

#[test]
fn a_crash_at_either_level_stops_and_shows_for_both() {
    let crash = "bcc's in-browser compiler crashed (trap).".to_owned();
    let cache = vec![(key("x", false), Err(crash.clone()))];
    assert_eq!(wanted(&cache, "x", true, true), None);
    assert_eq!(wanted(&cache, "x", false, true), None);
    for level in [false, true] {
        assert_eq!(
            live_status(&cache, "x", level, None, false, false),
            LiveCompile::Failed(crash.clone()),
            "-O{}",
            u8::from(level) * 2
        );
    }
    // A level compiled before the crash still shows its result.
    let cache = vec![
        (key("x", true), Ok(compiled("", 1))),
        (key("x", false), Err(crash.clone())),
    ];
    assert_eq!(
        live_status(&cache, "x", true, None, false, false),
        LiveCompile::Compiled
    );
    assert_eq!(
        live_status(&cache, "y", true, None, false, false),
        LiveCompile::Idle
    );
}

#[test]
fn the_selected_level_waits_on_whatever_runs_first() {
    let cache = Compiled::new();
    let other = key("x", false);
    assert_eq!(
        live_status(&cache, "x", true, Some(&other), false, true),
        LiveCompile::Loading
    );
    assert_eq!(
        live_status(&cache, "x", true, Some(&other), false, false),
        LiveCompile::Compiling
    );
    assert_eq!(
        live_status(&cache, "x", true, None, true, false),
        LiveCompile::Compiling
    );
    assert_eq!(
        live_status(&cache, "x", true, None, false, false),
        LiveCompile::Idle
    );
}

#[test]
fn refused_programs_are_let_go_like_compiled_ones() {
    let mut cache = Compiled::new();
    let large = "int x;\n".repeat(80_000);
    for edit in 0..35 {
        let text = format!("{large}// edit {edit}\n");
        let refusal = crate::pipeline::too_large(&text).expect("too large");
        remember(&mut cache, key(&text, false), Err(refusal));
    }
    assert_eq!(cache.len(), COMPILED_LIMIT);
    // The newest is kept, and remembering a key again replaces it.
    let newest = cache.last().unwrap().0.clone();
    assert!(newest.0.ends_with("// edit 34\n"));
    remember(&mut cache, newest.clone(), Err("again".to_owned()));
    assert_eq!(cache.len(), COMPILED_LIMIT);
    assert_eq!(
        cache.iter().filter(|(known, _)| *known == newest).count(),
        1
    );
}

#[cfg(feature = "worker")]
#[test]
fn each_step_announces_what_it_did_on_the_tape() {
    use crate::{
        machine::Machine,
        protocol::{
            Answer,
            Command,
        },
    };
    let mut machine = Machine::default();
    let mut snapshot = |command| match machine.run(command) {
        | Ok(Answer::Snapshot(snapshot)) => *snapshot,
        | _ => panic!("a snapshot"),
    };
    let mut states = vec![snapshot(Command::Load(
        demos::find("path").unwrap().source.to_owned(),
    ))];
    while states.last().is_some_and(Snapshot::can_step) {
        states.push(snapshot(Command::Step));
    }
    let summaries: Vec<String> = states.iter().filter_map(tape_summary).collect();
    for expected in [
        "Tape: track side begins, indeterminate.",
        "Tape: track side now 3.",
        "Tape: track side read, 3.",
        "Tape: track n begins, 4.",
        "Tape: track area now 16.",
    ] {
        assert!(
            summaries.iter().any(|summary| summary == expected),
            "{expected} in {summaries:#?}"
        );
    }
    // A constant touches no object, so it says nothing about the tape.
    let constant = states
        .iter()
        .find(|state| state.event.kind == "constant")
        .unwrap();
    assert_eq!(tape_summary(constant), None);
}
