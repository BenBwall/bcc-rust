//! Phase-7 constraints and source-backed semantic diagnostics.
//! C99: diagnostic requirement §5.1.1.3p1, p. 11; PDF p. 23.

use std::fmt;

use crate::{
    diagnostics::{
        Diagnostic,
        Explanation,
        ToDiagnostic,
        format_in,
        quote_spelling,
    },
    translation_phases::{
        Context,
        ErrorSeverity,
        GetPosition,
        GetSeverity,
        GetSourceVectors,
        SourcePosition,
        SourceVectors,
    },
    util::{
        bump::Bump,
        string_cache::StringCacheId,
    },
};

/// Symbolic semantic diagnostic, independent of rendered text.
/// C99: §6.7p3-4, p. 97; PDF p. 109; subsidiary constraints are cited below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SemanticErrorKind {
    /// C99: §7.15p3, p. 249; PDF p. 261.
    InvalidVaList,
    /// C99: §7.15.1.1p2, pp. 249-250; PDF pp. 261-262.
    InvalidVaArgType,
    /// C99: §7.15.1.4p4, p. 251; PDF p. 263; diagnosed UB.
    VaStartOutsideVariadic,
    /// C99: §7.17p3, p. 254; PDF p. 266.
    InvalidOffsetof,
    /// C99: §6.9.1p2, p. 141; PDF p. 153.
    InvalidFunctionDefinition,
    /// C99: §6.9.1p4, p. 141; PDF p. 153.
    FunctionDefinitionStorage,
    /// C99: §6.9.1p3, p. 141; PDF p. 153.
    IncompleteFunctionReturn,
    /// C99: §6.9.1p5-6, p. 141; PDF p. 153.
    InvalidDefinitionParameterList,
    /// C99: §6.9.1p5, p. 141; PDF p. 153.
    UnnamedDefinitionParameter,
    /// C99: §6.7.5.3p4, p. 118; PDF p. 130.
    IncompleteDefinitionParameter,
    /// C99: §6.7.5.2p4, p. 117; PDF p. 129.
    DefinitionStarArray,
    /// C99: §5.1.2.2.1p1, p. 12; PDF p. 24.
    MainSignature,
    /// C99: §6.9.2p3, p. 143; PDF p. 155.
    IncompleteInternalTentative,
    /// C99: §6.9p3,p5, p. 140; PDF p. 152.
    DuplicateDefinition,
    /// C99: §6.9.2p2, p. 143; PDF p. 155.
    TentativeArrayAssumedOne,
    /// C99: §6.9.2p2, p. 143; PDF p. 155.
    IncompleteTentativeDefinition,
    /// C99: §6.9p3, p. 140; PDF p. 152.
    UndefinedInternal,
    /// C99: §6.9p3, p. 140; PDF p. 152.
    UnusedStaticFunction,
    /// C99: §6.7.4p3, p. 112; PDF p. 124.
    InlineInternalReference,
    /// C99: §6.7.4p3, p. 112; PDF p. 124.
    InlineStaticObject,
    /// C99: §6.8.6.4p1, p. 139; PDF p. 151.
    VoidReturnValue,
    /// C99: §6.8.6.4p1, p. 139; PDF p. 151.
    MissingReturnValue,
    /// C99: §6.8.6.4p1, p. 139; PDF p. 151.
    MissingReturnValueWarning,
    /// C99: §6.8.6.4p3, p. 139; PDF p. 151.
    InvalidReturnConversion,
    /// C99: §6.8.5p3, p. 135; PDF p. 147.
    InvalidForDeclaration,
    /// C99: §6.8.1p3, p. 131; PDF p. 143.
    DuplicateLabel,
    /// C99: §6.8.6.1p1, p. 137; PDF p. 149.
    UndefinedLabel,
    /// GNU extension: local labels have lexical scope, unlike the function
    /// scope in C99 §6.2.1p3, p. 29; PDF p. 41.
    DuplicateLocalLabel,
    /// GNU extension: a local-label declaration requires a definition.
    /// C99 labels are implicit definitions: §6.2.1p3, p. 29; PDF p. 41.
    UndefinedLocalLabel,
    /// C99: §6.8.6.1p1, p. 137; PDF p. 149.
    JumpIntoVariableScope,
    /// C99: §6.8.4.2p2, p. 134; PDF p. 146.
    SwitchIntoVariableScope,
    /// C99: §6.8.1p2, p. 131; PDF p. 143.
    CaseOutsideSwitch,
    /// C99: §6.8.4.2p3, p. 134; PDF p. 146.
    EmptyCaseRange,
    /// C99: §6.8.6.3p1, p. 138; PDF p. 150.
    BreakOutsideLoopOrSwitch,
    /// C99: §6.8.6.2p1, p. 138; PDF p. 150.
    ContinueOutsideLoop,
    /// C99: §6.8.4.2p3, p. 134; PDF p. 146.
    DuplicateCase,

    /// C99: §6.7.7, p. 123-124; PDF p. 135-136.
    UnknownTypedef,
    /// C99: §6.5.1p2, p. 69; PDF p. 81.
    UndeclaredIdentifier,
    /// C99: §6.5.3.2p1, p. 78; PDF p. 90.
    InvalidAddressOperand,
    /// C99: §6.5.3p1, p. 78; PDF p. 90.
    InvalidUnaryOperand,
    /// C99: §6.5.16p2, p. 91; PDF p. 103.
    ExpectedModifiableLvalue,
    /// C99: §6.5.2.1p1, p. 70; PDF p. 82.
    InvalidSubscript,
    /// C99: §6.5.5p2, p. 82; PDF p. 94.
    InvalidArithmeticOperands,
    /// C99: §6.5.6p2-3, p. 82; PDF p. 94.
    InvalidAdditiveOperands,
    /// C99: §6.5.7p2, p. 84; PDF p. 96.
    InvalidIntegerOperands,
    /// C99: §6.5.13p2, p. 89; PDF p. 101.
    InvalidLogicalOperands,
    /// C99: §6.5.8p2, p. 85; PDF p. 97.
    InvalidComparisonOperands,
    /// C99: §6.5.15p2-3, p. 90; PDF p. 102.
    InvalidConditionalOperands,
    /// C99: §6.5.16.1p1, p. 92; PDF p. 104.
    InvalidAssignment,
    /// C99: §6.5.4p2-4, p. 81; PDF p. 93.
    InvalidCast,
    /// C99: §6.5.3.4p1, p. 80; PDF p. 92.
    InvalidSizeof,
    /// C99: §6.5.2.3p1-2, p. 72; PDF p. 84.
    InvalidMemberAccess,
    /// C99: §6.5.2.2p1, p. 71; PDF p. 83.
    InvalidCall,
    /// C99: §6.5.2.2p2, p. 71; PDF p. 83.
    InvalidArgumentCount,
    /// C99: §6.5.2.2p2, p. 71; PDF p. 83.
    InvalidArgumentType,
    /// C99: §6.5.2.5p1, p. 75; PDF p. 87.
    InvalidCompoundLiteral,
    /// C99: §6.8.4.1p1, p. 133; PDF p. 145.
    InvalidCondition,
    /// C99: §6.8.4.2p1, p. 134; PDF p. 146.
    InvalidSwitchExpression,
    /// C11: §6.7.10p2-3, p. 145; PDF p. 163. C99 ICE evaluation:
    /// §6.6p6, p. 95; PDF p. 107.
    FailedAssertion,
    /// C99: §6.7.8p2-3, p. 125; PDF p. 137.
    InvalidInitializer,
    /// C99: §6.7.8p2, p. 125; PDF p. 137.
    ExcessInitializer,
    /// C99: §6.7.8p6-7, p. 125; PDF p. 137.
    InvalidDesignator,
    /// C99: §6.7.8p4, p. 125; PDF p. 137.
    NonConstantInitializer,
    /// C99: §6.9p2, p. 140; PDF p. 152.
    InvalidStorage,
    /// C99: §6.7.1p5, p. 98; PDF p. 110.
    InvalidFunctionStorage,
    /// C99: §6.7.8p5, p. 125; PDF p. 137.
    LinkedBlockInitializer,
    /// C99: §6.7.3p2, p. 108; PDF p. 120.
    InvalidRestrict,
    /// C99: §6.7.3p8, p. 109; PDF p. 121. Qualifying a function type is
    /// undefined.
    QualifiedFunction,
    /// C99: §6.7.4p2-4, p. 112; PDF p. 124.
    InvalidInline,
    /// C99: §6.7.5.2p1; 6.7.5.3p1, p. 116-118; PDF p. 128-130.
    InvalidDerivedType,
    /// C99: §6.7.5.2p1, p. 116; PDF p. 128.
    InvalidArrayBound,
    /// C99: §6.7.5.2p4, pp. 116-117; PDF pp. 128-129. Function definitions
    /// report `DefinitionStarArray` instead.
    InvalidStarBound,
    /// Implementation limit, C99: §5.2.4.1p1, pp. 20-21; PDF pp. 32-33;
    /// §6.5.6p9, pp. 83-84; PDF pp. 95-96.
    ObjectTooLarge,
    /// C99: §6.7.5.2p2, p. 116; PDF p. 128; members §6.7.2.1p8, p. 102;
    /// PDF p. 114.
    FileScopeVariableType,
    /// C99: §6.7p4; 6.2.7p2, p. 97; 40; PDF p. 109; 52.
    IncompatibleDeclaration,
    /// C99: §6.7p3, p. 97; PDF p. 109.
    DuplicateDeclaration,
    /// C99: §6.2.2p4-7, p. 30-31; PDF p. 42-43.
    ConflictingLinkage,
    /// C99: §6.7.2.3p2, p. 106; PDF p. 118.
    TagKindMismatch,
    /// C99: §6.7.2.3p1, p. 106; PDF p. 118.
    TagRedefinition,
    /// C99: §6.7.2.3p3, p. 106; PDF p. 118.
    IncompleteEnum,
    /// C99: §6.6p3-6, p. 95; PDF p. 107.
    InvalidConstant,
    /// C99: §6.6p4; 6.5p5, p. 95; 67; PDF p. 107; 79.
    ConstantOverflow,
    /// C99: §6.7.2.2p4, p. 105; PDF p. 117. Values outside int that share
    /// one 64-bit type are the policy extension of §6.7.2.2p2.
    EnumeratorRange,
    /// C99: §6.7.2.1p2, p. 101; PDF p. 113.
    InvalidMember,
    /// C99: §6.7p3; 6.2.3p1, p. 97; 31; PDF p. 109; 43.
    DuplicateMember,
    /// C99: §6.7.2.1p4, p. 101; PDF p. 113.
    InvalidBitFieldType,
    /// C99: §6.7.2.1p3, p. 101; PDF p. 113.
    InvalidBitFieldWidth,
    /// C99: §6.7.2.1p3, p. 101; PDF p. 113.
    NamedZeroWidthBitField,
    /// C99: §6.7.5.2p1; §6.7.5.3p2 and p10, pp. 116-119; PDF pp. 128-131.
    InvalidParameter,
    /// C99: §6.7p7, p. 98; PDF p. 110.
    IncompleteObject,
    /// C11: §6.7.2.4p3, p. 121; PDF p. 139; §6.7.3p3.
    InvalidAtomicType,
    /// C11: §6.5.1.1p2, p. 78; PDF p. 96.
    InvalidGenericSelection,
    /// Clang/GCC type-generic atomic builtin constraints.
    InvalidAtomicOperand,
    /// Clang diagnoses invalid constant orders as warnings.
    InvalidAtomicOrder,
    /// Compare-exchange failure order cannot release.
    InvalidAtomicFailureOrder,
    /// Clang warns when expected/output buffers discard qualifiers.
    AtomicBufferQualifiers,
    /// GCC Additional Floating Types, target availability.
    UnsupportedFloat128,
    /// GCC Other Builtins: `choose_expr` requires an integer constant.
    InvalidChooseCondition,
}

impl SemanticErrorKind {
    /// The primary label names what is wrong at the labeled source.
    const fn label(self) -> &'static str {
        match self {
            | Self::UnsupportedFloat128 => "unsupported floating type",
            | Self::UnknownTypedef => "not a typedef name in this scope",
            | Self::UndeclaredIdentifier => "not declared",
            | Self::InvalidAddressOperand => "cannot take this address",
            | Self::InvalidUnaryOperand | Self::InvalidAdditiveOperands =>
                "operand type is not allowed here",
            | Self::ExpectedModifiableLvalue => "not a modifiable lvalue",
            | Self::InvalidSubscript => "invalid subscript operands",
            | Self::InvalidArithmeticOperands => "operands are not arithmetic",
            | Self::InvalidIntegerOperands => "operands are not integers",
            | Self::InvalidLogicalOperands => "operands are not scalar",
            | Self::InvalidComparisonOperands => "operands cannot be compared",
            | Self::InvalidConditionalOperands => "operands do not have a common type",
            | Self::InvalidAssignment => "value cannot be assigned to this type",
            | Self::InvalidCast => "invalid conversion",
            | Self::InvalidSizeof => "operand has no size",
            | Self::InvalidMemberAccess => "no such member",
            | Self::InvalidCall => "not a callable function",
            | Self::InvalidArgumentCount => "wrong number of arguments",
            | Self::InvalidArgumentType => "argument does not convert to the parameter type",
            | Self::InvalidCompoundLiteral => "invalid compound literal type",
            | Self::InvalidCondition => "condition is not scalar",
            | Self::InvalidSwitchExpression => "controlling expression is not an integer",
            | Self::FailedAssertion => "assertion is false",
            | Self::InvalidInitializer => "invalid initializer",
            | Self::ExcessInitializer => "no subobject is left to initialize",
            | Self::InvalidDesignator => "no such subobject",
            | Self::NonConstantInitializer => "not a constant expression",
            | Self::InvalidStorage => "external declaration",
            | Self::InvalidFunctionStorage => "function declared in a block",
            | Self::LinkedBlockInitializer => "initializer is not allowed here",
            | Self::InvalidRestrict => "restrict on a non-object pointer",
            | Self::QualifiedFunction => "function type qualifiers are ignored",
            | Self::InvalidInline => "inline applies only to functions other than main",
            | Self::InvalidDerivedType => "invalid element or return type",
            | Self::InvalidArrayBound => "invalid array size",
            | Self::InvalidStarBound => "`[*]` is not allowed here",
            | Self::ObjectTooLarge => "too large",
            | Self::FileScopeVariableType => "variably modified type",
            | Self::IncompatibleDeclaration => "conflicting type",
            | Self::DuplicateDeclaration => "declared again here",
            | Self::ConflictingLinkage => "linkage conflicts with the previous declaration",
            | Self::TagKindMismatch => "different tag kind",
            | Self::TagRedefinition | Self::DuplicateDefinition => "defined again here",
            | Self::IncompleteEnum => "enum is not yet complete",
            | Self::InvalidConstant | Self::InvalidChooseCondition =>
                "not an integer constant expression",
            | Self::ConstantOverflow => "overflow or invalid operation",
            | Self::EnumeratorRange => "no supported enum type holds this value with the others",
            | Self::InvalidMember => "incomplete member type",
            | Self::DuplicateMember => "member declared again here",
            | Self::InvalidBitFieldType => "not an integer type",
            | Self::InvalidBitFieldWidth => "invalid width",
            | Self::NamedZeroWidthBitField => "named zero-width bit-field",
            | Self::InvalidParameter => "invalid parameter declaration",
            | Self::IncompleteObject => "incomplete type",
            | Self::InvalidFunctionDefinition => "not a function declarator",
            | Self::FunctionDefinitionStorage => "storage class is not allowed on a definition",
            | Self::IncompleteFunctionReturn => "incomplete return type",
            | Self::InvalidDefinitionParameterList => "invalid parameter list for a definition",
            | Self::UnnamedDefinitionParameter => "parameter has no name",
            | Self::IncompleteDefinitionParameter => "incomplete parameter type",
            | Self::DefinitionStarArray => "`[*]` is not allowed in a definition",
            | Self::MainSignature => "nonportable signature for main",
            | Self::IncompleteInternalTentative | Self::IncompleteTentativeDefinition =>
                "still incomplete at the end of the translation unit",
            | Self::TentativeArrayAssumedOne => "array assumed to have one element",
            | Self::UndefinedInternal => "used here but never defined",
            | Self::UnusedStaticFunction => "never defined",
            | Self::InlineInternalReference => "internal identifier in an inline definition",
            | Self::InlineStaticObject => "modifiable static object in an inline definition",
            | Self::VoidReturnValue => "value returned from a void function",
            | Self::MissingReturnValue | Self::MissingReturnValueWarning => "no return value",
            | Self::InvalidReturnConversion => "value cannot be returned as the result type",
            | Self::InvalidForDeclaration => "not an automatic object",
            | Self::DuplicateLabel => "label defined again here",
            | Self::UndefinedLabel => "label is never defined",
            | Self::DuplicateLocalLabel => "local label declared again here",
            | Self::UndefinedLocalLabel => "local label is never defined",
            | Self::JumpIntoVariableScope | Self::SwitchIntoVariableScope =>
                "jumps into a variably modified scope",
            | Self::CaseOutsideSwitch => "not in a switch statement",
            | Self::EmptyCaseRange => "empty case range",
            | Self::BreakOutsideLoopOrSwitch => "not in a loop or switch",
            | Self::ContinueOutsideLoop => "not in a loop",
            | Self::DuplicateCase => "case value repeated here",
            | Self::InvalidVaList => "not a modifiable va_list",
            | Self::InvalidVaArgType => "not a complete object type",
            | Self::VaStartOutsideVariadic => "function is not variadic",
            | Self::InvalidOffsetof => "invalid member designator",
            | Self::InvalidAtomicType => "invalid atomic operand type",
            | Self::InvalidGenericSelection => "invalid generic association list",
            | Self::InvalidAtomicOperand => "invalid atomic builtin operand",
            | Self::InvalidAtomicOrder => "invalid memory order",
            | Self::InvalidAtomicFailureOrder => "invalid failure memory order",
            | Self::AtomicBufferQualifiers => "pointer conversion discards qualifiers",
        }
    }

    fn explanation(self) -> (&'static str, &'static str) {
        match self {
            | Self::AtomicBufferQualifiers => (
                "atomic buffer argument discards pointer target qualifiers",
                "Clang atomic builtins convert generic value, expected and output buffers to pointers to the unqualified value type",
            ),
            | Self::InvalidAtomicOperand => (
                "operand does not satisfy the atomic builtin type contract",
                "Clang C11 atomic builtins and GCC __atomic/__sync builtins: operations require eligible object pointers and compatible value or buffer operands (https://clang.llvm.org/docs/LanguageExtensions.html#c11-atomic-builtins)",
            ),
            | Self::InvalidAtomicOrder | Self::InvalidAtomicFailureOrder => (
                "memory order argument to atomic operation is invalid",
                "Clang/GCC atomic builtins: loads cannot release, stores cannot acquire, and compare-exchange failure cannot release (https://gcc.gnu.org/onlinedocs/gcc/_005f_005fatomic-Builtins.html)",
            ),
            | Self::InvalidGenericSelection => (
                "generic selection requires unique eligible associations and a matching type or default",
                "C11 §6.5.1.1p2: associations name complete non-variably-modified object types; compatible types and defaults cannot be repeated",
            ),
            | Self::InvalidAtomicType => (
                "atomic type requires an eligible complete object type",
                "C11 §6.7.2.4p3 and §6.7.3p3: arrays and functions cannot be atomic; an atomic type specifier also excludes qualified and atomic types",
            ),
            | Self::InvalidChooseCondition => (
                "__builtin_choose_expr requires an integer constant expression",
                "GCC Other Builtins: __builtin_choose_expr selects its second or third operand \
                 using an integer constant expression, preserving the selected type",
            ),
            | Self::UnsupportedFloat128 => (
                "__float128 is not supported on this target",
                "GCC Additional Floating Types: __float128 requires target binary128 support; \
                 Clang rejects it on x86_64-pc-windows-msvc",
            ),
            | Self::InvalidFunctionDefinition => (
                "definition requires a function declarator",
                "C99 §6.9.1p2: the function type must be specified by the declarator, not solely \
                 by a typedef",
            ),
            | Self::FunctionDefinitionStorage => (
                "invalid storage class in function definition",
                "C99 §6.9.1p4: a function definition permits only extern or static storage-class \
                 specifiers",
            ),
            | Self::IncompleteFunctionReturn => (
                "function definition requires a complete return object type",
                "C99 §6.9.1p3: a function returns void or an object type other than an array; its \
                 body requires a complete result type",
            ),
            | Self::InvalidDefinitionParameterList => (
                "invalid function definition parameter list",
                "C99 §6.9.1p5-6: prototype definitions have no declaration list; identifier-list \
                 declarations name only listed parameters, with no initializers",
            ),
            | Self::UnnamedDefinitionParameter => (
                "function definition parameter requires a name",
                "C99 §6.9.1p5: each prototype-definition parameter includes an identifier, except \
                 the sole void parameter",
            ),
            | Self::IncompleteDefinitionParameter => (
                "function definition parameter has incomplete type",
                "C99 §6.7.5.3p4: after adjustment, each prototype-definition parameter has \
                 complete object type",
            ),
            | Self::DefinitionStarArray => (
                "star array bound is not permitted in a function definition",
                "C99 §6.7.5.2p4: an unspecified variable-length array bound written as [*] is \
                 restricted to prototype scope",
            ),
            | Self::MainSignature => (
                "main has a nonportable signature",
                "C99 §5.1.2.2.1p1: hosted main returns int and accepts no parameters or int and \
                 char **, or an implementation-defined form",
            ),
            | Self::IncompleteInternalTentative => (
                "internal tentative definition has incomplete type",
                "C99 §6.9.2p3: a tentative definition with internal linkage shall not have an \
                 incomplete type",
            ),
            | Self::DuplicateDefinition => (
                "identifier is defined more than once",
                "C99 §6.9p3,p5: an identifier with linkage has at most one external definition in \
                 a translation unit",
            ),
            | Self::TentativeArrayAssumedOne => (
                "tentative array definition is assumed to have one element",
                "C99 §6.9.2p2: an incomplete tentative array without an external definition is \
                 completed at translation-unit end (see example 2)",
            ),
            | Self::IncompleteTentativeDefinition => (
                "tentative definition remains incomplete",
                "C99 §6.9.2p2: a tentative definition without an external definition behaves as a \
                 zero-initialized file-scope definition",
            ),
            | Self::UndefinedInternal => (
                "used internal identifier has no definition",
                "C99 §6.9p3: an internal-linkage identifier used outside a constant sizeof \
                 operand has exactly one definition in the translation unit",
            ),
            | Self::UnusedStaticFunction => (
                "static function is declared but never defined",
                "C99 §6.9p3: an unused internal function declaration need not be defined; this \
                 warning identifies a likely missing body",
            ),
            | Self::InlineInternalReference => (
                "inline definition references an internal-linkage identifier",
                "C99 §6.7.4p3: an inline definition with external linkage contains no reference \
                 to an identifier with internal linkage",
            ),
            | Self::InlineStaticObject => (
                "inline definition defines a modifiable static-storage object",
                "C99 §6.7.4p3: an inline definition with external linkage contains no definition \
                 of a modifiable object with static storage duration",
            ),
            | Self::VoidReturnValue => (
                "void function returns an expression",
                "C99 §6.8.6.4p1: a return statement with an expression shall not appear in a \
                 function returning void",
            ),
            | Self::MissingReturnValue => (
                "non-void function returns without a value",
                "C99 §6.8.6.4p1: a return statement without an expression appears only in a \
                 function returning void",
            ),
            | Self::MissingReturnValueWarning => (
                "non-void function returns without a value",
                "C99 §6.8.6.4p1: C99 requires a value; C89 and GNU modes retain this form with a \
                 warning",
            ),
            | Self::InvalidReturnConversion => (
                "return expression cannot be converted to the function result type",
                "C99 §6.8.6.4p3: the return expression is converted as if assigned to an object \
                 with the function return type",
            ),
            | Self::InvalidForDeclaration => (
                "for initializer declaration permits only automatic objects",
                "C99 §6.8.5p3: the declaration part of a for statement declares only objects with \
                 auto or register storage",
            ),
            | Self::DuplicateLabel => (
                "label is defined more than once",
                "C99 §6.8.1p3: labels occupy a function-wide namespace and each label is unique \
                 within its function",
            ),
            | Self::UndefinedLabel => (
                "goto target label is not defined",
                "C99 §6.8.6.1p1: a goto identifier names a label somewhere in the enclosing \
                 function",
            ),
            | Self::DuplicateLocalLabel => (
                "local label is declared more than once in the same scope",
                "GNU local labels require distinct declarations in a scope; C99 §6.2.1p3 gives \
                 ordinary labels function scope",
            ),
            | Self::UndefinedLocalLabel => (
                "declared local label is not defined",
                "GNU local-label declarations require a definition even without a goto; C99 \
                 §6.2.1p3 declares ordinary labels by their definitions",
            ),
            | Self::JumpIntoVariableScope => (
                "goto enters the scope of a variably modified identifier",
                "C99 §6.8.6.1p1: a goto cannot jump from outside the scope of a variably modified \
                 identifier to inside that scope",
            ),
            | Self::SwitchIntoVariableScope => (
                "switch dispatch enters the scope of a variably modified identifier",
                "C99 §6.8.4.2p2: the entire switch is within the scope of every variably modified \
                 identifier whose scope contains a case or default label",
            ),
            | Self::CaseOutsideSwitch => (
                "case or default label is outside a switch",
                "C99 §6.8.1p2: a case or default label appears only within a switch statement",
            ),
            | Self::EmptyCaseRange => (
                "case range is empty after conversion",
                "C99 §6.8.4.2p3: GNU extension: inclusive case ranges with a lower value greater \
                 than the upper value match nothing",
            ),
            | Self::BreakOutsideLoopOrSwitch => (
                "break is outside an iteration or switch statement",
                "C99 §6.8.6.3p1: a break statement appears only in or as a loop or switch body",
            ),
            | Self::ContinueOutsideLoop => (
                "continue is outside an iteration statement",
                "C99 §6.8.6.2p1: a continue statement appears only in or as a loop body",
            ),
            | Self::DuplicateCase => (
                "case values overlap after conversion",
                "C99 §6.8.4.2p3: case constants are converted to the promoted switch type and no \
                 two case values may be equal",
            ),
            | Self::UndeclaredIdentifier => (
                "identifier has no visible declaration",
                "C99 §6.5.1p2: an identifier expression designates a declared object, function or \
                 enumeration constant",
            ),
            | Self::InvalidAddressOperand => (
                "address operator requires an addressable object or function",
                "C99 §6.5.3.2p1: the operand is a function designator, subscript, indirection, or \
                 a non-register, non-bit-field lvalue",
            ),
            | Self::InvalidUnaryOperand => (
                "invalid operand type for unary operator",
                "C99 §6.5.3p1: unary operators require their specified arithmetic, scalar or \
                 pointer operands",
            ),
            | Self::ExpectedModifiableLvalue => (
                "operator requires a modifiable lvalue",
                "C99 §6.5.16p2: assignment and increment operators require a modifiable lvalue",
            ),
            | Self::InvalidSubscript => (
                "subscript requires an integer and a pointer to complete object type",
                "C99 §6.5.2.1p1: one operand is integer and the other points to a complete object \
                 type",
            ),
            | Self::InvalidArithmeticOperands => (
                "multiplicative operator requires arithmetic operands",
                "C99 §6.5.5p2: multiplicative operators require arithmetic operands; remainder \
                 requires integers",
            ),
            | Self::InvalidAdditiveOperands => (
                "invalid operand types for pointer or arithmetic addition",
                "C99 §6.5.6p2-3: addition and subtraction require arithmetic operands or the \
                 specified complete-object pointer combinations",
            ),
            | Self::InvalidIntegerOperands => (
                "operator requires integer operands",
                "C99 §6.5.7p2: shift and bitwise operators require integer operands",
            ),
            | Self::InvalidLogicalOperands => (
                "logical operator requires scalar operands",
                "C99 §6.5.13p2: both operands of a logical operator have scalar type",
            ),
            | Self::InvalidComparisonOperands => (
                "invalid operand types for comparison",
                "C99 §6.5.8p2: comparisons require the specified arithmetic or compatible pointer \
                 operands",
            ),
            | Self::InvalidConditionalOperands => (
                "invalid operand types for conditional operator",
                "C99 §6.5.15p2-3: the condition is scalar and the alternatives have compatible \
                 arithmetic, aggregate, void or pointer types",
            ),
            | Self::InvalidAssignment => (
                "incompatible assignment conversion",
                "C99 §6.5.16.1p1: assignment requires compatible values and cannot discard \
                 pointer-target qualifiers",
            ),
            | Self::InvalidCast => (
                "invalid operand or target type for cast",
                "C99 §6.5.4p2-4: casts require scalar operands and targets except conversion to \
                 void; floating and pointer types cannot be interconverted",
            ),
            | Self::InvalidSizeof => (
                "size query requires a complete object type and no bit-field",
                "C99 §6.5.3.4p1: sizeof cannot be applied to function, incomplete or bit-field \
                 types",
            ),
            | Self::InvalidMemberAccess => (
                "member access requires a declared record member",
                "C99 §6.5.2.3p1-2: member access names a member of the designated structure or \
                 union",
            ),
            | Self::InvalidCall => (
                "call requires a function pointer returning void or complete object type",
                "C99 §6.5.2.2p1: the called expression points to a function with a valid return \
                 type",
            ),
            | Self::InvalidArgumentCount => (
                "function call has the wrong number of arguments",
                "C99 §6.5.2.2p2: a prototype fixes the required arguments; only an ellipsis \
                 permits additional arguments",
            ),
            | Self::InvalidArgumentType => (
                "function argument cannot be converted to its parameter type",
                "C99 §6.5.2.2p2: each argument has a type assignable to its corresponding \
                 parameter",
            ),
            | Self::InvalidCompoundLiteral => (
                "compound literal requires a non-variable object type",
                "C99 §6.5.2.5p1: a compound literal specifies an object type or an array of \
                 unknown size, not a variable-length array",
            ),
            | Self::InvalidCondition => (
                "condition requires scalar type",
                "C99 §6.8.4.1p1: selection and iteration conditions have scalar type",
            ),
            | Self::InvalidSwitchExpression => (
                "switch controlling expression requires integer type",
                "C99 §6.8.4.2p1: the controlling expression of a switch has integer type",
            ),
            | Self::FailedAssertion => (
                "static assertion failed",
                "C11 §6.7.10p2-3: the assertion requires a nonzero integer constant expression",
            ),
            | Self::InvalidInitializer => (
                "initializer cannot initialize this object",
                "C99 §6.7.8p2-3: initialization requires an appropriate complete object or \
                 incomplete array and compatible scalar values",
            ),
            | Self::ExcessInitializer => (
                "excess initializer for the current object",
                "C99 §6.7.8p2: an initializer shall not provide a value for an object outside the \
                 initialized entity",
            ),
            | Self::InvalidDesignator => (
                "designator does not identify an available subobject",
                "C99 §6.7.8p6-7: array indices are nonnegative integer constants within bounds \
                 and field designators name record members",
            ),
            | Self::NonConstantInitializer => (
                "static-storage initializer requires a constant expression",
                "C99 §6.7.8p4: every expression in an initializer for an object of static storage \
                 duration is constant or a string literal",
            ),
            | Self::QualifiedFunction => (
                "qualifiers on a function type are ignored",
                "C99 §6.7.3p8: qualifying a function type has undefined behavior",
            ),
            | Self::IncompleteObject => (
                "object requires a complete type",
                "C99 §6.7p7: an object with no linkage has complete type by the end of its \
                 declarator or initializer",
            ),
            | Self::UnknownTypedef => (
                "typedef name has no visible type binding",
                "C99 §6.7.7: a typedef name denotes its declared type",
            ),
            | Self::InvalidVaList => (
                "varargs builtin requires a modifiable va_list operand",
                "C99 §7.15p3: ap is an object of type va_list",
            ),
            | Self::InvalidVaArgType => (
                "va_arg requires a complete object type",
                "C99 §7.15.1.1p2: type permits a pointer to an object of that type",
            ),
            | Self::VaStartOutsideVariadic => (
                "va_start requires a variadic function",
                "C99 §7.15.1.4p4: parmN is the parameter immediately before the ellipsis",
            ),
            | Self::InvalidOffsetof => (
                "offsetof requires a valid non-bit-field member path",
                "C99 §7.17p3: offsetof designates a member of the specified structure type; \
                 bit-fields have undefined behavior",
            ),
            | Self::InvalidStorage => (
                "auto and register are not permitted at file scope",
                "C99 §6.9p2: an external declaration has no auto or register storage class",
            ),
            | Self::InvalidFunctionStorage => (
                "block-scope function declaration has a storage class other than extern",
                "C99 §6.7.1p5: a function declared in a block has no explicit storage class other \
                 than extern",
            ),
            | Self::LinkedBlockInitializer => (
                "block-scope declaration with linkage cannot have an initializer",
                "C99 §6.7.8p5: a block-scope declaration of an identifier with external or \
                 internal linkage has no initializer",
            ),
            | Self::InvalidRestrict => (
                "restrict requires a pointer to an object or incomplete type",
                "C99 §6.7.3p2: restrict qualifies only pointers to object or incomplete types",
            ),
            | Self::InvalidInline => (
                "inline requires a function other than main",
                "C99 §6.7.4p2-4: inline applies only to function identifiers other than main",
            ),
            | Self::InvalidDerivedType => (
                "invalid array element or function return type",
                "C99 §6.7.5.2p1 and §6.7.5.3p1: arrays need complete object elements; functions \
                 cannot return arrays or functions",
            ),
            | Self::InvalidArrayBound => (
                "array bound requires integer type; a constant bound must be positive",
                "C99 §6.7.5.2p1: array sizes have integer type; a constant size shall be greater \
                 than zero",
            ),
            | Self::InvalidStarBound => (
                "`[*]` array bound outside a function prototype declaration",
                "C99 §6.7.5.2p4: a `[*]` size is used only in declarations with function \
                 prototype scope",
            ),
            | Self::ObjectTooLarge => (
                "object type is too large",
                "C99 §5.2.4.1: this implementation limits an object to PTRDIFF_MAX bytes so that \
                 pointer differences within it are representable (§6.5.6p9)",
            ),
            | Self::FileScopeVariableType => (
                "variably modified type is not permitted here",
                "C99 §6.7.5.2p2: only an ordinary identifier with block or prototype scope and no \
                 linkage has a variably modified type; members never do (§6.7.2.1p8)",
            ),
            | Self::IncompatibleDeclaration => (
                "redeclaration has an incompatible type",
                "C99 §6.7p4 and §6.2.7p2: declarations of the same entity require compatible types",
            ),
            | Self::DuplicateDeclaration => (
                "identifier with no linkage is declared more than once",
                "C99 §6.7p3: a no-linkage identifier has at most one declaration in the same scope",
            ),
            | Self::ConflictingLinkage => (
                "identifier has both internal and external linkage",
                "C99 §6.2.2p4 and p7: extern inherits visible linkage; mixing internal and \
                 external linkage is undefined",
            ),
            | Self::TagKindMismatch => (
                "tag is used with a different kind",
                "C99 §6.7.2.3p2: declarations of a tag shall use the same struct, union or enum \
                 kind",
            ),
            | Self::TagRedefinition => (
                "tag is already defined",
                "C99 §6.7.2.3p1: a specific type has its contents defined at most once",
            ),
            | Self::IncompleteEnum => (
                "enum tag has no preceding complete declaration",
                "C99 §6.7.2.3p3: an enum specifier without an enumerator list follows a complete \
                 declaration",
            ),
            | Self::InvalidConstant => (
                "an integer constant expression is required",
                "C99 §6.6p3 and p6: integer constant expressions restrict operators and operands",
            ),
            | Self::ConstantOverflow => (
                "constant expression overflows or has an invalid operation",
                "C99 §6.6p4: a constant expression is representable in its type; §6.5p5 forbids \
                 exceptional evaluation",
            ),
            | Self::EnumeratorRange => (
                "enumeration values have no common supported integer type",
                "C99 §6.7.2.2p4: an enumerated type is compatible with an integer type that \
                 represents every member; this implementation requires a common type of at most \
                 64 bits",
            ),
            | Self::InvalidMember => (
                "member requires a complete object type",
                "C99 §6.7.2.1p2: members cannot have incomplete or function type except a final \
                 flexible array member",
            ),
            | Self::DuplicateMember => (
                "member name is declared more than once",
                "C99 §6.7p3 and §6.2.3p1: members have no linkage and a separate namespace per \
                 aggregate",
            ),
            | Self::InvalidBitFieldType => (
                "bit-field requires an integer type",
                "C99 §6.7.2.1p4: a bit-field has _Bool, signed int, unsigned int or another \
                 implementation-defined integer type",
            ),
            | Self::InvalidBitFieldWidth => (
                "bit-field width is negative or exceeds its type",
                "C99 §6.7.2.1p3: the width is a nonnegative integer constant expression no wider \
                 than the bit-field's type",
            ),
            | Self::NamedZeroWidthBitField => (
                "named bit-field has zero width",
                "C99 §6.7.2.1p3: a zero-width bit-field has no declarator",
            ),
            | Self::InvalidParameter => (
                "invalid function parameter declaration",
                "C99 §6.7.5.3p2 and p10: only register storage is allowed; void must be the sole \
                 unnamed parameter; §6.7.5.2p1 limits qualifiers/static to the outermost array",
            ),
        }
    }
}

/// Semantic error with primary and optional prior-declaration provenance.
/// C99: §5.1.1.3p1, p. 11; PDF p. 23.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SemanticError {
    pub(crate) kind:           SemanticErrorKind,
    pub(crate) source_vectors: SourceVectors,
    pub(crate) name:           Option<StringCacheId>,
    pub(crate) previous:       Option<SourceVectors>,
}

impl fmt::Display for SemanticError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.kind.explanation().0)
    }
}
impl std::error::Error for SemanticError {}
impl GetSeverity for SemanticError {
    fn severity(&self) -> ErrorSeverity {
        if matches!(
            self.kind,
            SemanticErrorKind::QualifiedFunction
                | SemanticErrorKind::MainSignature
                | SemanticErrorKind::TentativeArrayAssumedOne
                | SemanticErrorKind::UnusedStaticFunction
                | SemanticErrorKind::EmptyCaseRange
                | SemanticErrorKind::MissingReturnValueWarning
                | SemanticErrorKind::InvalidAtomicOrder
                | SemanticErrorKind::InvalidAtomicFailureOrder
                | SemanticErrorKind::AtomicBufferQualifiers
        ) {
            ErrorSeverity::Warning
        } else {
            ErrorSeverity::Error
        }
    }
}
impl GetPosition for SemanticError {
    fn position(&self, context: &Context<'_>) -> SourcePosition {
        context
            .get_source_vectors(self.source_vectors)
            .first()
            .map_or_else(SourcePosition::default, |v| v.position(context))
    }
}
impl GetSourceVectors for SemanticError {
    fn source_vectors(&self, _context: &mut Context<'_>) -> SourceVectors {
        self.source_vectors
    }
}
impl ToDiagnostic for SemanticError {
    fn diagnostic_in<'d>(
        &self,
        context: &Context<'_>,
        source: SourceVectors,
        arena: &'d Bump,
    ) -> Diagnostic<'d> {
        let (message, note) = self.kind.explanation();
        let message = if let Some(name) = self.name {
            format_in!(
                arena,
                "{message}: {}",
                quote_spelling(context.string_cache.at(name))
            )
        } else {
            message
        };
        let mut diagnostic = Explanation::new(arena, message)
            .label(self.kind.label())
            .note(note)
            .at(self.severity(), source);
        if let Some(previous) = self.previous {
            let label = match self.kind {
                | SemanticErrorKind::DuplicateLabel | SemanticErrorKind::DuplicateCase =>
                    "previous label is here",
                | SemanticErrorKind::JumpIntoVariableScope
                | SemanticErrorKind::SwitchIntoVariableScope =>
                    "variably modified identifier is declared here",
                | _ => "previous declaration is here",
            };
            diagnostic = diagnostic.secondary(previous, label);
        }
        diagnostic
    }
}
