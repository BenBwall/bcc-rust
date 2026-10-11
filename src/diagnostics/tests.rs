use std::path::Path;

use super::{
    formatting::MAX_QUOTED_SPELLING,
    *,
};
use crate::translation_phases::{
    ErrorSeverity,
    SourceVectors,
};

fn with_context<R>(text: &str, inspect: impl FnOnce(&mut Context<'_>, u32) -> R) -> R {
    let tu = Bump::new();
    let mut context = Context::new(&tu);
    let file = context.intern_source_file(Path::new("example.c"));
    context.record_source_text(file, text);
    inspect(&mut context, file)
}

fn span(context: &mut Context<'_>, file: u32, text: &str, needle: &str) -> SourceVectors {
    let index = text.find(needle).expect("needle occurs in text");
    let line = text[..index].matches('\n').count() + 1;
    let column = index - text[..index].rfind('\n').map_or(0, |newline| newline + 1) + 1;
    context.create_source_vectors(
        crate::translation_phases::SourcePosition {
            index,
            line: u32::try_from(line).unwrap(),
            column: u32::try_from(column).unwrap(),
        },
        file,
        needle.len(),
    )
}

#[test]
fn renders_primary_and_secondary_labels_with_notes() {
    let text = "int x \"abc\";\n";
    with_context(text, |context, file| {
        let primary = span(context, file, text, "\"abc\"");
        let secondary = span(context, file, text, "x");
        let arena = Bump::new();
        let diagnostic = Explanation::new(&arena, "expected `;`, found string literal")
            .label("expected `;`")
            .note("a declaration ends with `;`")
            .help("add `;` after `x`")
            .at(ErrorSeverity::Error, primary)
            .secondary(secondary, "declarator");

        let mut renderer = Renderer::new(ColorChoice::Plain);
        let actual = renderer.render_text(&diagnostic, context).to_owned();

        assert_eq!(
            actual,
            "error: expected `;`, found string literal\n --> example.c:1:7\n  |\n1 | int x \
             \"abc\";\n  |     - ^^^^^ expected `;`\n  |     |\n  |     declarator\n  |\n  = \
             note: a declaration ends with `;`\n  = help: add `;` after `x`\n\n"
        );
        assert_eq!(renderer.render_text(&diagnostic, context), actual);
    });
}

#[test]
fn reused_renderer_uses_the_current_contexts_line_starts() {
    let mut renderer = Renderer::new(ColorChoice::Plain);
    with_context("first line", |context, file| {
        let source = span(context, file, "first line", "first");
        let arena = Bump::new();
        let diagnostic = Explanation::new(&arena, "first").at(ErrorSeverity::Error, source);
        _ = renderer.render_text(&diagnostic, context);
    });
    with_context("x\ny\n", |context, file| {
        let source = span(context, file, "x\ny\n", "y");
        let arena = Bump::new();
        let diagnostic = Explanation::new(&arena, "second").at(ErrorSeverity::Error, source);
        let output = renderer.render_text(&diagnostic, context);
        assert!(output.contains("2 | y"), "{output}");
    });
}

#[test]
fn per_diagnostic_scratch_does_not_scale_with_source_line_count() {
    let scratch_for = |lines| {
        with_context(&"int x;\n".repeat(lines), |context, file| {
            let source = span(context, file, "int x;\n", "x");
            let arena = Bump::new();
            let diagnostic = Explanation::new(&arena, "example").at(ErrorSeverity::Error, source);
            let mut renderer = Renderer::new(ColorChoice::Plain);
            let first = renderer.render_text(&diagnostic, context).to_owned();
            let used = context.tu_arena().used();
            for _ in 0..16 {
                assert_eq!(renderer.render_text(&diagnostic, context), first);
            }
            assert_eq!(context.tu_arena().used(), used);
            renderer.scratch.high_water()
        })
    };
    assert_eq!(scratch_for(32), scratch_for(8_192));
}

#[test]
fn replaced_source_text_uses_its_own_line_index() {
    with_context("first line", |context, file| {
        let mut renderer = Renderer::new(ColorChoice::Plain);
        let arena = Bump::new();
        let source = span(context, file, "first line", "first");
        let diagnostic = Explanation::new(&arena, "first").at(ErrorSeverity::Error, source);
        _ = renderer.render_text(&diagnostic, context);

        let text = "x\r\ny\rz\n";
        context.record_source_text(file, text);
        let source = context.create_source_vectors(
            crate::translation_phases::SourcePosition {
                index:  5,
                line:   3,
                column: 1,
            },
            file,
            1,
        );
        let diagnostic = Explanation::new(&arena, "second").at(ErrorSeverity::Error, source);
        let output = renderer.render_text(&diagnostic, context);
        assert_eq!(
            output,
            "error: second\n --> example.c:3:1\n  |\n3 | z\n  | ^\n\n"
        );
    });
}

#[test]
fn zero_length_ranges_point_after_the_line() {
    let text = "int x";
    with_context(text, |context, file| {
        let end = context.create_source_vectors(
            crate::translation_phases::SourcePosition {
                index:  5,
                line:   1,
                column: 6,
            },
            file,
            0,
        );
        let arena = Bump::new();
        let diagnostic =
            Explanation::new(&arena, "no newline at end of file").at(ErrorSeverity::Warning, end);

        let mut renderer = Renderer::new(ColorChoice::Plain);
        let output = renderer.render_text(&diagnostic, context);

        assert!(output.contains("1 | int x\n  |      ^\n"), "{output}");
    });
}

fn c_quoted(prefix: &str, quote: char, value: &str) -> String {
    let mut out = String::new();
    write_c_quoted(&mut out, prefix, quote, value).expect("writing to a string cannot fail");
    out
}

#[test]
fn quoted_strings_use_c_escapes() {
    assert_eq!(c_quoted("L", '"', "a\n\"\u{1b}"), "L\"a\\n\\\"\\033\"");
    assert_eq!(c_quoted("", '\'', "'"), "'\\''");
    assert_eq!(quote_spelling("12\0").to_string(), "`12`");
    let long = "a".repeat(MAX_QUOTED_SPELLING);
    assert_eq!(quote_spelling(&long).to_string(), format!("`{long}`"));
    assert_eq!(
        quote_spelling(&format!("{long}é")).to_string(),
        format!("`{}…`", &long[1..])
    );
    assert_eq!(count_of(1, "error").to_string(), "1 error");
    assert_eq!(count_of(2, "error").to_string(), "2 errors");
}
