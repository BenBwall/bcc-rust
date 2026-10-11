pub(super) mod atomics;
mod intrinsics;
mod names;
mod x86_builtin_table;
pub(crate) mod x86_builtins;

pub(crate) use modeled as implemented_builtin;
pub(crate) use names::named_builtin;

use super::{
    Analyzer,
    ArenaList,
    ArenaVec,
    ArrayBound,
    Cell,
    ConstantClass,
    Expression,
    ExpressionInfo,
    ExpressionType,
    Identifier,
    Integer,
    Scalar,
    SemanticErrorKind,
    SourceVectors,
    SyntaxOperand,
    Tag,
    TagKind,
    TypeId,
    TypeKind,
    TypeQualifiers,
    expressions,
};
use crate::translation_phases::preprocessing::KeywordTokenType;

/// Intrinsics supporting standard headers are available even in strict modes.
/// GCC/Clang implementation keywords extending C99 §7.15, pp. 249-252;
/// PDF pp. 261-264 and §7.17p3, p. 254; PDF p. 266.
pub(crate) fn modeled(keyword: KeywordTokenType) -> bool {
    matches!(
        keyword,
        KeywordTokenType::BuiltinVaArg
            | KeywordTokenType::BuiltinVaStart
            | KeywordTokenType::BuiltinVaEnd
            | KeywordTokenType::BuiltinVaCopy
            | KeywordTokenType::BuiltinTypesCompatible
            | KeywordTokenType::BuiltinChooseExpr
            | KeywordTokenType::BuiltinBitCast
            | KeywordTokenType::BuiltinConvertVector
            | KeywordTokenType::BuiltinOffsetof
    )
}
