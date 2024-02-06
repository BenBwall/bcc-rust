use std::sync::Arc;

use super::SourceVectors;
use crate::util::string_cache::Id as StringCacheId;

pub(crate) struct Parser<Prev> {
    pub(crate) previous_phase: Prev,
    pub(crate) state:          State,
    pub(crate) types:          Vec<Type>,
}

pub(crate) enum State {
    Default,
    Done,
}

pub(crate) struct TopLevelState {
    pub(crate) source_vectors: SourceVectors,
    pub(crate) kind:           TopLevelStatementType,
}

pub(crate) enum TopLevelStatementType {
    FunctionDeclaration(FunctionDeclaration),
    FunctionDefinition(FunctionDefinition),
    VariableDeclaration(Variable),
    VariableDefinition(VariableDefinition),
}

pub(crate) struct Identifier {
    pub(crate) name: StringCacheId,
}

pub(crate) struct Variable {
    pub(crate) name:          Identifier,
    pub(crate) type_:         Type,
    pub(crate) storage_class: StorageClass,
}

pub(crate) struct VariableDefinition {
    pub(crate) variable: Variable,
    pub(crate) initializer: Expression,
}

pub(crate) enum StorageClass {
    Auto,
    Register,
    Static,
    Extern,
    Typedef,
}

pub(crate) enum PrimitiveType {
    Char,
    Short,
    Int,
    Long,
    LongLong,
    UnsignedChar,
    UnsignedShort,
    UnsignedInt,
    UnsignedLong,
    UnsignedLongLong,
    Float,
    Double,
    LongDouble,
    Void,
    Bool,
}

pub(crate) struct Type {
    is_const:    bool,
    is_volatile: bool,
    kind:        TypeKind,
}

pub(crate) enum TypeKind {
    Primitive(PrimitiveType),
    Pointer {
        pointee_index: usize,
    },
    Struct {
        name:   Identifier,
        fields: Arc<[Variable]>,
    },
    Union {
        name:   Identifier,
        fields: Arc<[Variable]>,
    },
    Typedef {
        name:           Identifier,
        referent_index: usize,
    },
    Enum {
        name:   Identifier,
        values: Arc<[EnumValue]>,
    },
}

pub(crate) struct EnumValue {
    pub(crate) name:  Identifier,
    pub(crate) value: i64,
}

pub(crate) struct FunctionDeclaration {
    pub(crate) name:        Identifier,
    pub(crate) parameters:  Arc<[Variable]>,
    pub(crate) return_type: Option<Type>,
}

pub(crate) struct FunctionDefinition {
    pub(crate) declaration: FunctionDeclaration,
    pub(crate) body:        Vec<Statement>,
}

impl<Prev, PrevError> Iterator for Parser<Prev>
where
    Prev: TranslationPhase<Yield = Token, Error = PrevError>,
{
    type Item = TopLevelStatement;
}
