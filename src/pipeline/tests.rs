//! Pipeline regressions: `--tokens` reporting order, and crate-level
//! syntax-tree consumption.

use std::path::PathBuf;

use super::*;
use crate::{
    configuration::{
        CStandard,
        CompilerConfiguration,
        ExtensionPolicy,
    },
    diagnostics::{
        ColorChoice as RenderColor,
        Renderer,
        ToDiagnostic,
    },
    translation_phases::{
        Context,
        GetSourceVectors,
        TranslationError,
        parsing::{
            ExternalDeclaration,
            Parser as LanguageParser,
        },
        preprocessing::{
            Preprocessor,
            PreprocessorError,
            PreprocessorErrorType,
            Token,
            TokenType,
        },
    },
    util::shared::SharedVec,
};

/// Preprocesses `source` as the CLI's `--tokens` does and scopes its tokens
/// and context together, since the tokens borrow the translation-unit arena.
fn with_preprocessed_with<R>(
    source: &str,
    configuration: CompilerConfiguration,
    inspect: impl FnOnce(
        crate::util::region_vec::IntoIter<Result<Token, TranslationError<'_>>>,
        &mut Context<'_>,
    ) -> R,
) -> R {
    let tu = Bump::new();
    let mut context = Context::with_configuration(&tu, configuration);
    let preprocess_arena = Bump::new();
    let preprocessor = Preprocessor::new(
        &preprocess_arena,
        &mut context,
        PathBuf::from("<test>").into_boxed_path(),
        source,
        SharedVec::default(),
        SharedVec::default(),
    );
    let items = preprocess_with_diagnostics(preprocessor, &mut context);
    inspect(items.into_iter(), &mut context)
}

fn with_preprocessed<R>(
    source: &str,
    inspect: impl FnOnce(
        crate::util::region_vec::IntoIter<Result<Token, TranslationError<'_>>>,
        &mut Context<'_>,
    ) -> R,
) -> R {
    with_preprocessed_with(source, CompilerConfiguration::default(), inspect)
}

fn with_parser<R>(source: &str, inspect: impl FnOnce(&mut LanguageParser<'_, '_, '_>) -> R) -> R {
    let tu = Bump::new();
    let mut context = Context::new(&tu);
    let preprocess_arena = Bump::new();
    let parse_arena = Bump::new();
    let preprocessor = Preprocessor::new(
        &preprocess_arena,
        &mut context,
        PathBuf::from("<test>").into_boxed_path(),
        source,
        SharedVec::default(),
        SharedVec::default(),
    );
    inspect(&mut LanguageParser::new(
        preprocessor,
        &mut context,
        &parse_arena,
    ))
}

#[test]
fn library_startup_options_share_the_cli_directive_path() {
    use crate::configuration::PreprocessingOption::{
        Define,
        Undefine,
    };
    let tu = Bump::new();
    let mut context = Context::new(&tu);
    context.preprocessing_options = &[Define("VALUE=3"), Undefine("VALUE"), Define("VALUE=7")];
    let source = "#if VALUE != 7\n#error option order\n#endif\nint value = VALUE;\n";
    let unit = parse_translation_unit(
        &mut context,
        Path::new("<test>"),
        source,
        HeaderSearch::default(),
    );
    assert_eq!(context.pending_error_count(), 0);
    assert_eq!(unit.external_declarations().len(), 1);
}

#[test]
fn diagnostic_is_yielded_before_the_token_produced_alongside_it() {
    with_preprocessed_with(
        "#if (0, 2)\nCOMMA_RESULT_2\n#endif\n",
        CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Deny),
        |mut iterator, context| {
            let error = iterator.next().unwrap().unwrap_err();
            assert!(matches!(
                &error,
                TranslationError::Preprocessing(PreprocessorError {
                    error_type: PreprocessorErrorType::CommaOperatorInPreprocessorExpression(
                        crate::translation_phases::ErrorSeverity::Error
                    ),
                    ..
                })
            ));
            let error_source_vectors = error.source_vectors(context);
            assert_ne!(context.get_source_vectors(error_source_vectors), []);

            let token = iterator.next().unwrap().unwrap();
            assert_eq!(token.kind, TokenType::Identifier);
            assert_eq!(context.string_cache.at(token.contents), "COMMA_RESULT_2");
            assert!(iterator.next().is_none());
        },
    );
}

#[test]
fn nested_token_pastes_keep_their_operands_while_tokens_are_yielded() {
    with_preprocessed(
        concat!(
            "#define LIM1(x) x##0; x##1;\n",
            "#define LIM2(x) LIM1(x##0) LIM1(x##1)\n",
            "#define LIM3(x) LIM2(x##0) LIM2(x##1)\n",
            "LIM3(int value)\n",
        ),
        |iterator, context| {
            let mut names = Vec::new();
            for token in iterator {
                let token = token.unwrap();
                assert_ne!(context.get_source_vectors(token.source_vectors), []);
                if token.kind == TokenType::Identifier {
                    names.push(context.string_cache.at(token.contents).to_owned());
                }
            }
            assert_eq!(
                names,
                [
                    "value000", "value001", "value010", "value011", "value100", "value101",
                    "value110", "value111",
                ]
            );
        },
    );
}

#[test]
fn adjacent_string_lookahead_keeps_the_buffered_token_provenance() {
    with_preprocessed("\"a\" identifier\n", |mut iterator, context| {
        let string = iterator.next().unwrap().unwrap();
        assert!(matches!(string.kind, TokenType::String(_)));
        let identifier = iterator.next().unwrap().unwrap();
        assert_eq!(identifier.kind, TokenType::Identifier);
        assert_eq!(context.string_cache.at(identifier.contents), "identifier");
        assert_ne!(context.get_source_vectors(identifier.source_vectors), []);
        assert!(iterator.next().is_none());
    });
}

#[test]
fn adjacent_string_lookahead_defers_buffered_token_diagnostics() {
    with_preprocessed("\"a\" 0xg\n", |mut iterator, _context| {
        assert!(matches!(
            iterator.next().unwrap().unwrap().kind,
            TokenType::String(_)
        ));
        assert!(matches!(
            iterator.next().unwrap().unwrap_err(),
            TranslationError::Preprocessing(PreprocessorError {
                error_type: PreprocessorErrorType::InvalidHexadecimalIntegerLiteral,
                ..
            })
        ));
        assert!(iterator.next().is_some());
        assert!(iterator.next().is_none());
    });
}

#[test]
fn adjacent_string_lookahead_keeps_current_token_before_later_diagnostics() {
    with_preprocessed("\"\\q\" 0xg", |mut iterator, _context| {
        assert!(matches!(
            iterator.next().unwrap().unwrap_err(),
            TranslationError::Preprocessing(PreprocessorError {
                error_type: PreprocessorErrorType::InvalidEscapeSequence,
                ..
            })
        ));
        assert!(matches!(
            iterator.next().unwrap().unwrap().kind,
            TokenType::String(_)
        ));
        assert!(matches!(
            iterator.next().unwrap().unwrap_err(),
            TranslationError::InitialProcessing(_)
        ));
        assert!(matches!(
            iterator.next().unwrap().unwrap_err(),
            TranslationError::Preprocessing(PreprocessorError {
                error_type: PreprocessorErrorType::InvalidHexadecimalIntegerLiteral,
                ..
            })
        ));
        assert!(matches!(
            iterator.next().unwrap().unwrap().kind,
            TokenType::Integer(_)
        ));
        assert!(iterator.next().is_none());
    });
}

#[test]
fn adjacent_string_lookahead_keeps_deferred_eof_diagnostic_provenance() {
    with_preprocessed("\"a\"\n#error boom\n", |mut iterator, context| {
        assert!(matches!(
            iterator.next().unwrap().unwrap().kind,
            TokenType::String(_)
        ));
        let error = iterator.next().unwrap().unwrap_err();
        assert!(matches!(
            &error,
            TranslationError::Preprocessing(PreprocessorError {
                error_type: PreprocessorErrorType::ErrorDirective(message),
                ..
            }) if message.trim() == "boom"
        ));
        let source_vectors = error.source_vectors(context);
        assert_ne!(context.get_source_vectors(source_vectors), []);
        assert!(iterator.next().is_none());
    });
}

#[test]
fn parser_yields_declarations_whose_debug_view_shows_their_syntax() {
    with_parser("int value;\n", |parser| {
        let root = parser.next_item().unwrap();
        assert!(matches!(root, ExternalDeclaration::Declaration(_)));
        assert!(parser.next_item().is_none());
        assert!(
            format!("{root:#?}").contains("init_declarators"),
            "the debug view should show the syntax the root refers to"
        );
    });
}

#[test]
fn parser_yields_a_parsed_initialized_declaration() {
    with_parser("int value = 1;\n", |parser| {
        assert!(matches!(
            parser.next_item().unwrap(),
            ExternalDeclaration::Declaration(_)
        ));
        assert!(parser.next_item().is_none());
        assert!(parser.context().take_pending_errors().is_empty());
    });
}

#[test]
fn complete_translation_unit_owns_ordered_roots_and_typed_syntax() {
    let tu = Bump::new();
    let mut context = Context::new(&tu);
    let preprocess_arena = Bump::new();
    let parse_arena = Bump::new();
    let preprocessor = Preprocessor::new(
        &preprocess_arena,
        &mut context,
        PathBuf::from("<test>").into_boxed_path(),
        "int first; int second = 2;\n",
        SharedVec::default(),
        SharedVec::default(),
    );

    let unit =
        LanguageParser::new(preprocessor, &mut context, &parse_arena).parse_translation_unit();

    assert_eq!(unit.external_declarations().len(), 2);
    let ExternalDeclaration::Declaration(first) = unit.external_declarations()[0] else {
        panic!("expected the first root to be a declaration")
    };
    let ExternalDeclaration::Declaration(second) = unit.external_declarations()[1] else {
        panic!("expected the second root to be a declaration")
    };
    assert_eq!(first.init_declarators.len(), 1);
    assert_eq!(second.init_declarators.len(), 1);
}

#[test]
fn cli_parser_details_render_recovery_ranges_and_notes() {
    let tu = Bump::new();
    let mut context = Context::new(&tu);
    let preprocess_arena = Bump::new();
    let parse_arena = Bump::new();
    let preprocessor = Preprocessor::new(
        &preprocess_arena,
        &mut context,
        PathBuf::from("<test>").into_boxed_path(),
        "int first extra junk; int after;\n",
        SharedVec::default(),
        SharedVec::default(),
    );
    let _unit =
        LanguageParser::new(preprocessor, &mut context, &parse_arena).parse_translation_unit();
    let errors = context.take_pending_errors();
    let diagnostic = errors
        .iter()
        .find_map(|error| match error {
            | TranslationError::Parsing(error)
                if error
                    .recovery
                    .is_some_and(|recovery| recovery.discarded_tokens > 0) =>
                Some(error),
            | _ => None,
        })
        .expect("expected discarded-input recovery");

    let source = diagnostic.source_vectors(&mut context);
    let rendered = Renderer::new(RenderColor::Plain)
        .render(&diagnostic.to_diagnostic(&context, source), &context);
    let expected = [
        "error: expected `,`, `=`, `;`, or a function body after the declarator, found identifier \
         `extra`",
        " --> <test>:1:11",
        "  |",
        "1 | int first extra junk; int after;",
        "  |           ^^^^^ ---- skipped to recover",
        "  |           |",
        "  |           expected one of `,`, `=`, `;`, or `{`",
        "  |",
        "  = help: if this starts a new declaration, add `;` before it",
        "",
        "",
    ]
    .join("\n");
    assert_eq!(rendered, expected);
}
use crate::translation_phases::parsing::{
    DirectDeclarator,
    TypeSpecifiers,
};

#[test]
fn sibling_consumer_can_traverse_parameter_and_member_syntax() {
    let tu = Bump::new();
    let mut context = Context::new(&tu);
    let preprocess_arena = Bump::new();
    let parse_arena = Bump::new();
    let preprocessor = Preprocessor::new(
        &preprocess_arena,
        &mut context,
        PathBuf::from("<syntax-tree-consumer-test>").into_boxed_path(),
        "struct S { int member : 3; }; int f(int parameter);",
        SharedVec::default(),
        SharedVec::default(),
    );
    let unit =
        LanguageParser::new(preprocessor, &mut context, &parse_arena).parse_translation_unit();

    let ExternalDeclaration::Declaration(struct_root) = unit.external_declarations()[0] else {
        panic!("expected struct declaration")
    };
    let TypeSpecifiers::StructOrUnion(specifier) =
        struct_root.declaration_specifiers.type_specifiers
    else {
        panic!("expected struct type specifier")
    };
    let members = specifier
        .struct_declaration_list
        .expect("struct definition has members");
    assert_eq!(members[0].type_specifiers, TypeSpecifiers::Int);
    let member_declarators = members[0].struct_declarator_list;
    assert!(member_declarators[0].declarator.is_some());
    assert!(member_declarators[0].bitfield_width.is_some());

    let ExternalDeclaration::Declaration(function_root) = unit.external_declarations()[1] else {
        panic!("expected function declaration")
    };
    let declarator = function_root.init_declarators[0].declarator;
    let parameter_list = declarator
        .kind
        .iter()
        .find_map(|direct| match direct {
            | DirectDeclarator::Function { parameter_list, .. } => Some(*parameter_list),
            | _ => None,
        })
        .expect("function declarator has a parameter list");
    let parameters = parameter_list;
    assert_eq!(
        parameters[0].declaration_specifiers.type_specifiers,
        TypeSpecifiers::Int
    );
    assert!(parameters[0].declarator.is_some());
}

/// The regions and commit one compilation holds at its peak on this thread,
/// rendering every diagnostic as the CLI does.
fn compilation_peak(source: &str, path: &Path) -> crate::util::vm::accounting::Usage {
    use crate::util::vm::accounting;
    let before = accounting::live();
    accounting::reset_peak();
    {
        let tu = Bump::new();
        let mut context = Context::new(&tu);
        let source = tu.alloc_str(source);
        let _unit = parse_translation_unit(&mut context, path, source, HeaderSearch::default());
        let mut renderer = Renderer::new(RenderColor::Plain);
        for error in context.take_pending_errors() {
            let location = error.source_vectors(&mut context);
            drop(renderer.render(&error.to_diagnostic(&context, location), &context));
        }
    }
    assert_eq!(
        accounting::live(),
        before,
        "a compilation releases its regions"
    );
    let peak = accounting::peak();
    accounting::Usage {
        regions:   peak.regions - before.regions,
        reserved:  peak.reserved - before.reserved,
        committed: peak.committed - before.committed,
    }
}

/// A directory with a header that includes itself until the nesting limit.
struct RecursiveHeader(crate::test_support::TempDir);

impl RecursiveHeader {
    fn new() -> Self {
        let directory = crate::test_support::TempDir::new("region-budget");
        directory.write("loop.h", "#include \"loop.h\"\nint header_after;\n");
        Self(directory)
    }

    fn main(&self) -> PathBuf {
        self.0.join("main.c")
    }
}

const RECURSIVE_MAIN: &str = "#include \"loop.h\"\nint caller_after;\n";

/// Regions one compilation may hold at once: the translation-unit,
/// preprocessor, and expansion arenas, the string cache's two buffers, three
/// provenance stores, the parser's token stream, and the renderer's scratch.
/// The parse arena, the file-scope typedef set, and the parsed roots are
/// reserved only after the preprocessor's arenas and its provenance store are
/// released. Lexed files and include depth add none.
const MAX_REGIONS_PER_COMPILATION: usize = 10;

fn assert_within_budget(usage: crate::util::vm::accounting::Usage, what: &str) {
    assert!(
        usage.regions <= MAX_REGIONS_PER_COMPILATION,
        "{what}: {usage:?}"
    );
    // At 100 GiB per region, a compilation reserves at most 1000 GiB, so
    // over a hundred fit in 128 TiB of user address space at once.
    assert!(
        usage.reserved <= MAX_REGIONS_PER_COMPILATION * 100 * (1 << 30),
        "{what}: {usage:?}"
    );
    assert!(
        usage.committed <= MAX_SMALL_COMPILATION_COMMIT,
        "{what}: {usage:?}"
    );
}

/// Commit a small compilation may hold at once. Commit starts at 64 KiB per
/// region and grows with use, so 1 MiB holds every region's first step. On
/// Linux a region commits whole 2 MiB huge pages, so each region in use
/// holds at least one, and the bound is one per region the budget allows.
const MAX_SMALL_COMPILATION_COMMIT: usize = if cfg!(all(target_os = "linux", not(miri))) {
    MAX_REGIONS_PER_COMPILATION * crate::util::vm::HUGE_PAGE
} else {
    1 << 20
};

#[test]
fn small_compilations_hold_few_regions_and_commit_little() {
    for source in [
        "int main(void) { return 0; }\n",
        "#define TWICE(x) (x) + (x)\nint main(void) { return TWICE(1); }\n",
        "int main(void) { return 0 }\n#include <nowhere.h>\nint x = ;\n",
        "typedef int t;\nt main(void) { t x = 0; return x; }\n",
    ] {
        let usage = compilation_peak(source, Path::new("<input>"));
        assert_within_budget(usage, source);
    }
}

/// Parsing reserves its arena, the file-scope typedef set, and the roots, but
/// releases the preprocessor's provenance store first, which nothing reads
/// once phase 6 has ended. Holding it would make this compilation's peak 9.
#[test]
fn parsing_does_not_keep_the_preprocessor_provenance_region() {
    let usage = compilation_peak(
        "typedef int t;\nt main(void) { t x = 0; return x; }\n",
        Path::new("<input>"),
    );
    assert!(usage.regions <= 8, "{usage:?}");
}

#[test]
fn include_depth_adds_no_regions() {
    let headers = RecursiveHeader::new();
    let usage = compilation_peak(RECURSIVE_MAIN, &headers.main());
    assert_within_budget(usage, "recursive include to the nesting limit");
}

#[test]
fn concurrent_deep_compilations_fit_in_the_address_space() {
    let headers = RecursiveHeader::new();
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..16)
            .map(|_| scope.spawn(|| compilation_peak(RECURSIVE_MAIN, &headers.main())))
            .collect();
        for worker in workers {
            let usage = worker.join().expect("every compilation completes");
            assert_within_budget(usage, "concurrent recursive include");
        }
    });
}

/// The parse arena's high-water mark after parsing `source`, with the number
/// of diagnostics reported.
fn parse_arena_high_water(source: &str) -> (usize, usize) {
    let tu = Bump::new();
    let mut context = Context::new(&tu);
    let preprocessed = with_preprocessor(
        &mut context,
        Path::new("<parse-arena-test>"),
        source,
        HeaderSearch::default(),
        |preprocessor, context, _pp| Parser::preprocess(preprocessor, context),
    );
    let parse = Bump::new();
    let _unit = parse_with_arena(preprocessed, &mut context, &parse);
    (parse.high_water(), context.take_pending_errors().len())
}

/// Every frame family, the scopes and labels of a function body, and two
/// recovery scans. Names repeat between copies, so a copy adds nothing that
/// a parser must remember after it.
const EVERY_FRAME: &str = "typedef int t;
struct s { int a : 3; struct { int b; } inner; t c[2]; };
enum e { E0, E1 = 2 };
int (*f(int p, int (*q)(int, ...)))(char);
int k(a, b) int a; char *b; { return a; }
int g(int x) {
    int y = x, z[3] = { [1] = 2, 3 }, *w = &y;
    struct s v = { .a = 1, .inner = { .b = 2 }, .c = { [0] = 1 } };
    switch (x) { case 1: y++; break; default: y--; }
    for (int i = 0; i < 3; i++) { if (i) continue; else y += i; }
    while (y > 0) { t u = y; y = u - 1; }
    do { y++; } while (y < 2);
  again:
    y = (int)sizeof(struct s) + k(y, 0) + f(1, 0)('a') + z[1] + (t){ 0 } + *w;
    if (y < 0) goto again;
    return y ? x : -x;
}
void h(void) { int q = ; q = (1 + ; }
";

#[test]
fn parse_arena_use_does_not_grow_with_repeated_declarations() {
    let (once, once_errors) = parse_arena_high_water(&EVERY_FRAME.repeat(40));
    let (twice, twice_errors) = parse_arena_high_water(&EVERY_FRAME.repeat(80));
    assert!(once_errors > 0, "the input exercises recovery");
    assert_eq!(twice_errors, 2 * once_errors);
    assert!(once > 0, "frames and scopes use the parse arena");
    assert_eq!(twice, once, "parse arena use grows with the input");
}

#[test]
fn parse_arena_use_does_not_grow_with_distinct_block_scope_names() {
    use std::fmt::Write as _;
    let functions = |count: usize| {
        let mut source = String::new();
        for index in 0..count {
            writeln!(
                source,
                "int g(int p{index}) {{ int l{index} = p{index}; {{ typedef int t{index};                  t{index} m{index} = l{index}; }} return l{index}; }}"
            )
            .expect("writing to a string cannot fail");
        }
        source
    };
    let (once, _) = parse_arena_high_water(&functions(400));
    let (twice, _) = parse_arena_high_water(&functions(800));
    assert_eq!(twice, once, "block-scope bookkeeping grows with the input");
}

#[test]
fn parse_arena_use_does_not_grow_with_distinct_file_scope_names() {
    use std::fmt::Write as _;
    let declarations = |count: usize| {
        let mut source = String::new();
        for index in 0..count {
            writeln!(
                source,
                "int i{index} = {index}; typedef int t{index}; t{index} v{index}; int t{index};"
            )
            .expect("writing to a string cannot fail");
        }
        source
    };
    let (once, once_errors) = parse_arena_high_water(&declarations(2000));
    let (twice, _) = parse_arena_high_water(&declarations(4000));
    assert_eq!(once_errors, 0);
    assert!(once > 0, "declarations use the parse arena");
    assert_eq!(twice, once, "file-scope names grow the parse arena");
}

/// Runs phases 1-7 on `source` and hands the result to a caller outside the
/// semantic analyzer, as lowering will be.
fn with_semantics<R>(
    source: &str,
    inspect: impl for<'tu> FnOnce(
        &mut Context<'tu>,
        &ParsedTranslationUnit<'tu>,
        &crate::translation_phases::semantic_analysis::SemanticTranslationUnit<'tu>,
    ) -> R,
) -> R {
    let tu = Bump::new();
    let source = tu.alloc_str(&format!("{source}\n"));
    let mut context = Context::new(&tu);
    let unit = parse_translation_unit(
        &mut context,
        Path::new("<test>"),
        source,
        HeaderSearch::default(),
    );
    let semantics = analyze_translation_unit(&mut context, &unit);
    inspect(&mut context, &unit, &semantics)
}

#[test]
fn lowering_names_the_retained_semantic_vocabulary() {
    use crate::translation_phases::semantic_analysis::{
        ArrayBound,
        Binding,
        BindingKind,
        Duration,
        Field,
        Layout,
        Linkage,
        Scalar,
        Tag,
        TagKind,
        TypeId,
        TypeKind,
    };
    with_semantics(
        "struct S { int a; _Atomic long b; } s; int g[3]; unsigned char *p;",
        |context, _, sema| {
            assert_eq!(context.pending_error_count(), 0);
            let binding = |name: &str| -> Binding {
                *sema
                    .bindings
                    .iter()
                    .find(|b| context.string_cache.at(b.name.name) == name)
                    .expect("binding is retained")
            };
            let s = binding("s");
            assert_eq!(s.kind, BindingKind::Object);
            assert_eq!(s.linkage, Linkage::External);
            assert_eq!(s.duration, Duration::Static);
            let tag: &Tag<'_> = sema.types.tag(s.ty).expect("s has a structure type");
            assert_eq!(tag.kind, TagKind::Struct);
            let fields: &[Field<'_>] = tag.fields.get();
            assert_eq!(fields.len(), 2);
            assert_eq!(fields[1].offset, 8);
            let atomic: TypeId = fields[1].ty;
            assert!(matches!(sema.types.kind(atomic), TypeKind::Atomic(_)));
            assert_eq!(
                sema.types.kind(sema.types.non_atomic(atomic)),
                TypeKind::Scalar(Scalar::Long)
            );
            assert_eq!(
                sema.types.layout(s.ty),
                Some(Layout {
                    size:  16,
                    align: 8,
                })
            );
            let g = binding("g");
            assert!(matches!(
                sema.types.kind(g.ty),
                TypeKind::Array(_, ArrayBound::Constant(3))
            ));
            assert!(!sema.types.unanalyzed(g.ty));
            let TypeKind::Pointer(target) = sema.types.kind(binding("p").ty) else {
                panic!("p is a pointer");
            };
            assert_eq!(
                sema.types.kind(target),
                TypeKind::Scalar(Scalar::UnsignedChar)
            );
            assert!(sema.types.tag(target).is_none());
        },
    );
}

#[test]
fn retained_types_report_unanalyzed_array_elements() {
    use crate::translation_phases::semantic_analysis::TypeKind;
    // GNU `__typeof__` of an unmodeled builtin call has no modeled type.
    with_semantics(
        "__typeof__(__builtin_expect(1, 1)) a[2];",
        |context, _, sema| {
            let a = sema
                .bindings
                .iter()
                .find(|b| context.string_cache.at(b.name.name) == "a")
                .expect("a is bound");
            assert!(matches!(sema.types.kind(a.ty), TypeKind::Array(..)));
            assert!(sema.types.unanalyzed(a.ty));
        },
    );
}

#[test]
fn lowering_refuses_a_unit_with_a_semantic_error() {
    with_semantics("int f(void) { return undeclared; }", |context, _, sema| {
        assert_eq!(sema.errors, 1);
        assert_eq!(context.error_count(), 1);
        assert!(!sema.lowerable(context));
    });
}

#[test]
fn lowering_accepts_a_unit_with_only_warnings() {
    // An incomplete external tentative array is completed with one element
    // and a warning (C99 §6.9.2p2), and `#warning` is a GNU warning.
    with_semantics(
        "#warning check\nint a[];\nint f(void) { return a[0]; }",
        |context, _, sema| {
            assert_eq!(context.pending_error_count(), 2, "two warnings are queued");
            assert_eq!(sema.errors, 0);
            assert_eq!(context.error_count(), 0);
            assert!(sema.lowerable(context));
        },
    );
}

#[test]
fn lowering_accepts_a_clean_unit() {
    with_semantics("int f(int x) { return x + 1; }", |context, _, sema| {
        assert_eq!(context.pending_error_count(), 0);
        assert_eq!(sema.errors, 0);
        assert!(sema.lowerable(context));
    });
}

#[test]
fn lowering_refuses_a_unit_whose_error_came_from_an_earlier_phase() {
    with_semantics("int x = ;\nint y;", |context, _, sema| {
        assert_eq!(sema.errors, 0, "the parse error is not semantic");
        assert!(context.error_count() > 0);
        assert!(!sema.lowerable(context));
        // A reporter that drains the queue does not clear the gate.
        while context.pop_pending_error().is_some() {}
        assert_eq!(context.pending_error_count(), 0);
        assert!(context.error_count() > 0);
        assert!(!sema.lowerable(context));
    });
}
