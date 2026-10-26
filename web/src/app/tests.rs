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
