//! Checked C library builtins: exact target prototypes, ordinary argument
//! checking, retained instances and effects, and the `FILE` requirement.
use super::*;
use crate::{
    builtins::{
        BuiltinEffectClass,
        BuiltinOperation,
    },
    configuration::CompilerConfiguration,
    target::Target,
    translation_phases::{
        TranslationError,
        semantic_analysis::facts::effects::EffectSet,
    },
};

/// Every checked builtin, called once with valid operands.
const CALLS: &str = "typedef struct stream FILE;
int f(char *b, FILE *stream, const char *s, ...) {
    __builtin_va_list ap; __builtin_va_start(ap, s);
    int n = __builtin___vsprintf_chk(b, 1, 8, s, ap)
        + __builtin___vsnprintf_chk(b, 4, 1, 8, s, ap)
        + __builtin___vprintf_chk(1, s, ap)
        + __builtin___vfprintf_chk(stream, 1, s, ap)
        + __builtin___sprintf_chk(b, 1, 8, s, 1)
        + __builtin___snprintf_chk(b, 4, 1, 8, s, 1.0);
    char *p = __builtin___strcpy_chk(b, s, 8);
    p = __builtin___stpcpy_chk(b, s, 8);
    p = __builtin___strncpy_chk(b, s, 2, 8);
    p = __builtin___stpncpy_chk(b, s, 2, 8);
    p = __builtin___strcat_chk(b, s, 8);
    p = __builtin___strncat_chk(b, s, 2, 8);
    void *q = __builtin___memcpy_chk(b, s, 2, 8);
    q = __builtin___memmove_chk(b, s, 2, 8);
    q = __builtin___mempcpy_chk(b, s, 2, 8);
    q = __builtin___memset_chk(b, 0, 2, 8);
    __builtin_va_end(ap);
    return n + (p != 0) + (q != 0);
}";

/// A type as C spells it, with `size_t` and `va_list` resolved.
fn spell(unit: &SemanticTranslationUnit<'_>, context: &Context<'_>, ty: TypeId) -> String {
    let constant = if ty.qualifiers.contains(TypeQualifiers::CONST) {
        "const "
    } else {
        ""
    };
    match unit.types.nodes[ty.index] {
        | TypeKind::Scalar(scalar) => format!("{constant}{}", scalar.spelling()),
        | TypeKind::Pointer(target) => format!("{constant}{} *", spell(unit, context, target)),
        | TypeKind::Tag(tag) => format!(
            "{constant}{}",
            context.string_cache.at(unit.types.tags[tag].name.unwrap())
        ),
        | TypeKind::Function {
            result,
            parameters,
            prototype,
            variadic,
        } => {
            assert!(prototype);
            let mut text = format!("{} (", spell(unit, context, result));
            let parameters: Vec<_> = parameters
                .iter()
                .map(|&parameter| spell(unit, context, parameter))
                .collect();
            text.push_str(&parameters.join(", "));
            if variadic {
                text.push_str(", ...");
            }
            text.push(')');
            text
        },
        | kind => panic!("unexpected {kind:?}"),
    }
}

#[test]
fn checked_builtins_project_exact_target_prototypes() {
    for target in [
        Target::LinuxGnu,
        Target::LinuxMusl,
        Target::WindowsGnu,
        Target::WindowsMsvc,
    ] {
        let windows = matches!(target, Target::WindowsGnu | Target::WindowsMsvc);
        let size = if windows {
            "unsigned long long"
        } else {
            "unsigned long"
        };
        // System V's list is an array of one record, adjusted to a pointer
        // to it; Windows' is `char *`.
        let list = if windows {
            "char *"
        } else {
            "__builtin_va_list *"
        };
        let expected = [
            (
                "__builtin___vsprintf_chk",
                format!("int (char *, int, {size}, const char *, {list})"),
            ),
            (
                "__builtin___vsnprintf_chk",
                format!("int (char *, {size}, int, {size}, const char *, {list})"),
            ),
            (
                "__builtin___vprintf_chk",
                format!("int (int, const char *, {list})"),
            ),
            (
                "__builtin___vfprintf_chk",
                format!("int (stream *, int, const char *, {list})"),
            ),
            (
                "__builtin___sprintf_chk",
                format!("int (char *, int, {size}, const char *, ...)"),
            ),
            (
                "__builtin___snprintf_chk",
                format!("int (char *, {size}, int, {size}, const char *, ...)"),
            ),
            (
                "__builtin___strcpy_chk",
                format!("char * (char *, const char *, {size})"),
            ),
            (
                "__builtin___stpcpy_chk",
                format!("char * (char *, const char *, {size})"),
            ),
            (
                "__builtin___strncpy_chk",
                format!("char * (char *, const char *, {size}, {size})"),
            ),
            (
                "__builtin___stpncpy_chk",
                format!("char * (char *, const char *, {size}, {size})"),
            ),
            (
                "__builtin___strcat_chk",
                format!("char * (char *, const char *, {size})"),
            ),
            (
                "__builtin___strncat_chk",
                format!("char * (char *, const char *, {size}, {size})"),
            ),
            (
                "__builtin___memcpy_chk",
                format!("void * (void *, const void *, {size}, {size})"),
            ),
            (
                "__builtin___memmove_chk",
                format!("void * (void *, const void *, {size}, {size})"),
            ),
            (
                "__builtin___mempcpy_chk",
                format!("void * (void *, const void *, {size}, {size})"),
            ),
            (
                "__builtin___memset_chk",
                format!("void * (void *, int, {size}, {size})"),
            ),
        ];
        with_configuration(
            CALLS,
            CompilerConfiguration::default().with_target(target),
            |context, unit| {
                assert_eq!(
                    unit.errors,
                    0,
                    "{target:?}: {:?}",
                    context.take_pending_errors()
                );
                for (name, prototype) in &expected {
                    let binding = unit
                        .bindings
                        .iter()
                        .find(|binding| context.string_cache.at(binding.name.name) == *name)
                        .unwrap_or_else(|| panic!("{name} is bound"));
                    let descriptor = binding.origin.descriptor().unwrap();
                    assert_eq!(descriptor.spelling, *name);
                    assert_eq!(descriptor.operation, BuiltinOperation::Library);
                    assert_eq!(
                        binding.origin.library(),
                        Some(name.trim_start_matches("__builtin_"))
                    );
                    assert_eq!(
                        &spell(unit, context, binding.ty),
                        prototype,
                        "{target:?} {name}"
                    );
                }
                let mut calls = 0;
                for info in unit.expressions {
                    let ExpressionType::Call { arguments, .. } = info.expression.kind else {
                        continue;
                    };
                    let Some(facts) = unit.builtin_call_facts(info.expression) else {
                        continue;
                    };
                    if facts.operation != BuiltinOperation::Library {
                        continue;
                    }
                    calls += 1;
                    // Every written operand is evaluated exactly once, as an
                    // ordinary prototyped call's operand.
                    assert_eq!(facts.operands.len(), arguments.len());
                    assert!(facts.operands.iter().all(|operand| {
                        operand.evaluation == crate::builtins::ArgumentEvaluation::Value
                    }));
                    assert_eq!(facts.effects, BuiltinEffectClass::Library);
                    assert!(
                        unit.effect_summary(info.expression, EffectContext::Value)
                            .unwrap()
                            .contains(EffectSet::CALL)
                    );
                }
                assert_eq!(calls, expected.len());
            },
        );
    }
}

/// Clang rejects a checked stream builtin until `FILE` is declared, and
/// checks every operand against the prototype.
#[test]
fn checked_builtins_reject_missing_file_and_mismatched_operands() {
    let diagnose = |source: &str| {
        let mut kinds = Vec::new();
        let mut errors = 0;
        with_source(source, |context, unit| {
            errors = unit.errors;
            for error in context.take_pending_errors() {
                if let TranslationError::Semantic(error) = error {
                    kinds.push(error.kind);
                }
            }
        });
        (errors, kinds)
    };
    let (errors, kinds) = diagnose(
        "int f(void *p, __builtin_va_list ap) { return __builtin___vfprintf_chk(p, 1, \"x\", ap); \
         }",
    );
    assert_eq!(errors, 1, "{kinds:?}");
    assert!(kinds.contains(&SemanticErrorKind::BuiltinRequiresHeader));
    for call in [
        // An int is no list, and a list is no format.
        "__builtin___vsprintf_chk(b, 1, 4, \"x\", 5)",
        "__builtin___vsprintf_chk(b, 1, 4, ap, ap)",
        // A FILE pointer of another type, and a missing or extra size.
        "__builtin___vfprintf_chk((int *)b, 1, \"x\", ap)",
        "__builtin___memcpy_chk(b, b, 4)",
        "__builtin___strcpy_chk(b, b, 4, 4)",
    ] {
        let source = format!(
            "typedef struct stream FILE; int f(char *b, __builtin_va_list ap) {{ return {call} != \
             0; }}"
        );
        let (errors, kinds) = diagnose(&source);
        assert!(errors > 0, "{call}: {kinds:?}");
    }
}

/// Clang forms the stream prototype from file scope, independently of local
/// typedefs with the same spelling (C99 §6.2.1p4, p. 29; PDF p. 41).
#[test]
fn checked_stream_builtins_use_file_scope_typedef() {
    for target in [
        Target::LinuxGnu,
        Target::LinuxMusl,
        Target::WindowsGnu,
        Target::WindowsMsvc,
    ] {
        for (source, missing_header, valid) in [
            (
                "typedef struct stream FILE; int f(FILE *p, __builtin_va_list ap) { \
                 typedef int FILE; return __builtin___vfprintf_chk(p, 1, \"x\", ap); }",
                false,
                true,
            ),
            (
                "int f(void *p, __builtin_va_list ap) { typedef int FILE; \
                 return __builtin___vfprintf_chk(p, 1, \"x\", ap); }",
                true,
                false,
            ),
            (
                "typedef struct stream FILE; int f(int *p, __builtin_va_list ap) { \
                 typedef int FILE; return __builtin___vfprintf_chk(p, 1, \"x\", ap); }",
                false,
                false,
            ),
        ] {
            with_configuration(
                source,
                CompilerConfiguration::default().with_target(target),
                |context, unit| {
                    let kinds: Vec<_> = context
                        .take_pending_errors()
                        .into_iter()
                        .filter_map(|error| match error {
                            | TranslationError::Semantic(error) => Some(error.kind),
                            | _ => None,
                        })
                        .collect();
                    assert_eq!(unit.errors == 0, valid, "{target:?}: {source}: {kinds:?}");
                    assert_eq!(
                        kinds.contains(&SemanticErrorKind::BuiltinRequiresHeader),
                        missing_header,
                        "{target:?}: {source}: {kinds:?}"
                    );
                    if valid {
                        let binding = unit.bindings.iter().find(|binding| {
                            context.string_cache.at(binding.name.name)
                                == "__builtin___vfprintf_chk"
                        }).unwrap();
                        assert!(
                            spell(unit, context, binding.ty).starts_with("int (stream *,"),
                            "{target:?}: {}",
                            spell(unit, context, binding.ty)
                        );
                    }
                },
            );
        }
    }
}
