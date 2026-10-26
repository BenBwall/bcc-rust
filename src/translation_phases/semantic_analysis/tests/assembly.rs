//! GNU assembly statements inside functions: the location analysis picks
//! for each operand, the parsed template and clobbers it retains, and the
//! constraint, lvalue, constant and template diagnostics.
//! C99 §J.5.10, p. 514; PDF p. 526. GNU extension: GCC manual, "Extended
//! Asm".

use super::{
    super::assembly::AsmFault,
    *,
};

fn kinds(source: &str) -> Vec<SemanticErrorKind> {
    let mut kinds = Vec::new();
    with_source(source, |context, _| {
        kinds.extend(
            context
                .take_pending_errors()
                .iter()
                .filter_map(|e| match e {
                    | TranslationError::Semantic(e) => Some(e.kind),
                    | _ => None,
                }),
        );
    });
    kinds
}

/// Runs `check` on the facts of the only `asm` statement in `source`.
fn facts(source: &str, check: impl FnOnce(&AsmFacts<'_>, &SemanticTranslationUnit<'_>)) {
    with_source(source, |context, s| {
        assert_eq!(
            context.pending_error_count(),
            0,
            "{:?}",
            context.take_pending_errors()
        );
        let statements: Vec<_> = s.assembly.iter().collect();
        assert_eq!(statements.len(), 1, "{source}");
        check(statements[0], s);
    });
}

#[test]
fn operands_take_registers_memory_immediates_ties_and_flags() {
    facts(
        "int f(int x, int *p) { int r, z; __asm__(\"op %0 %1 %2 %3 %4 %5\" : \"=&r\"(r), \
         \"+m\"(*p), \"=@ccz\"(z) : \"g\"(5), \"0\"(x), \"a\"(x)); return r + z; }",
        |facts, _| {
            let kinds: Vec<_> = facts
                .operands
                .iter()
                .map(|operand| (operand.role, operand.kind, operand.early_clobber))
                .collect();
            assert_eq!(
                kinds,
                [
                    (
                        AsmRole::Output,
                        AsmOperandKind::Register(AsmRegisterClass::General(
                            AsmRegisterClass::ANY_GENERAL
                        )),
                        true
                    ),
                    (AsmRole::Update, AsmOperandKind::Memory, false),
                    (
                        AsmRole::Output,
                        AsmOperandKind::Flag(AsmCondition::named("z").unwrap()),
                        false
                    ),
                    (
                        AsmRole::Input,
                        AsmOperandKind::Immediate { symbolic: false },
                        false
                    ),
                    (AsmRole::Input, AsmOperandKind::Tied(0), false),
                    (
                        AsmRole::Input,
                        AsmOperandKind::Register(AsmRegisterClass::General(1)),
                        false
                    ),
                ]
            );
            assert!(
                !facts.volatile,
                "an asm with outputs is not implicitly volatile"
            );
        },
    );
}

/// `g` takes a register for a constant no instruction encodes as an
/// immediate, and memory for an aggregate; `i` takes an address constant.
#[test]
fn generic_constraints_follow_the_operand() {
    facts(
        "struct S { int a[4]; } s; int g(void); void f(void) { __asm__ volatile(\"\" :: \
         \"g\"(0x100000000LL), \"g\"(s), \"i\"(&s), \"i\"(g)); }",
        |facts, _| {
            let kinds: Vec<_> = facts.operands.iter().map(|operand| operand.kind).collect();
            assert_eq!(
                kinds,
                [
                    AsmOperandKind::Register(AsmRegisterClass::General(
                        AsmRegisterClass::ANY_GENERAL
                    )),
                    AsmOperandKind::Memory,
                    AsmOperandKind::Immediate { symbolic: true },
                    AsmOperandKind::Immediate { symbolic: true },
                ]
            );
            assert!(facts.volatile);
        },
    );
}

#[test]
fn templates_resolve_names_modifiers_labels_and_escapes() {
    facts(
        "void f(int x) { __asm__ goto(\"mov %k[v], %%eax{|}%=\\n\\tjz %l1\" :: [v] \"r\"(x) : \
         \"cc\", \"memory\", \"%rcx\", \"xmm3\" : out); out: ; }",
        |facts, _| {
            assert_eq!(
                facts.template,
                [
                    AsmPiece::Text("mov "),
                    AsmPiece::Operand {
                        operand:  0,
                        modifier: Some(b'k'),
                    },
                    AsmPiece::Text(", "),
                    AsmPiece::Text("%"),
                    AsmPiece::Text("eax"),
                    AsmPiece::DialectOpen,
                    AsmPiece::DialectNext,
                    AsmPiece::DialectClose,
                    AsmPiece::UniqueId,
                    AsmPiece::Text("\n\tjz "),
                    AsmPiece::Label(0),
                ]
            );
            assert_eq!(facts.jumps.len(), 1);
            assert!(facts.volatile, "asm goto is implicitly volatile");
            let clobbers = facts.clobbers;
            assert!(clobbers.flags && clobbers.memory);
            assert_eq!(clobbers.general, 1 << 1);
            assert_eq!(clobbers.sse, 1 << 3);
        },
    );
    // Basic assembly substitutes nothing.
    facts(
        "void f(void) { __asm__(\"movl %eax, %ebx\"); }",
        |facts, _| {
            assert_eq!(facts.template, [AsmPiece::Text("movl %eax, %ebx")]);
            assert!(facts.volatile);
        },
    );
}

#[test]
fn invalid_statements_name_their_fault() {
    let fault = |source: &str| {
        let kinds = kinds(source);
        assert_eq!(kinds.len(), 1, "{source}: {kinds:?}");
        let SemanticErrorKind::InvalidAsm(fault) = kinds[0] else {
            panic!("{source}: {kinds:?}");
        };
        fault
    };
    for (body, expected) in [
        ("__asm__(\"\" : \"r\"(r));", AsmFault::OutputModifier),
        ("__asm__(\"\" : \"=W\"(r));", AsmFault::OutputConstraint),
        ("__asm__(\"\" :: \"=r\"(x));", AsmFault::InputConstraint),
        ("__asm__(\"\" :: \"&r\"(x));", AsmFault::InputConstraint),
        ("__asm__(\"\" : \"=r\"(x + 1));", AsmFault::OutputLvalue),
        ("__asm__(\"\" : \"=r\"(c));", AsmFault::OutputLvalue),
        ("__asm__(\"\" : \"=m\"(b.f));", AsmFault::OutputLvalue),
        ("__asm__(\"\" :: \"m\"(x + 1));", AsmFault::InputLvalue),
        ("__asm__(\"\" :: \"n\"(x));", AsmFault::ImmediateRequired),
        ("__asm__(\"\" :: \"i\"(&x));", AsmFault::ImmediateRequired),
        ("__asm__(\"\" :: \"I\"(32));", AsmFault::ImmediateRange),
        ("__asm__(\"\" :: \"K\"(-129));", AsmFault::ImmediateRange),
        ("__asm__(\"\" : \"=r\"(s));", AsmFault::OperandType),
        ("__asm__(\"\" : \"=r\"(w));", AsmFault::OperandType),
        ("__asm__(\"\" :: \"1\"(x) : );", AsmFault::TiedOperand),
        ("__asm__(\"\" : \"=r\"(r) : \"0\"(w));", AsmFault::TiedType),
        (
            "__asm__(\"\" : \"=r\"(r) : \"0\"(1.0));",
            AsmFault::TiedType,
        ),
        ("__asm__(\"\" ::: \"rfoo\");", AsmFault::UnknownRegister),
        (
            "__asm__(\"\" : \"=a\"(r) :: \"eax\");",
            AsmFault::ClobberConflict,
        ),
        ("__asm__(\"%\" : \"=r\"(r));", AsmFault::Escape),
        ("__asm__(\"%#\" : \"=r\"(r));", AsmFault::Escape),
        ("__asm__(\"%1\" : \"=r\"(r));", AsmFault::OperandNumber),
        ("__asm__(\"%[none]\" : \"=r\"(r));", AsmFault::OperandName),
        ("__asm__(\"%y0\" : \"=r\"(r));", AsmFault::Modifier),
        (
            "__asm__(\"\" : [a] \"=r\"(r), [a] \"=r\"(r));",
            AsmFault::DuplicateName,
        ),
        ("__asm__(L\"nop\");", AsmFault::WideString),
    ] {
        let source = format!(
            "struct S {{ int a[4]; }} s; struct B {{ int f: 3; }} b; const int c = 1;\nint f(int \
             x) {{ int r; __int128 w; {body} return r; }}"
        );
        assert_eq!(fault(&source), expected, "{body}");
    }
}

/// An `asm goto` label is a jump: it must exist and must not enter the
/// scope of a variably modified object.
/// C99: §6.8.6.1p1, p. 137; PDF p. 149.
#[test]
fn goto_labels_are_checked_as_jumps() {
    assert_eq!(
        kinds("void f(void) { __asm__ goto(\"\" :::: nowhere); }"),
        [SemanticErrorKind::UndefinedLabel]
    );
    assert_eq!(
        kinds("void f(int n) { __asm__ goto(\"\" :::: in); { int a[n]; in: a[0] = 0; } }"),
        [SemanticErrorKind::JumpIntoVariableScope]
    );
    assert_eq!(
        kinds("void f(int x) { __asm__ goto(\"jmp %l[b]\" :::: a, b); a: b: ; }"),
        []
    );
}

/// Each goto label has its own jump, in label order.
#[test]
fn goto_jumps_follow_the_labels_in_order() {
    with_source(
        "void f(void) { __asm__ goto(\"\" :::: second, first); first: ; second: ; }",
        |context, s| {
            assert_eq!(context.pending_error_count(), 0);
            let facts = s.assembly.iter().next().expect("the asm is retained");
            let labels: Vec<_> = facts
                .jumps
                .iter()
                .map(|jump| match s.control.jumps[jump.0].target {
                    | ControlTarget::Label(label) => {
                        let definition = s.control.labels[label.0]
                            .definition
                            .expect("the label is defined");
                        let crate::translation_phases::parsing::syntax::StatementType::Label(
                            name,
                            _,
                        ) = definition.kind
                        else {
                            panic!("a label definition is a labeled statement");
                        };
                        String::from(context.string_cache.at(name.name))
                    },
                    | _ => panic!("an asm goto jump reaches a label"),
                })
                .collect();
            assert_eq!(labels, ["second", "first"]);
        },
    );
}

/// One alternative column is chosen for the whole statement: `=a,b` and
/// `=b,a` take `%rax` and `%rbx` together, never one register twice, and
/// an aggregate input moves every operand to the memory column.
/// GNU extension: GCC manual, "Multiple Alternative Constraints".
#[test]
fn alternatives_are_chosen_for_the_whole_statement() {
    facts(
        "void f(void) { int x, y; __asm__(\"\" : \"=a,b\"(x), \"=b,a\"(y)); }",
        |facts, _| {
            let kinds: Vec<_> = facts.operands.iter().map(|operand| operand.kind).collect();
            assert_eq!(
                kinds,
                [
                    AsmOperandKind::Register(AsmRegisterClass::General(1)),
                    AsmOperandKind::Register(AsmRegisterClass::General(1 << 3)),
                ]
            );
        },
    );
    facts(
        "struct S { int a[4]; } s; void f(void) { int x; __asm__(\"\" : \"=r,m\"(x) : \
         \"r,m\"(s)); }",
        |facts, _| {
            let kinds: Vec<_> = facts.operands.iter().map(|operand| operand.kind).collect();
            assert_eq!(kinds, [AsmOperandKind::Memory, AsmOperandKind::Memory]);
        },
    );
    for (body, expected) in [
        (
            "__asm__(\"\" : \"=r,m\"(x) : \"r\"(x));",
            AsmFault::Alternatives,
        ),
        // The tied output is in memory in the only alternative.
        (
            "__asm__(\"\" : \"=m\"(x) : \"0\"(x));",
            AsmFault::TiedOperand,
        ),
        ("__asm__(\"\" : \"=r,\"(x));", AsmFault::OutputConstraint),
    ] {
        let kinds = kinds(&format!("void f(void) {{ int x; {body} }}"));
        assert_eq!(kinds, [SemanticErrorKind::InvalidAsm(expected)], "{body}");
    }
}

/// GCC numbers the input each `+` output reads after the written inputs,
/// and the goto labels after those: with one `+r` output the first label
/// is `%l2`, and `%1` names the output again.
#[test]
fn update_outputs_count_twice_before_goto_labels() {
    facts(
        "void f(int x) { __asm__ goto(\"%1 %l2\" : \"+r\"(x) ::: out); out: ; }",
        |facts, _| {
            assert_eq!(
                facts.template,
                [
                    AsmPiece::Operand {
                        operand:  0,
                        modifier: None,
                    },
                    AsmPiece::Text(" "),
                    AsmPiece::Label(0),
                ]
            );
        },
    );
    assert_eq!(
        kinds("void f(int x) { __asm__ goto(\"%l1\" : \"+r\"(x) ::: out); out: ; }"),
        [SemanticErrorKind::InvalidAsm(AsmFault::OperandNumber)]
    );
}

/// Every goto label is checked, not only the first.
/// C99: §6.8.6.1p1, p. 137; PDF p. 149.
#[test]
fn every_goto_label_is_checked_for_variably_modified_scopes() {
    assert_eq!(
        kinds("void f(int n) { __asm__ goto(\"\" :::: ok, in); ok: { int a[n]; in: a[0] = 0; } }"),
        [SemanticErrorKind::JumpIntoVariableScope]
    );
}
