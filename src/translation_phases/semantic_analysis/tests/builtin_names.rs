//! Exhaustive builtin name recognition, pinned by a snapshot.
//!
//! Every spelling the compiler has ever recognized stays in
//! `tests/fixtures/semantic/builtin-names.snap`: its `__has_builtin` answer in
//! four configurations, and, for a spelling that is not a keyword, the
//! origin and type of the binding an undeclared call creates and the
//! diagnostics of a call without arguments. The candidate set is the
//! snapshot's own names plus every registry spelling, so moving a family
//! between recognizers cannot silently drop or change a name.
//! GCC/Clang extensions to C99 §6.5.2.2, pp. 71-72; PDF pp. 83-84 and
//! §6.10.1p4, p. 148; PDF p. 160 (`__has_builtin`).
use std::{
    collections::BTreeSet,
    fmt::Write as _,
};

use super::*;
use crate::{
    configuration::{
        CompilerConfiguration,
        ExtensionPolicy,
    },
    target::Target,
    translation_phases::preprocessing::KeywordTokenType,
};

const SNAPSHOT: &str = "tests/fixtures/semantic/builtin-names.snap";

/// Spellings outside the registry that the snapshot must keep probing even
/// before it exists: unregistered families and deliberate negatives.
const PROBES: &[&str] = &[
    "__builtin_M6_unknown",
    "__builtin_ia32_M6_unknown",
    "__atomic_M6_unknown",
    "__sync_M6_unknown",
    "__builtin_tgmath",
    "__cpuidex",
    "alloca",
];

#[test]
fn builtin_names_match_the_recognition_snapshot() {
    let mut names = BTreeSet::new();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(SNAPSHOT);
    if let Ok(previous) = std::fs::read_to_string(path) {
        names.extend(
            previous
                .lines()
                .filter_map(|line| line.split_whitespace().next())
                .map(str::to_owned),
        );
    }
    names.extend(PROBES.iter().map(|&name| name.to_owned()));
    names.extend(baseline_names());
    names.extend(
        crate::builtins::test_descriptors().map(|descriptor| descriptor.spelling.to_owned()),
    );
    names.extend(
        KeywordTokenType::ALL
            .iter()
            .map(|keyword| keyword.spelling())
            .filter(|spelling| spelling.starts_with("__builtin_"))
            .map(str::to_owned),
    );
    let names: Vec<_> = names.into_iter().collect();
    let configurations = [
        CompilerConfiguration::default(),
        CompilerConfiguration::default().with_gnu_extensions(true),
        CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Deny),
        CompilerConfiguration::default()
            .with_target(Target::WindowsMsvc)
            .with_msvc_extensions(true),
    ];
    let advertised: Vec<_> = configurations
        .iter()
        .map(|&configuration| has_builtin(&names, configuration))
        .collect();
    let mut snapshot = String::new();
    for (index, name) in names.iter().enumerate() {
        let bits: String = advertised
            .iter()
            .map(|set| if set.contains(&index) { '1' } else { '0' })
            .collect();
        write!(snapshot, "{name} has={bits}").unwrap();
        if KeywordTokenType::ALL
            .iter()
            .any(|keyword| keyword.spelling() == name)
        {
            snapshot.push_str(" keyword\n");
            continue;
        }
        writeln!(snapshot, " {}", call(name)).unwrap();
    }
    crate::test_support::assert_snapshot(SNAPSHOT, &snapshot);
}

/// Indexes of `names` advertised by `__has_builtin` under `configuration`.
fn has_builtin(names: &[String], configuration: CompilerConfiguration) -> BTreeSet<usize> {
    let mut source = String::new();
    for (index, name) in names.iter().enumerate() {
        writeln!(
            source,
            "#if __has_builtin({name})\nint advertised_{index};\n#endif"
        )
        .unwrap();
    }
    let mut set = BTreeSet::new();
    with_configuration(&source, configuration, |context, unit| {
        for binding in unit.bindings {
            if let Some(index) = context
                .string_cache
                .at(binding.name.name)
                .strip_prefix("advertised_")
            {
                _ = set.insert(index.parse().unwrap());
            }
        }
    });
    set
}

/// The binding an undeclared call creates and the call's diagnostics.
fn call(name: &str) -> String {
    let mut row = String::new();
    with_configuration(
        &format!("void f(void){{(void){name}();}}"),
        CompilerConfiguration::default().with_gnu_extensions(true),
        |context, unit| {
            let binding = unit
                .bindings
                .iter()
                .find(|binding| context.string_cache.at(binding.name.name) == name);
            match binding {
                | Some(binding) => write!(
                    row,
                    "{} {}",
                    match binding.origin {
                        | BindingOrigin::User => "user",
                        | BindingOrigin::ImplicitOrdinary => "implicit",
                        | origin if origin.is_intrinsic() => "intrinsic",
                        | _ => unreachable!(),
                    },
                    spell(unit, context, binding.ty)
                )
                .unwrap(),
                | None => row.push_str("unbound"),
            }
            let mut other = 0;
            let mut kinds = Vec::new();
            for error in context.take_pending_errors() {
                match error {
                    | TranslationError::Semantic(error) => kinds.push(format!("{:?}", error.kind)),
                    | _ => other += 1,
                }
            }
            write!(row, " [{}] +{other}", kinds.join(",")).unwrap();
        },
    );
    row
}

/// A type as a compact C-like string; vectors as `V<lanes>(element)`.
pub(super) fn spell(
    unit: &SemanticTranslationUnit<'_>,
    context: &Context<'_>,
    ty: TypeId,
) -> String {
    let constant = if ty.qualifiers.contains(TypeQualifiers::CONST) {
        "const "
    } else {
        ""
    };
    let body = match unit.types.nodes[ty.index] {
        | TypeKind::Unknown => "?".to_owned(),
        | TypeKind::Unmodeled(kind) => format!("unmodeled({kind:?})"),
        | TypeKind::Scalar(scalar) => scalar.spelling().to_owned(),
        | TypeKind::Pointer(target) => format!("{}*", spell(unit, context, target)),
        | TypeKind::Vector { element, count, .. } =>
            format!("V{count}({})", spell(unit, context, element)),
        | TypeKind::Tag(tag) => unit.types.tags[tag]
            .name
            .map_or("<tag>", |name| context.string_cache.at(name))
            .to_owned(),
        | TypeKind::Function {
            result,
            parameters,
            prototype,
            variadic,
        } => {
            let parameters: Vec<_> = parameters
                .iter()
                .map(|&parameter| spell(unit, context, parameter))
                .collect();
            format!(
                "{}({}{}{})",
                spell(unit, context, result),
                parameters.join(","),
                if variadic { ",..." } else { "" },
                if prototype { "" } else { "unprototyped" }
            )
        },
        | kind => format!("{kind:?}"),
    };
    format!("{constant}{body}")
}

/// Families recognized outside the registry before M6. The snapshot
/// retains them once blessed.
fn baseline_names() -> impl Iterator<Item = String> {
    include_str!("../builtins/x86_builtin_table.rs")
        .lines()
        .filter_map(|line| line.trim().strip_prefix("(\""))
        .filter_map(|line| line.split('"').next())
        .chain([
            "__builtin_classify_type",
            "__builtin_shufflevector",
            "__builtin_elementwise_abs",
            "__builtin_elementwise_sqrt",
            "__builtin_elementwise_add_sat",
            "__builtin_elementwise_sub_sat",
            "__builtin_elementwise_min",
            "__builtin_elementwise_max",
            "__builtin_nondeterministic_value",
            "__builtin_nontemporal_load",
            "__builtin_nontemporal_store",
            "__builtin_nans",
            "__builtin_huge_valf128",
            "__builtin_inff128",
            "__builtin_nanf128",
            "__builtin_nansf128",
            "__builtin_fabsf128",
            "__builtin_copysignf128",
        ])
        .map(str::to_owned)
}
