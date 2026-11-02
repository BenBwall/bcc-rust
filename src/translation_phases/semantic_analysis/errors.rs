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

/// Semantic error with primary and optional prior-declaration provenance.
/// C99: §5.1.1.3p1, p. 11; PDF p. 23.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SemanticError {
    pub(crate) extension_severity: Option<ErrorSeverity>,
    pub(crate) kind:               SemanticErrorKind,
    pub(crate) source_vectors:     SourceVectors,
    pub(crate) name:               Option<StringCacheId>,
    pub(crate) previous:           Option<SourceVectors>,
}

/// Symbolic semantic diagnostic, independent of rendered text.
/// C99: §6.7p3-4, p. 97; PDF p. 109; subsidiary constraints are cited below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SemanticErrorKind {
    /// Pointer constraints diagnosed with Clang's group/default severity.
    /// C99: §6.5.16.1p1, p. 92; PDF p. 104; §6.5.9p2, p. 86; PDF p. 98;
    /// §6.5.8p2, p. 85; PDF p. 97; §6.5.2.2p2,p7, pp. 71-72; PDF pp. 83-84;
    /// §6.8.6.4p3, p. 139; PDF p. 151; §6.7.8p11, p. 126; PDF p. 138.
    PointerConstraint(crate::configuration::DiagnosticGroup),
    /// Additional constant forms under C99 §6.6p10, p. 96; PDF p. 108.
    FoldedArrayBound,
    FoldedIntegerConstant,
    NullPointerArithmetic,
    /// C99: §6.10.6p1, p. 159; PDF p. 171; GCC/MSVC pragma extension.
    PragmaPackMalformed,
    PragmaPackPopEmpty,
    PragmaPackPopNameAlignment,
    PragmaPackUnterminated,
    PragmaPackShow(u32),
    /// `-fpack-struct` packs every structure, so a pack pragma does nothing.
    PragmaPackIgnored,
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
    /// C99: §6.7.1p5, p. 98; PDF p. 110 forbids static on a block-scope
    /// function. GNU nested definitions have no linkage, so GCC rejects
    /// extern on them too.
    /// GNU extension: GCC manual, "Nested Functions".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Nested-Functions.html>
    NestedFunctionStorage,
    /// GCC: a local register variable's label names no register.
    /// GNU extension: GCC manual, "Local Register Variables".
    InvalidRegisterName,
    /// Clang: a local register variable's label names no register.
    /// GNU extension: GCC manual, "Local Register Variables".
    UnknownRegisterName,
    /// GCC: a local register variable's register is one the x86-64 target
    /// cannot access without APX or AVX-512.
    InaccessibleRegister,
    /// GCC: a local register variable's register cannot hold a variable.
    RegisterNotGeneral,
    /// C99: §6.9.1p3, p. 141; PDF p. 153.
    IncompleteFunctionReturn,
    /// C99: §6.9.1p5-6, p. 141; PDF p. 153.
    InvalidDefinitionParameterList,
    /// C99: §6.9.1p5, p. 141; PDF p. 153.
    UnnamedDefinitionParameter,
    /// C99: §6.7.5.3p4, p. 118; PDF p. 130.
    IncompleteDefinitionParameter,
    /// C99: §6.7.5.2p4, pp. 116-117; PDF pp. 128-129.
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
    /// `return` with a value in a function returning `void`, under
    /// `-Wreturn-mismatch`; the name is the function's.
    /// C99: §6.8.6.4p1, p. 139; PDF p. 151.
    VoidReturnMismatch,
    /// `return;` in a function returning a value, under
    /// `-Wreturn-mismatch`; the name is the function's.
    /// C99: §6.8.6.4p1, p. 139; PDF p. 151.
    ReturnMismatch,
    /// A call to an undeclared identifier, which declares it as C89 did,
    /// under `-Wimplicit-function-declaration`.
    /// C99: §6.5.1p2, p. 69; PDF p. 81; C89 §3.3.2.2 declared it
    /// `extern int f();`.
    ImplicitFunctionDeclaration,
    /// Declaration specifiers without a type specifier, under
    /// `-Wimplicit-int`; the name is the declared one.
    /// C99: §6.7.2p2, p. 99; PDF p. 111.
    ImplicitInt,
    /// A function definition's declaration specifiers without a type
    /// specifier, under `-Wimplicit-int`.
    /// C99: §6.7.2p2, p. 99; PDF p. 111.
    ImplicitIntReturn,
    /// An identifier-list parameter without a declaration, under
    /// `-Wimplicit-int`; the name is the parameter's.
    /// C99: §6.9.1p6, p. 141; PDF p. 153.
    ImplicitIntParameter,
    /// A nonempty identifier list in a function declarator that is no
    /// definition, under GCC's `-Wdeclaration-missing-parameter-type`.
    /// C99: §6.7.5.3p3, p. 118; PDF p. 130.
    DeclarationMissingParameterType,
    /// Clang: implicit int in C23, an error no option controls.
    /// C23: §6.7.3.1p2, p. 103; PDF p. 116.
    TypeSpecifierRequired,
    /// Clang and clang-cl: a nonempty identifier list in a function
    /// declarator that is no definition, an error no option controls.
    /// C99: §6.7.5.3p3, p. 118; PDF p. 130.
    IdentifierListDeclaration,
    /// C99: §6.8.6.4p3, p. 139; PDF p. 151.
    InvalidReturnConversion,
    /// C99: §6.8.5p3, p. 135; PDF p. 147.
    InvalidForDeclaration,
    /// C99: §6.8.1p3, p. 132; PDF p. 144.
    DuplicateLabel,
    /// C99: §6.8.6.1p1, p. 137; PDF p. 149.
    UndefinedLabel,
    /// GNU extension: local labels have lexical scope, unlike the function
    /// scope in C99 §6.2.1p3, p. 29; PDF p. 41.
    /// GNU extension: GCC manual, "Local Labels".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Local-Labels.html>
    DuplicateLocalLabel,
    /// GNU extension: a local-label declaration requires a definition.
    /// C99 labels are implicit definitions: §6.2.1p3, p. 29; PDF p. 41.
    /// GNU extension: GCC manual, "Local Labels".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Local-Labels.html>
    UndefinedLocalLabel,
    /// GNU extension: GCC "Statement Exprs" forbids jumping into expressions.
    /// <https://gcc.gnu.org/onlinedocs/gcc/Statement-Exprs.html>
    /// C99: §6.8.6.1p1, p. 137; PDF p. 149 (extended goto constraint).
    JumpIntoStatementExpression,
    /// GNU extension to switch dispatch, C99 §6.8.4.2p5, p. 134; PDF p. 146.
    /// <https://gcc.gnu.org/onlinedocs/gcc/Statement-Exprs.html>
    SwitchIntoStatementExpression,
    /// C99: §6.8.6.1p1, p. 137; PDF p. 149.
    JumpIntoVariableScope,
    /// C99: §6.8.4.2p2, p. 134; PDF p. 146.
    SwitchIntoVariableScope,
    /// GNU extension: a computed goto may reach every address-taken label,
    /// so one inside a variably modified scope the goto is outside of is an
    /// error, as in Clang. C99: §6.8.6.1p1, p. 137; PDF p. 149.
    /// <https://gcc.gnu.org/onlinedocs/gcc/Labels-as-Values.html>
    IndirectJumpIntoVariableScope,
    /// GNU extension: GCC "Statement Exprs" makes a computed goto into a
    /// statement expression undefined; Clang rejects a possible one.
    /// <https://gcc.gnu.org/onlinedocs/gcc/Statement-Exprs.html>
    IndirectJumpIntoStatementExpression,
    /// GNU extension: a computed goto can only reach labels whose address
    /// is taken in its function, so without one it has no target; Clang
    /// rejects it. <https://gcc.gnu.org/onlinedocs/gcc/Labels-as-Values.html>
    IndirectGotoWithoutLabels,
    /// GNU extension: a computed goto's operand converts to `const void *`
    /// as by simple assignment. C99: §6.5.16.1p1, p. 92; PDF p. 104.
    /// <https://gcc.gnu.org/onlinedocs/gcc/Labels-as-Values.html>
    InvalidComputedGoto,
    /// GNU extension: `&&label` names a label of the enclosing function, so
    /// it cannot appear outside a function body.
    /// <https://gcc.gnu.org/onlinedocs/gcc/Labels-as-Values.html>
    LabelAddressOutsideFunction,
    /// C99: §6.8.1p2, p. 131; PDF p. 143.
    CaseOutsideSwitch,
    /// C99: §6.8.4.2p3, p. 134; PDF p. 146.
    /// GNU extension: GCC manual, "Case Ranges".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Case-Ranges.html>
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
    /// Clang-compatible extension, `-Wpredefined-identifier-outside-function`;
    /// C99: §6.4.2.2p1, p. 52; PDF p. 64.
    PredefinedOutsideFunction,
    /// Clang-compatible extension; C99: §6.5.2.2p6,p9, pp. 71-72;
    /// PDF pp. 83-84; §7.1.4p2, p. 167; PDF p. 179.
    ImplicitLibraryFunction(StringCacheId),
    /// C99: §6.5.3.2p1, p. 78; PDF p. 90.
    InvalidAddressOperand,
    /// Intrinsic-only GNU builtins have no addressable external function.
    /// GNU extension: GCC manual, "Built-in Functions Provided by GCC".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Library-Builtins.html>
    BuiltinRequiresCall,
    /// A checked library builtin whose prototype names `FILE` before the
    /// program declares it, as Clang requires `<stdio.h>`.
    /// GNU extension: GCC "Object Size Checking"; C99: §7.19.1p2, p. 262;
    /// PDF p. 274.
    BuiltinRequiresHeader,
    /// GNU extension: GCC "Object Size Checking" mode constraint.
    /// <https://gcc.gnu.org/onlinedocs/gcc/Object-Size-Checking.html>
    InvalidObjectSizeMode,
    /// GNU extension: GCC "Getting the Return or Frame Address of a Function"
    /// level constraint.
    /// <https://gcc.gnu.org/onlinedocs/gcc/Return-Address.html>
    InvalidFrameLevel,
    /// GNU extension: GCC "Nonlocal Gotos": the second argument of
    /// `__builtin_longjmp` must be the constant 1.
    /// <https://gcc.gnu.org/onlinedocs/gcc/Nonlocal-Gotos.html>
    InvalidLongjmpValue,
    /// GNU extension: the bound of `__builtin_alloca_with_align_and_max`
    /// must be a constant, as GCC 13 requires it ("third argument to
    /// function ... must be a constant integer").
    InvalidAllocaMaximum,
    /// Clang's `-Wconstant-conversion`: an implicit conversion changes an
    /// integer constant's value; the payload is the interned message.
    /// C99: §6.3.1.3p3, p. 43; PDF p. 55.
    ConstantConversion(StringCacheId),
    /// GCC's `-Woverflow`: a floating constant converted out of an integer
    /// type's range in a static initializer, which stores the saturated
    /// value; the payload is the interned message.
    /// C99: §6.3.1.4 paragraph 1, p. 43; PDF p. 55.
    FloatOverflow(StringCacheId),
    /// GCC's pedantic "overflow in constant expression": a static
    /// initializer's floating value converted out of an integer type's
    /// range, by a cast too, or arithmetic on one.
    /// C99: §6.6 paragraph 4, p. 95; PDF p. 107.
    PedanticOverflow,
    /// Clang's `-Wliteral-conversion`: a floating literal's implicit
    /// conversion to an integer type is out of range or changes its value;
    /// the payload is the interned message.
    /// C99: §6.3.1.4 paragraph 1, p. 43; PDF p. 55.
    LiteralConversion(StringCacheId),
    /// A string literal initializes an array too short for its characters,
    /// null character aside; the characters that fit are stored. GCC and
    /// Clang accept it with a warning, as the constraint violation it is.
    /// C99: §6.7.8 paragraph 2, p. 125; PDF p. 137; paragraph 14, p. 126;
    /// PDF p. 138.
    ExcessString,
    /// GNU extension: GCC "Other Builtins" probability constraint.
    /// <https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html>
    InvalidBuiltinProbability,
    /// GNU extension: GCC "Other Builtins" alignment constraint.
    /// <https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html>
    InvalidBuiltinAlignment,
    /// GNU extension: GCC "Other Builtins" prefetch read/write constraint.
    /// <https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html>
    InvalidPrefetchReadWrite,
    /// GNU extension: GCC "Other Builtins" prefetch locality constraint.
    /// <https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html>
    InvalidPrefetchLocality,
    /// C99: §6.5.3p1, p. 78; PDF p. 90.
    InvalidUnaryOperand,
    /// C99: §6.5.16p2, p. 91; PDF p. 103.
    ExpectedModifiableLvalue,
    /// TCC's warning for a write through a const-qualified lvalue, which
    /// it carries out (the TCC identity only). Extension to C99 §6.5.16p2,
    /// p. 91; PDF p. 103.
    ReadOnlyLocation,
    /// C99: §6.5.2.1p1, p. 70; PDF p. 82.
    InvalidSubscript,
    /// C99: §6.5.5p2, p. 82; PDF p. 94.
    InvalidArithmeticOperands,
    /// C99: §6.5.6p2-3, pp. 82-83; PDF pp. 94-95.
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
    /// An lvalue of incomplete structure, union or enumeration type used
    /// for its value. C99 leaves the behavior undefined; GCC and Clang
    /// reject it, and there is no value to load.
    /// C99: §6.3.2.1p2, p. 46; PDF p. 58.
    IncompleteValue,
    /// C99: §6.5.2.3p1-2, p. 72; PDF p. 84.
    InvalidMemberAccess,
    /// C99: §6.5.2.2p1, p. 71; PDF p. 83.
    InvalidCall,
    /// C99: §6.5.2.2p2, p. 71; PDF p. 83.
    InvalidArgumentCount,
    /// C99: §6.5.2.2p2, p. 71; PDF p. 83.
    InvalidArgumentType,
    /// Clang: Microsoft's `selectany` applies to data with external linkage.
    /// MSVC extension: Microsoft Learn, "selectany".
    SelectAnySubject,
    /// Clang: Microsoft's `__annotation` takes wide string literals only.
    /// MSVC extension: Microsoft Learn, "__annotation".
    AnnotationArgument,
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
    /// GCC: an automatic array initialized by an array compound literal;
    /// Clang: by one whose initializer has side effects.
    /// GNU extension: GCC manual, "Compound Literals".
    NonConstantArrayInitializer,
    /// C99: §6.7.8p2, p. 125; PDF p. 137.
    ExcessInitializer,
    /// A flexible array member given elements outside a static object's own
    /// structure; GNU C allows only that case, and Clang an empty list too.
    /// C99: §6.7.2.1p16, p. 103; PDF p. 115; §6.7.8p2, p. 125; PDF p. 137.
    FlexibleArrayInitializer,
    /// A designator naming a flexible array member with an initializer that
    /// is neither a brace-enclosed list nor a string literal, an error in
    /// GCC and Clang. GNU extension: GCC manual, "Arrays of Length Zero".
    /// C99: §6.7.2.1p16, p. 103; PDF p. 115; §6.7.8p16, p. 126; PDF p. 138.
    FlexibleArrayBraces,
    /// A designator into an element of a flexible array member, which GCC
    /// and Clang reject. GNU extension: GCC manual, "Arrays of Length Zero".
    /// C99: §6.7.2.1p16, p. 103; PDF p. 115; §6.7.8p6-7, p. 125; PDF p. 137.
    FlexibleArrayDesignator,
    /// Clang's `-Winitializer-overrides`: a later initializer replaces all
    /// of a subobject an earlier one initialized.
    /// C99: §6.7.8p19, p. 127; PDF p. 139.
    InitializerOverride,
    /// Clang's `-Winitializer-overrides`: a later initializer replaces part
    /// of a subobject an earlier expression initialized as a whole.
    /// C99: §6.7.8p19, p. 127; PDF p. 139.
    PartialInitializerOverride,
    /// C99: §6.7.8p6-7, p. 125; PDF p. 137.
    InvalidDesignator,
    /// C99: §6.7.8p4, p. 125; PDF p. 137.
    NonConstantInitializer,
    /// C99: §6.9p2, p. 140; PDF p. 152.
    InvalidStorage,
    /// C99: §6.7.1p5, p. 98; PDF p. 110.
    InvalidFunctionStorage,
    /// C11: §6.7.1p4, p. 109; PDF p. 127.
    ThreadLocalFunction,
    /// C11: §6.7.1p3, p. 109; PDF p. 127.
    ThreadLocalBlockScope,
    /// C11: §6.7.1p3, p. 109; PDF p. 127.
    ThreadLocalAfterNonThreadLocal,
    /// C11: §6.7.1p3, p. 109; PDF p. 127.
    NonThreadLocalAfterThreadLocal,
    /// C99: §6.7.8p5, p. 125; PDF p. 137.
    LinkedBlockInitializer,
    /// C99: §6.7.3p2, p. 108; PDF p. 120.
    InvalidRestrict,
    /// C99: §6.7.3p8, p. 109; PDF p. 121. Qualifying a function type is
    /// undefined.
    QualifiedFunction,
    /// C99: §6.7.4p2-4, p. 112; PDF p. 124.
    InvalidInline,
    /// C99: §6.7.4p2, p. 112; PDF p. 124. A parameter declares an object.
    InlineParameter,
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
    /// A variably modified member, which Clang never accepts.
    /// C99: §6.7.2.1 paragraph 8, p. 102; PDF p. 114; §6.7.5.2 paragraph 2,
    /// p. 116; PDF p. 128.
    VariableMember,
    /// C99: §6.7.5.2p2, p. 116; PDF p. 128; members §6.7.2.1p8, p. 102;
    /// PDF p. 114.
    FileScopeVariableType,
    /// C99: §6.7p4; 6.2.7p2, p. 97; 40; PDF p. 109; 52.
    IncompatibleDeclaration,
    /// A declaration of a Microsoft compiler intrinsic whose type is not
    /// the intrinsic's (Clang's `conflicting types for '__va_start'`).
    /// MSVC extension; C99: §6.2.7p2, p. 40; PDF p. 52.
    IncompatibleIntrinsicDeclaration,
    /// A function definition of a Microsoft compiler intrinsic (Clang's
    /// `definition of builtin function`). MSVC extension; C99: §6.9.1,
    /// pp. 141-142; PDF pp. 153-154.
    IntrinsicDefinition,
    /// C99: §6.7p3, p. 97; PDF p. 109.
    DuplicateDeclaration,
    /// [`Self::DuplicateDeclaration`] of a typedef name as another typedef
    /// of a different type, which TCC words apart. C99: §6.7p3, p. 97;
    /// PDF p. 109.
    TypedefRedeclaration,
    /// C99: §6.2.2p4-7, pp. 30-31; PDF pp. 42-43.
    ConflictingLinkage,
    /// C99: §6.7.2.3p2, p. 106; PDF p. 118.
    TagKindMismatch,
    /// C99: §6.7.2.3p1, p. 106; PDF p. 118.
    TagRedefinition,
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
    /// GCC: a `_BitInt` width that is zero or negative; Clang: an
    /// `unsigned _BitInt` width below 1.
    /// C23: §6.7.3.1 paragraph 4, p. 104; PDF p. 117.
    NonPositiveBitIntWidth,
    /// A signed `_BitInt` width below 2.
    /// C23: §6.7.3.1 paragraph 4, p. 104; PDF p. 117.
    SignedBitIntWidth,
    /// A `_BitInt` width above `BITINT_MAXWIDTH`.
    /// C23: §6.7.3.1 paragraph 4, p. 104; PDF p. 117.
    BitIntWidthTooLarge,
    /// C99: §6.7.5.2p1; §6.7.5.3p2 and p10, pp. 116-119; PDF pp. 128-131.
    InvalidParameter,
    /// C99: §6.7p7, p. 98; PDF p. 110.
    IncompleteObject,
    /// C11: §6.7.2.4p3, p. 121; PDF p. 139; §6.7.3p3.
    InvalidAtomicType,
    /// C11: §6.5.1.1p2, p. 79; PDF p. 97.
    InvalidGenericSelection,
    /// Clang/GCC type-generic atomic builtin constraints.
    /// Clang extension: Clang Language Extensions, "C11 atomic builtins".
    /// <https://clang.llvm.org/docs/LanguageExtensions.html#c11-atomic-builtins>
    /// GNU extension: GCC manual, "__atomic Builtins".
    /// <https://gcc.gnu.org/onlinedocs/gcc/_005f_005fatomic-Builtins.html>
    /// GNU extension: GCC manual, "__sync Builtins".
    /// <https://gcc.gnu.org/onlinedocs/gcc/_005f_005fsync-Builtins.html>
    InvalidAtomicOperand,
    /// Clang: an `__atomic` builtin other than `__atomic_test_and_set` and
    /// `__atomic_clear` on a `_BitInt` object, which GCC accepts.
    /// Clang extension: the error "argument to atomic builtin of type
    /// '`_BitInt`' is not supported".
    AtomicBuiltinBitInt,
    /// Clang: a `__sync` builtin on a `_BitInt` whose width is not a power
    /// of two, which GCC accepts.
    /// Clang extension: the error "atomic memory operand must have a
    /// power-of-two size".
    AtomicOperandSize,
    /// Clang: a `__sync` builtin on an object of more than 16 bytes, as a
    /// `_BitInt` above 128 bits is; GCC refuses it too ("operand type ...
    /// is incompatible").
    /// Clang extension: the error "address argument to atomic builtin must
    /// be a pointer to 1,2,4,8 or 16 byte type".
    AtomicOperandBytes,
    /// Clang: `__builtin_mul_overflow` with a signed `_BitInt` operand or
    /// result of more than 128 bits, which GCC accepts.
    /// Clang extension: the error "`__builtin_mul_overflow` does not support
    /// 'signed `_BitInt`' operands of more than 128 bits".
    OverflowBuiltinBitInt,
    /// A type-generic bit builtin's operand that is no integer: GCC's
    /// "does not have integral type", Clang's "must be a scalar unsigned
    /// integer type".
    /// GNU extension: GCC manual, "Bit Operation Builtins".
    BitBuiltinNotInteger,
    /// A signed operand of a type-generic bit builtin that counts an
    /// unsigned one's bits: GCC's "has signed type", Clang's "must be a
    /// scalar unsigned integer type".
    /// GNU extension: GCC manual, "Bit Operation Builtins".
    BitBuiltinSigned,
    /// An unsigned operand of GCC's `__builtin_ffsg` or `__builtin_clrsbg`,
    /// which take a signed one: GCC's "has unsigned type".
    /// GNU extension: GCC manual, "Bit Operation Builtins".
    BitBuiltinUnsigned,
    /// A `bool` or enumerated operand, which GCC refuses for every
    /// type-generic bit builtin and Clang for the `__builtin_stdc_*` forms
    /// other than the rotations.
    /// GNU extension: GCC manual, "Bit Operation Builtins".
    BitBuiltinBoolOrEnum,
    /// The result for zero given to `__builtin_clzg` or `__builtin_ctzg`
    /// whose promoted type is not `int`.
    /// GNU extension: GCC manual, "Bit Operation Builtins".
    BitBuiltinFallback,
    /// A rotation count that is no integer, or under GCC's rules an
    /// enumerated one.
    /// GNU extension: GCC manual, "Bit Operation Builtins".
    BitBuiltinRotateCount,
    /// Clang diagnoses invalid constant orders as warnings.
    /// GNU extension: GCC manual, "__atomic Builtins".
    /// <https://gcc.gnu.org/onlinedocs/gcc/_005f_005fatomic-Builtins.html>
    InvalidAtomicOrder,
    /// Compare-exchange failure order cannot release.
    /// GNU extension: GCC manual, "__atomic Builtins".
    /// <https://gcc.gnu.org/onlinedocs/gcc/_005f_005fatomic-Builtins.html>
    /// C11: §7.17.7.4 paragraph 2, p. 283; PDF p. 301.
    InvalidAtomicFailureOrder,
    /// Clang warns when expected/output buffers discard qualifiers.
    /// GNU extension: GCC manual, "__atomic Builtins".
    /// <https://gcc.gnu.org/onlinedocs/gcc/_005f_005fatomic-Builtins.html>
    /// C99: §6.5.16.1 paragraph 1, p. 92; PDF p. 104.
    AtomicBufferQualifiers,
    /// GCC Additional Floating Types, target availability.
    /// GNU extension: GCC manual, "Floating Types".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Floating-Types.html>
    UnsupportedFloat128,
    /// `_Float128` needs binary128 and `_Float64x` a format wider than
    /// binary64, which the target lacks.
    /// C23: implementation-defined extended types, §H.2.4 paragraph 2,
    /// p. 554; PDF p. 567.
    UnsupportedInterchangeFloat,
    /// `_Float16` on a target whose ABI has no binary16 type: the pinned
    /// Clang rejects it on WebAssembly.
    /// C23: implementation-defined interchange types, §H.2.2 paragraph 1,
    /// p. 553; PDF p. 566.
    UnsupportedFloat16,
    /// `__bf16` on a target whose ABI has no bfloat16 type: the pinned Clang
    /// rejects it on WebAssembly.
    /// Clang Language Extensions, "Half-Precision Floating Point".
    UnsupportedBFloat16,
    /// `_Complex __bf16`: neither GCC nor Clang has the complex type.
    /// Extension to C99 §6.7.2 paragraph 2, pp. 99-100; PDF pp. 111-112.
    ComplexBFloat16,
    /// `_Complex __fp16`: Clang has no complex storage type.
    /// Extension to C99 §6.7.2 paragraph 2, pp. 99-100; PDF pp. 111-112.
    ComplexFp16,
    /// A parameter of type `__fp16`, which Clang refuses: the type is
    /// storage only.
    /// Clang Language Extensions, "Half-Precision Floating Point".
    Fp16Parameter,
    /// A function returning `__fp16`, which Clang refuses.
    /// Clang Language Extensions, "Half-Precision Floating Point".
    Fp16Return,
    /// GCC Other Builtins: `choose_expr` requires an integer constant.
    /// GNU extension: GCC manual, "Other Builtins".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html>
    InvalidChooseCondition,
    /// Clang extension: a selected `diagnose_if` or `unavailable` contract.
    /// <https://clang.llvm.org/docs/AttributeReference.html#diagnose-if>
    CallAttribute,
    /// GNU function `warning` attribute on an emitted direct call.
    CallAttributeWarning,
    /// Clang extension: overload candidates require viable conversions and
    /// satisfied `enable_if` predicates.
    /// <https://clang.llvm.org/docs/AttributeReference.html#overloadable>
    NoViableOverload,
    /// Clang extension: overload selection requires a unique best candidate.
    /// <https://clang.llvm.org/docs/AttributeReference.html#overloadable>
    AmbiguousOverload,
    /// GCC Vector Extensions and Clang Vectors and Extended Vectors.
    /// GNU extension: GCC manual, "Vector Extensions".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Vector-Extensions.html>
    /// Clang extension: Clang Language Extensions, "Vectors and Extended
    /// Vectors".
    /// <https://clang.llvm.org/docs/LanguageExtensions.html#vectors-and-extended-vectors>
    InvalidVectorAttribute,
    InvalidAlignmentAttribute,
    InvalidStandardAlignment,
    /// An alignment request above the 2^28 bytes GCC accepts, under GCC's
    /// rules; Clang's ignore it. GNU extension: GCC manual, "Common
    /// Variable Attributes", aligned.
    /// <https://gcc.gnu.org/onlinedocs/gcc/Common-Variable-Attributes.html>
    ExcessiveAlignment,
    /// A record that GCC's identity defines in big-endian scalar storage
    /// order, by `scalar_storage_order` or `#pragma scalar_storage_order`,
    /// which bcc does not implement: every bcc target is little-endian, and
    /// ignoring the order would store the record's scalars in the other
    /// byte order than GCC's. GNU extension: GCC manual, "Common Type
    /// Attributes", `scalar_storage_order`.
    /// <https://gcc.gnu.org/onlinedocs/gcc/Common-Type-Attributes.html>
    UnsupportedScalarStorageOrder,
    /// GCC's `scalar_storage_order` without exactly one argument.
    StorageOrderArgumentCount,
    /// GCC's `scalar_storage_order` with an argument other than the string
    /// `"big-endian"` or `"little-endian"`.
    StorageOrderArgument,
    InvalidModeAttribute,
    /// A `transparent_union` attribute ignored, as Clang warns: it names no
    /// union definition, or the union has no member, a floating or vector
    /// first member, or a member whose size differs from the first's or
    /// whose alignment exceeds it.
    /// GNU extension: GCC manual, "Common Type Attributes".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Common-Type-Attributes.html>
    IgnoredTransparentUnion,
    /// `transparent_union` written on a declaration of something other than
    /// a union type: an object, function, parameter, member, structure or
    /// non-union typedef, which Clang ignores with a warning.
    /// GNU extension: GCC manual, "Common Type Attributes".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Common-Type-Attributes.html>
    TransparentUnionNotUnion,
    /// `transparent_union` on a union specifier that does not define the
    /// union, which Clang ignores with a warning.
    /// GNU extension: GCC manual, "Common Type Attributes".
    TransparentUnionNotDefinition,
    /// `transparent_union` in the specifiers of a declaration that declares
    /// only a tag of this kind, outside the tag's specifier, which Clang
    /// ignores with a warning that the attribute belongs after the keyword.
    /// GNU extension: GCC manual, "Attribute Syntax".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Attribute-Syntax.html>
    TransparentUnionPlacement(super::types::TagKind),
    /// GCC Vector Extensions: compatible lanes and safe scalar splats.
    /// GNU extension: GCC manual, "Vector Extensions".
    /// <https://gcc.gnu.org/onlinedocs/gcc/Vector-Extensions.html>
    InvalidVectorOperand,
    /// Clang Language Extensions: shuffle and conversion builtin contracts.
    /// Clang extension: Clang Language Extensions, "Builtin Functions".
    /// <https://clang.llvm.org/docs/LanguageExtensions.html#builtin-functions>
    InvalidVectorBuiltin,
    /// Clang x86 intrinsic immediate argument constraint.
    /// GNU extension: GCC manual, "x86 Built-in Functions".
    /// <https://gcc.gnu.org/onlinedocs/gcc/x86-Built-in-Functions.html>
    InvalidImmediate,
    /// Clang `-Wargument-outside-range`, an error by default: an x86
    /// builtin's immediate constant outside its range.
    /// Clang extension: Clang Language Extensions, "Builtin Functions".
    ImmediateOutOfRange {
        value: i32,
        low:   i32,
        high:  i32,
    },
    /// GNU extension: GCC manual, "Extended Asm"; C99 §J.5.10, p. 514;
    /// PDF p. 526.
    InvalidAsm(super::assembly::AsmFault),
    /// C23 `auto` and GNU `__auto_type` inference constraints, with each
    /// compiler's choices for the implementation-defined forms.
    /// C23 (N3220): §6.7.10 paragraphs 1-2, pp. 134-135; PDF pp. 147-148;
    /// §6.7.1 paragraph 12, p. 98; PDF p. 111.
    InvalidInference(super::inference::InferenceFault),
    /// Clang `-Wunknown-attributes`: an attribute name no rule knows.
    /// C23: §6.7.13.2p2, p. 143; PDF p. 156 (unrecognized attributes are
    /// ignored). GNU extension: GCC manual, "Attribute Syntax".
    UnknownAttribute,
    /// Clang `-Wignored-attributes`: an attribute that does not apply to
    /// this declaration, or whose effect bcc drops.
    /// Clang extension: Clang Attributes Reference.
    IgnoredAttribute,
    /// Clang `-Wignored-attributes`: a 32-bit x86 calling convention that
    /// x86-64 targets other than Windows do not support.
    IgnoredConvention,
    /// Clang: an attribute's arguments have the wrong number or kind.
    /// GNU extension: GCC manual, "Attribute Syntax".
    AttributeArguments,
    /// Clang: an attribute argument must be an integer constant.
    /// C99: §6.6p6, p. 95; PDF p. 107.
    AttributeRequiresConstant,
    /// GCC: constructor and destructor priorities are 0 to 65535.
    /// GNU extension: GCC manual, "Common Function Attributes".
    InvalidPriority,
    /// GCC `-Wprio-ctor-dtor`: priorities 0 to 100 are reserved.
    /// GNU extension: GCC manual, "Common Function Attributes".
    ReservedPriority,
    /// Clang: `alias` and `section` apply to functions and global variables.
    /// GNU extension: GCC manual, "Common Variable Attributes".
    AttributeSubject,
    /// Clang: a weak declaration needs external linkage.
    /// GNU extension: GCC manual, "Common Function Attributes", weak.
    WeakInternal,
    /// Clang: a definition cannot also be an alias.
    /// GNU extension: GCC manual, "Common Function Attributes", alias.
    AliasDefinition,
    /// Clang: an alias names a definition of this translation unit.
    /// GNU extension: GCC manual, "Common Function Attributes", alias.
    AliasTarget,
    /// Clang: a function aliases a function and a variable a variable.
    AliasKind,
    /// Clang: aliases that name each other in a cycle.
    AliasCycle,
    /// Clang `-Wattribute-alias`: alias and target have different types.
    AliasTypeMismatch,
    /// Clang `-Wsection`: a redeclaration names another section.
    SectionMismatch,
    /// GNU section extension: ELF NOBITS cannot contain function code.
    BssFunction,
    /// GNU section extension: ELF NOBITS cannot contain initialized bytes.
    BssInitializer,
    /// GNU section extension: ELF NOBITS cannot contain address fixups.
    BssFixups,
    /// Clang `-Wignored-attributes`: an unsupported visibility argument.
    InvalidVisibility,
    /// Clang: the cleanup argument names no function.
    /// GNU extension: GCC manual, "Common Variable Attributes", cleanup.
    CleanupNotFunction,
    /// Clang: cleanup names one function, not an overload set.
    CleanupOverloadSet,
    /// Clang: the cleanup function takes exactly one parameter.
    CleanupParameterCount,
    /// Clang: a pointer to the variable converts to the cleanup function's
    /// parameter type. C99: §6.5.16.1p1, p. 92; PDF p. 104.
    CleanupParameterType,
    /// Clang: a goto cannot bypass a cleanup variable's declaration.
    /// C99: §6.8.6.1p1, p. 137; PDF p. 149 (extended to cleanup scopes).
    JumpIntoCleanupScope,
    /// Clang: switch dispatch cannot bypass a cleanup variable's
    /// declaration. C99: §6.8.4.2p2, p. 134; PDF p. 146 (extended).
    SwitchIntoCleanupScope,
    /// Clang: an indirect goto cannot enter or leave a cleanup variable's
    /// scope. GNU extension: GCC manual, "Labels as Values".
    IndirectJumpCleanupScope,
    /// Clang `-Wignored-attributes`: an attribute a redeclaration adds after
    /// the entity's definition is ignored.
    /// Clang extension: Clang `SemaDecl.cpp`, `checkNewAttributesAfterDef`.
    AttributeAfterDefinition,
    /// Clang `-Winvalid-unevaluated-string`: an attribute's string names
    /// text, so an encoding prefix has no effect.
    /// C23: §6.4.5p1, p. 67; PDF p. 80 (encoding prefixes).
    UnevaluatedStringPrefix,
    /// Clang `-Wignored-attributes`: `__declspec` takes only the names
    /// Clang lists in that spelling, without GNU underscore wrapping.
    /// MSVC extension: Microsoft Learn, "`__declspec`".
    UnsupportedDeclspec,
    /// Clang `-Wignored-attributes`: MinGW does not import an inline
    /// function. MSVC extension: Microsoft Learn, "dllexport, dllimport".
    DllImportOnInline,
    /// Clang `-Wignored-attributes`: on MinGW, an inline redeclaration
    /// drops an earlier `dllimport`.
    DllImportRedeclaredInline,
    /// Clang `-Wdll-attribute-on-redeclaration`: a redeclaration adds
    /// `dllimport` or `dllexport` to an entity declared without it.
    DllAttributeOnRedeclaration,
    /// Clang: once a variable is used, or a function is used and the new
    /// attribute is `dllexport`, a redeclaration cannot add DLL storage.
    DllAttributeAfterUse,
    /// Clang `-Winconsistent-dllimport`: a redeclaration without
    /// `dllimport` drops the earlier one.
    DllImportDropped,
    /// Clang `-Winconsistent-dllimport`: on the Microsoft ABI, a definition
    /// without `dllimport` after an imported declaration is exported.
    DllExportAdded,
    /// Clang: DLL storage needs external linkage.
    DllInternal,
    /// Clang: thread-local data crosses no DLL boundary.
    DllThreadLocal,
    /// Clang: an imported function has only inline definitions here.
    DllImportFunctionDefinition,
    /// Clang: imported data is defined in another image.
    DllImportDataDefinition,
    /// Clang: an exported symbol cannot be hidden.
    DllExportHidden,
    /// Clang: an imported symbol has default visibility.
    DllImportVisibility,
}

impl SemanticErrorKind {
    /// Attribute-name policy warnings use Clang's independent suppression
    /// and severity groups. C99: §5.1.1.3p1, p. 11; PDF p. 23.
    pub(super) const fn attribute_group(self) -> Option<crate::configuration::DiagnosticGroup> {
        use crate::configuration::DiagnosticGroup;
        match self {
            | Self::FoldedArrayBound | Self::FoldedIntegerConstant =>
                Some(DiagnosticGroup::GnuFoldingConstant),
            | Self::NullPointerArithmetic => Some(DiagnosticGroup::GnuNullPointerArithmetic),
            | Self::UnknownAttribute => Some(DiagnosticGroup::UnknownAttributes),
            | Self::IgnoredAttribute
            | Self::AttributeAfterDefinition
            | Self::UnsupportedDeclspec
            | Self::DllImportOnInline
            | Self::DllImportRedeclaredInline => Some(DiagnosticGroup::IgnoredAttributes),
            | Self::DllAttributeOnRedeclaration =>
                Some(DiagnosticGroup::DllAttributeOnRedeclaration),
            | Self::DllImportDropped | Self::DllExportAdded =>
                Some(DiagnosticGroup::InconsistentDllimport),
            | Self::CallAttributeWarning => Some(DiagnosticGroup::AttributeWarning),
            | Self::ImmediateOutOfRange { .. } => Some(DiagnosticGroup::ArgumentOutsideRange),
            | Self::ConstantConversion(_) => Some(DiagnosticGroup::ConstantConversion),
            | Self::FloatOverflow(_) => Some(DiagnosticGroup::Overflow),
            | Self::PedanticOverflow => Some(DiagnosticGroup::PedanticOverflow),
            | Self::LiteralConversion(_) => Some(DiagnosticGroup::LiteralConversion),
            | Self::ExcessString => Some(DiagnosticGroup::ExcessInitializers),
            | _ => None,
        }
    }

    /// The primary label names what is wrong at the labeled source.
    const fn label(self) -> &'static str {
        match self {
            | Self::PointerConstraint(
                crate::configuration::DiagnosticGroup::PointerToIntCast
                | crate::configuration::DiagnosticGroup::PointerToEnumCast
                | crate::configuration::DiagnosticGroup::VoidPointerToIntCast
                | crate::configuration::DiagnosticGroup::VoidPointerToEnumCast
                | crate::configuration::DiagnosticGroup::IntToPointerCast
                | crate::configuration::DiagnosticGroup::IntToVoidPointerCast,
            ) => "the integer and pointer widths differ",
            | Self::PointerConstraint(_) => "pointer constraint violation",
            | Self::FoldedArrayBound => "folded array bound",
            | Self::FoldedIntegerConstant => "folded integer constant expression",
            | Self::NullPointerArithmetic => "arithmetic on a null pointer",
            | Self::PragmaPackMalformed
            | Self::PragmaPackPopEmpty
            | Self::PragmaPackPopNameAlignment
            | Self::PragmaPackUnterminated
            | Self::PragmaPackShow(_)
            | Self::PragmaPackIgnored => "packing directive",
            | Self::UnsupportedFloat128
            | Self::UnsupportedInterchangeFloat
            | Self::UnsupportedFloat16
            | Self::UnsupportedBFloat16 => "unsupported floating type",
            | Self::ComplexBFloat16 | Self::ComplexFp16 => "invalid complex type",
            | Self::Fp16Parameter | Self::Fp16Return => "storage-only floating type",
            | Self::InvalidVectorAttribute => "invalid vector size or element type",
            | Self::InvalidAlignmentAttribute => "alignment must be a power-of-two constant",
            | Self::InvalidStandardAlignment => "invalid alignment specifier",
            | Self::ExcessiveAlignment => "requested alignment exceeds maximum 268435456",
            | Self::UnsupportedScalarStorageOrder => "big-endian storage order is not supported",
            | Self::StorageOrderArgumentCount => "expected one argument",
            | Self::StorageOrderArgument => "expected \"big-endian\" or \"little-endian\"",
            | Self::InvalidModeAttribute => "invalid scalar machine mode",
            | Self::IgnoredTransparentUnion => "this union cannot be made transparent",
            | Self::TransparentUnionNotUnion => "not a union type",
            | Self::TransparentUnionNotDefinition => "not a union definition",
            | Self::TransparentUnionPlacement(_) => "applies to no type here",
            | Self::InvalidVectorOperand => "invalid vector operands",
            | Self::InvalidVectorBuiltin => "invalid vector builtin operands",
            | Self::InvalidImmediate => "expected an integer constant expression in range",
            | Self::ImmediateOutOfRange { .. } => "outside the builtin's valid range",
            | Self::InvalidAsm(fault) => fault.label(),
            | Self::InvalidInference(fault) => fault.label(),
            | Self::CallAttribute | Self::CallAttributeWarning => "function attribute diagnostic",
            | Self::NoViableOverload => "no viable overload",
            | Self::AmbiguousOverload => "ambiguous overload",
            | Self::UnknownAttribute => "unknown attribute",
            | Self::IgnoredAttribute => "attribute ignored here",
            | Self::IgnoredConvention => "convention ignored on this target",
            | Self::AttributeArguments => "invalid attribute arguments",
            | Self::AttributeRequiresConstant => "priority is not an integer constant",
            | Self::InvalidPriority => "priority outside 0 to 65535",
            | Self::ReservedPriority => "reserved priority",
            | Self::AttributeSubject => "not a function or global variable",
            | Self::WeakInternal => "weak declaration without external linkage",
            | Self::AliasDefinition => "definition with an alias attribute",
            | Self::AliasTarget => "alias target is not defined here",
            | Self::AliasKind => "alias and target differ in kind",
            | Self::AliasCycle => "alias is part of a cycle",
            | Self::AliasTypeMismatch => "alias type differs from its target's",
            | Self::SectionMismatch => "different section",
            | Self::BssFunction => "function in a BSS section",
            | Self::BssInitializer => "non-zero initializer in a BSS section",
            | Self::BssFixups => "address initializer in a BSS section",
            | Self::InvalidVisibility => "unsupported visibility",
            | Self::CleanupNotFunction => "not a function",
            | Self::CleanupOverloadSet => "not a single function",
            | Self::CleanupParameterCount => "cleanup function must take one parameter",
            | Self::CleanupParameterType => "parameter type is incompatible with the variable",
            | Self::JumpIntoCleanupScope | Self::SwitchIntoCleanupScope =>
                "jump bypasses a cleanup variable's initialization",
            | Self::IndirectJumpCleanupScope => "indirect jump crosses a cleanup scope",
            | Self::AttributeAfterDefinition => "ignored after the definition",
            | Self::UnevaluatedStringPrefix => "prefix ignored",
            | Self::UnsupportedDeclspec => "not a __declspec attribute",
            | Self::DllImportOnInline | Self::DllImportRedeclaredInline =>
                "inline function not imported",
            | Self::DllAttributeOnRedeclaration | Self::DllAttributeAfterUse =>
                "DLL storage added on a redeclaration",
            | Self::DllImportDropped => "dllimport dropped",
            | Self::DllExportAdded => "dllexport added",
            | Self::DllInternal => "no external linkage",
            | Self::DllThreadLocal => "thread-local",
            | Self::DllImportFunctionDefinition => "imported function defined here",
            | Self::DllImportDataDefinition => "imported data defined here",
            | Self::DllExportHidden | Self::DllImportVisibility => "visibility conflicts",
            | Self::UnknownTypedef => "not a typedef name in this scope",
            | Self::UndeclaredIdentifier => "not declared",
            | Self::PredefinedOutsideFunction => "used outside a function",
            | Self::ImplicitLibraryFunction(_) => "library prototype used here",
            | Self::InvalidAddressOperand => "cannot take this address",
            | Self::BuiltinRequiresCall => "builtin must be directly called",
            | Self::BuiltinRequiresHeader => "use of a builtin whose library type is undeclared",
            | Self::InvalidObjectSizeMode | Self::InvalidPrefetchLocality =>
                "expected an integer constant from 0 to 3",
            | Self::InvalidBuiltinProbability => "expected a constant probability from 0 to 1",
            | Self::InvalidFrameLevel => "expected an integer constant frame level",
            | Self::InvalidLongjmpValue => "expected the constant 1",
            | Self::InvalidAllocaMaximum => "expected a constant",
            | Self::ConstantConversion(_) => "the converted value differs",
            | Self::FloatOverflow(_) | Self::LiteralConversion(_) | Self::PedanticOverflow =>
                "the value is not the integer's",
            | Self::ExcessString => "characters past the array's end are dropped",
            | Self::InvalidBuiltinAlignment => "expected a constant power-of-two alignment",
            | Self::InvalidPrefetchReadWrite => "expected an integer constant of 0 or 1",
            | Self::InvalidUnaryOperand | Self::InvalidAdditiveOperands =>
                "operand type is not allowed here",
            | Self::ExpectedModifiableLvalue => "not a modifiable lvalue",
            | Self::ReadOnlyLocation => "writes a read-only object",
            | Self::InvalidSubscript => "invalid subscript operands",
            | Self::InvalidArithmeticOperands => "operands are not arithmetic",
            | Self::InvalidIntegerOperands => "operands are not integers",
            | Self::InvalidLogicalOperands => "operands are not scalar",
            | Self::InvalidComparisonOperands => "operands cannot be compared",
            | Self::InvalidConditionalOperands => "operands do not have a common type",
            | Self::InvalidAssignment => "value cannot be assigned to this type",
            | Self::InvalidCast => "invalid conversion",
            | Self::InvalidSizeof => "operand has no size",
            | Self::IncompleteValue => "value of incomplete type",
            | Self::InvalidMemberAccess => "no such member",
            | Self::InvalidCall => "not a callable function",
            | Self::InvalidArgumentCount => "wrong number of arguments",
            | Self::AnnotationArgument => "not a wide string literal",
            | Self::SelectAnySubject => "not a data item with external linkage",
            | Self::InvalidArgumentType => "argument does not convert to the parameter type",
            | Self::InvalidCompoundLiteral => "invalid compound literal type",
            | Self::InvalidCondition => "condition is not scalar",
            | Self::InvalidSwitchExpression => "controlling expression is not an integer",
            | Self::FailedAssertion => "assertion is false",
            | Self::InvalidInitializer => "invalid initializer",
            | Self::NonConstantArrayInitializer => "not a constant array",
            | Self::ExcessInitializer => "no subobject is left to initialize",
            | Self::FlexibleArrayInitializer => "initializes a flexible array member",
            | Self::FlexibleArrayBraces => "not a brace-enclosed list",
            | Self::FlexibleArrayDesignator => "designates inside a flexible array member",
            | Self::InitializerOverride | Self::PartialInitializerOverride =>
                "overrides an earlier initializer",
            | Self::InvalidDesignator => "no such subobject",
            | Self::NonConstantInitializer => "not a constant expression",
            | Self::InvalidStorage => "external declaration",
            | Self::InvalidFunctionStorage => "function declared in a block",
            | Self::ThreadLocalFunction => "declared as a function",
            | Self::ThreadLocalBlockScope => "automatic object at block scope",
            | Self::ThreadLocalAfterNonThreadLocal => "declared thread-local here",
            | Self::NonThreadLocalAfterThreadLocal => "declared without thread storage here",
            | Self::LinkedBlockInitializer => "initializer is not allowed here",
            | Self::InvalidRestrict => "restrict on a non-object pointer",
            | Self::QualifiedFunction => "function type qualifiers are ignored",
            | Self::InvalidInline => "inline applies only to functions other than main",
            | Self::InlineParameter => "parameters are objects, not functions",
            | Self::InvalidDerivedType => "invalid element or return type",
            | Self::InvalidArrayBound => "invalid array size",
            | Self::InvalidStarBound => "`[*]` is not allowed here",
            | Self::ObjectTooLarge => "too large",
            | Self::VariableMember => "variably modified member",
            | Self::FileScopeVariableType => "variably modified type",
            | Self::IncompatibleDeclaration | Self::IncompatibleIntrinsicDeclaration =>
                "conflicting type",
            | Self::IntrinsicDefinition => "defines a compiler intrinsic",
            | Self::DuplicateDeclaration | Self::TypedefRedeclaration => "declared again here",
            | Self::ConflictingLinkage => "linkage conflicts with the previous declaration",
            | Self::TagKindMismatch => "different tag kind",
            | Self::TagRedefinition | Self::DuplicateDefinition => "defined again here",
            | Self::InvalidConstant | Self::InvalidChooseCondition =>
                "not an integer constant expression",
            | Self::ConstantOverflow => "overflow or invalid operation",
            | Self::EnumeratorRange => "no supported enum type holds this value with the others",
            | Self::InvalidMember => "incomplete member type",
            | Self::DuplicateMember => "member declared again here",
            | Self::InvalidBitFieldType => "not an integer type",
            | Self::InvalidBitFieldWidth => "invalid width",
            | Self::NonPositiveBitIntWidth
            | Self::SignedBitIntWidth
            | Self::BitIntWidthTooLarge => "invalid _BitInt width",
            | Self::NamedZeroWidthBitField => "named zero-width bit-field",
            | Self::InvalidParameter => "invalid parameter declaration",
            | Self::IncompleteObject => "incomplete type",
            | Self::InvalidFunctionDefinition => "not a function declarator",
            | Self::FunctionDefinitionStorage => "storage class is not allowed on a definition",
            | Self::NestedFunctionStorage => "storage class is not allowed on a nested definition",
            | Self::InvalidRegisterName | Self::UnknownRegisterName => "not a register name",
            | Self::InaccessibleRegister => "not accessible on this target",
            | Self::RegisterNotGeneral => "not a register a variable can live in",
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
            | Self::VoidReturnMismatch => "value returned from a void function",
            | Self::ReturnMismatch => "no return value",
            | Self::ImplicitFunctionDeclaration => "undeclared function called here",
            | Self::ImplicitInt | Self::ImplicitIntReturn => "no type specifier",
            | Self::ImplicitIntParameter => "undeclared parameter",
            | Self::DeclarationMissingParameterType | Self::IdentifierListDeclaration =>
                "parameter names without types",
            | Self::TypeSpecifierRequired => "no type specifier",
            | Self::InvalidReturnConversion => "value cannot be returned as the result type",
            | Self::InvalidForDeclaration => "not an automatic object",
            | Self::DuplicateLabel => "label defined again here",
            | Self::UndefinedLabel => "label is never defined",
            | Self::DuplicateLocalLabel => "local label declared again here",
            | Self::UndefinedLocalLabel => "local label is never defined",
            | Self::JumpIntoVariableScope | Self::SwitchIntoVariableScope =>
                "jumps into a variably modified scope",
            | Self::JumpIntoStatementExpression | Self::SwitchIntoStatementExpression =>
                "jumps into a statement expression",
            | Self::IndirectJumpIntoVariableScope => "may jump into a variably modified scope",
            | Self::IndirectJumpIntoStatementExpression => "may jump into a statement expression",
            | Self::InvalidComputedGoto => "not convertible to `const void *`",
            | Self::IndirectGotoWithoutLabels => "no label of this function has its address taken",
            | Self::LabelAddressOutsideFunction => "label address outside a function",
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
            | Self::AtomicBuiltinBitInt => "a _BitInt object",
            | Self::AtomicOperandSize => "not a power-of-two size",
            | Self::AtomicOperandBytes => "more than 16 bytes",
            | Self::OverflowBuiltinBitInt => "a signed _BitInt of more than 128 bits",
            | Self::BitBuiltinNotInteger => "not an integer",
            | Self::BitBuiltinSigned => "has a signed type",
            | Self::BitBuiltinUnsigned => "has an unsigned type",
            | Self::BitBuiltinBoolOrEnum => "has a boolean or enumerated type",
            | Self::BitBuiltinFallback => "not of type int after promotion",
            | Self::BitBuiltinRotateCount => "not a valid rotation count",
            | Self::InvalidAtomicOrder => "invalid memory order",
            | Self::InvalidAtomicFailureOrder => "invalid failure memory order",
            | Self::AtomicBufferQualifiers => "pointer conversion discards qualifiers",
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "Keep semantic diagnostic headlines and their standard notes in one exhaustive \
                  table."
    )]
    fn explanation(self) -> (&'static str, &'static str) {
        match self {
            | Self::CallAttribute
            | Self::CallAttributeWarning
            | Self::NoViableOverload
            | Self::AmbiguousOverload
            | Self::UnknownAttribute
            | Self::IgnoredAttribute
            | Self::IgnoredConvention
            | Self::AttributeArguments
            | Self::AttributeRequiresConstant
            | Self::InvalidPriority
            | Self::ReservedPriority
            | Self::AttributeSubject
            | Self::WeakInternal
            | Self::AliasDefinition
            | Self::AliasTarget
            | Self::AliasKind
            | Self::AliasCycle
            | Self::AliasTypeMismatch
            | Self::SectionMismatch
            | Self::BssFunction
            | Self::BssInitializer
            | Self::BssFixups
            | Self::InvalidVisibility
            | Self::CleanupNotFunction
            | Self::CleanupOverloadSet
            | Self::CleanupParameterCount
            | Self::CleanupParameterType
            | Self::JumpIntoCleanupScope
            | Self::SwitchIntoCleanupScope
            | Self::IndirectJumpCleanupScope
            | Self::AttributeAfterDefinition
            | Self::UnevaluatedStringPrefix
            | Self::UnsupportedDeclspec
            | Self::DllImportOnInline
            | Self::DllImportRedeclaredInline
            | Self::DllAttributeOnRedeclaration
            | Self::DllAttributeAfterUse
            | Self::DllImportDropped
            | Self::DllExportAdded
            | Self::DllInternal
            | Self::DllThreadLocal
            | Self::DllImportFunctionDefinition
            | Self::DllImportDataDefinition
            | Self::DllExportHidden
            | Self::DllImportVisibility => self.attribute_explanation(),
            | Self::FoldedArrayBound => (
                "variable length array folded to constant array as an extension",
                "C99 §6.7.5.2p4: a non-ICE bound declares a variable length array",
            ),
            | Self::FoldedIntegerConstant => (
                "expression is not an integer constant expression; folding it to a constant is a GNU extension",
                "C99 §6.6p6: an integer constant expression reads no object and converts no pointer",
            ),
            | Self::NullPointerArithmetic => (
                "arithmetic on a null pointer treated as a cast from integer to pointer is a GNU extension",
                "C99 §6.6p10 permits additional constant forms",
            ),
            | Self::PointerConstraint(group @ (crate::configuration::DiagnosticGroup::PointerToIntCast
            | crate::configuration::DiagnosticGroup::PointerToEnumCast
            | crate::configuration::DiagnosticGroup::VoidPointerToIntCast
            | crate::configuration::DiagnosticGroup::VoidPointerToEnumCast
            | crate::configuration::DiagnosticGroup::IntToPointerCast
            | crate::configuration::DiagnosticGroup::IntToVoidPointerCast)) => (
                match group {
                    | crate::configuration::DiagnosticGroup::PointerToIntCast => "cast from pointer to integer of different size",
                    | crate::configuration::DiagnosticGroup::PointerToEnumCast => "cast from pointer to smaller enumeration type",
                    | crate::configuration::DiagnosticGroup::VoidPointerToIntCast => "cast from 'void *' to smaller integer type",
                    | crate::configuration::DiagnosticGroup::VoidPointerToEnumCast => "cast from 'void *' to smaller enumeration type",
                    | crate::configuration::DiagnosticGroup::IntToPointerCast => "cast to pointer from integer of different size",
                    | _ => "cast to 'void *' from smaller integer type",
                },
                "C99 §6.3.2.3p5-6: the result of converting between a pointer and an integer is implementation-defined and may not be representable",
            ),
            | Self::PointerConstraint(group) => (
                match group {
                    | crate::configuration::DiagnosticGroup::CompareDistinctPointerTypes => "comparison of distinct pointer types",
                    | crate::configuration::DiagnosticGroup::IncompatiblePointerTypes => "incompatible pointer types in conversion",
                    | crate::configuration::DiagnosticGroup::NonLiteralNullConversion => "expression which evaluates to zero treated as a null pointer constant",
                    | crate::configuration::DiagnosticGroup::PointerCompare => "comparison between pointer and zero character constant",
                    | crate::configuration::DiagnosticGroup::NullConversion => "implicit conversion of nullptr constant to bool",
                    | crate::configuration::DiagnosticGroup::PointerBoolConversion => "address always evaluates to true in a boolean context",
                    | crate::configuration::DiagnosticGroup::VoidPtrDereference => "dereferencing void pointer",
                    | crate::configuration::DiagnosticGroup::Nonnull => "null passed to a callee that requires a non-null argument",
                    | crate::configuration::DiagnosticGroup::PointerSign => "conversion between pointers to integer types with different sign",
                    | crate::configuration::DiagnosticGroup::PointerQualifiers => "pointer conversion discards qualifiers, including nested pointer qualifiers",
                    | crate::configuration::DiagnosticGroup::IncompatibleFunctionPointerTypes => "incompatible function pointer types in conversion",
                    | crate::configuration::DiagnosticGroup::IntConversion => "incompatible integer and pointer conversion",
                    | crate::configuration::DiagnosticGroup::PointerIntegerCompare => "comparison between pointer and integer",
                    | crate::configuration::DiagnosticGroup::OrderedCompareFunctionPointers => "ordered comparison of function pointers",
                    | crate::configuration::DiagnosticGroup::OrderedPointerIntegerCompare => "ordered comparison between pointer and integer",
                    | crate::configuration::DiagnosticGroup::PointerTypeMismatch => "pointer type mismatch in conditional expression",
                    | crate::configuration::DiagnosticGroup::ConditionalTypeMismatch => "pointer/integer type mismatch in conditional expression",
                    | crate::configuration::DiagnosticGroup::PointerToIntCast
                    | crate::configuration::DiagnosticGroup::PointerToEnumCast
                    | crate::configuration::DiagnosticGroup::VoidPointerToIntCast
                    | crate::configuration::DiagnosticGroup::VoidPointerToEnumCast
                    | crate::configuration::DiagnosticGroup::IntToPointerCast
                    | crate::configuration::DiagnosticGroup::IntToVoidPointerCast
                    | crate::configuration::DiagnosticGroup::ArgumentOutsideRange
                    | crate::configuration::DiagnosticGroup::NewlineEof
                    | crate::configuration::DiagnosticGroup::Undef
                    | crate::configuration::DiagnosticGroup::PredefinedOutsideFunction
                    | crate::configuration::DiagnosticGroup::UnknownWarningOption
                    | crate::configuration::DiagnosticGroup::UnknownAttributes
                    | crate::configuration::DiagnosticGroup::IgnoredAttributes
                    | crate::configuration::DiagnosticGroup::AttributeWarning
                    | crate::configuration::DiagnosticGroup::DllAttributeOnRedeclaration
                    | crate::configuration::DiagnosticGroup::InconsistentDllimport
                    | crate::configuration::DiagnosticGroup::UnknownStdcPragma
                    | crate::configuration::DiagnosticGroup::UnknownEscapeSequence
                    | crate::configuration::DiagnosticGroup::InvalidSourceEncoding
                    | crate::configuration::DiagnosticGroup::UnknownPragmas
                    | crate::configuration::DiagnosticGroup::UnusedFunction
                    | crate::configuration::DiagnosticGroup::InitializerOverrides
                    | crate::configuration::DiagnosticGroup::GnuFoldingConstant
                    | crate::configuration::DiagnosticGroup::GnuNullPointerArithmetic
                    | crate::configuration::DiagnosticGroup::ConstantPromotion
                    | crate::configuration::DiagnosticGroup::ConstantConversion
                    | crate::configuration::DiagnosticGroup::Overflow
                    | crate::configuration::DiagnosticGroup::PedanticOverflow
                    | crate::configuration::DiagnosticGroup::Conversion
                    | crate::configuration::DiagnosticGroup::LiteralConversion
                    | crate::configuration::DiagnosticGroup::ExcessInitializers
                    | crate::configuration::DiagnosticGroup::ImplicitFunctionDeclaration
                    | crate::configuration::DiagnosticGroup::ImplicitLibraryFunction
                    | crate::configuration::DiagnosticGroup::ImplicitInt
                    | crate::configuration::DiagnosticGroup::Implicit
                    | crate::configuration::DiagnosticGroup::ReturnMismatch
                    | crate::configuration::DiagnosticGroup::VoidReturnMismatch
                    | crate::configuration::DiagnosticGroup::DeclarationMissingParameterType => "warning-group diagnostic",
                },
                "C99 §6.5.16.1p1, §6.5.15p3, §6.5.9p2, §6.5.8p2, §6.5.2.2p2,p7, §6.8.6.4p3 and §6.7.8p11: pointer constraints require a diagnostic; a warning satisfies §5.1.1.3p1",
            ),
            | Self::PragmaPackMalformed => ("malformed #pragma pack ignored", "C99 §6.10.6p1: implementation-defined GCC/MSVC packing pragma"),
            | Self::PragmaPackPopEmpty => ("#pragma pack(pop, ...) failed: stack empty", "GCC/MSVC pack restores a saved alignment from its stack"),
            | Self::PragmaPackPopNameAlignment => ("specifying both a name and alignment to 'pop' is undefined", "Clang warns and applies the named pop followed by the alignment"),
            | Self::PragmaPackUnterminated => ("unterminated '#pragma pack(push, ...)' at end of file", "restore the saved packing state with #pragma pack(pop)"),
            | Self::PragmaPackShow(_) => ("current #pragma pack alignment value", "GCC/MSVC packing state at this directive"),
            | Self::PragmaPackIgnored => ("'#pragma pack' has no effect with '-fpack-struct' - ignored", "GCC packs every structure under -fpack-struct without a value"),
            | Self::BuiltinRequiresHeader => (
                "this builtin's prototype needs the FILE type of <stdio.h>",
                "GNU extension: the checked stream builtin takes a FILE pointer; Clang rejects its use until FILE is declared",
            ),
            | Self::BuiltinRequiresCall => (
                "builtin functions must be directly called",
                "GNU extension: intrinsic-only builtins have no addressable external function; parentheses and compile-time selections preserve callee designation",
            ),
            | Self::InvalidObjectSizeMode => (
                "the mode argument must be an integer constant from 0 to 3",
                "GNU extension: GCC Object Size Checking requires a constant mode after conversion to int",
            ),
            | Self::InvalidFrameLevel => (
                "the frame level must be an integer constant",
                "GNU extension: GCC requires the level of __builtin_frame_address and __builtin_return_address to be a constant unsigned integer",
            ),
            | Self::PedanticOverflow => (
                "overflow in constant expression",
                "C99 §6.6p4: a constant expression evaluates to a value its type represents; GCC \
                 folds an out-of-range conversion to the nearest bound",
            ),
            | Self::FloatOverflow(_) | Self::LiteralConversion(_) => (
                "floating value converted to an integer type changes",
                "C99 §6.3.1.4p1: a floating value converts to an integer by truncation toward \
                 zero; an integral part out of range is undefined, and GCC and Clang store the \
                 nearest bound, or 0 for a NaN",
            ),
            | Self::ExcessString => (
                "initializer-string is too long for its array",
                "C99 §6.7.8p2 and p14: the string's characters initialize the array's elements; \
                 none may lie outside the array",
            ),
            | Self::ConstantConversion(_) => (
                "implicit conversion changes the value of an integer constant",
                "C99 §6.3.1.3p3: the conversion to a signed type that cannot represent the value \
                 is implementation-defined; bcc, GCC and Clang reduce it modulo 2^N",
            ),
            | Self::InvalidAllocaMaximum => (
                "the third argument must be a constant integer",
                "GNU extension: GCC requires the maximum size of \
                 __builtin_alloca_with_align_and_max to be a constant",
            ),
            | Self::InvalidLongjmpValue => (
                "the value argument must be the integer constant 1",
                "GNU extension: GCC Nonlocal Gotos requires the second argument of \
                 __builtin_longjmp to be the integer constant 1",
            ),
            | Self::InvalidBuiltinProbability => (
                "the probability argument must be a constant from 0 to 1",
                "GNU extension: GCC Other Builtins requires a constant probability in the inclusive range [0, 1] after conversion to double",
            ),
            | Self::InvalidBuiltinAlignment => (
                "the alignment argument must be a positive power-of-two integer constant",
                "GNU extension: GCC Other Builtins requires a constant power-of-two alignment after conversion to size_t; the optional offset may be nonconstant",
            ),
            | Self::InvalidPrefetchReadWrite => (
                "the read/write argument must be an integer constant of 0 or 1",
                "GNU extension: GCC Other Builtins uses 0 for read prefetch and 1 for write prefetch",
            ),
            | Self::InvalidPrefetchLocality => (
                "the locality argument must be an integer constant from 0 to 3",
                "GNU extension: GCC Other Builtins uses locality 0 for no temporal locality through 3 for high temporal locality",
            ),
            | Self::ImplicitLibraryFunction(_) => (
                "implicitly declaring library function",
                "C99 §6.5.2.2p6,p9: calls require compatible function types; §7.1.4p2 permits declaring library functions without their headers",
            ),
            | Self::AtomicBufferQualifiers => (
                "atomic buffer argument discards pointer target qualifiers",
                "Clang atomic builtins convert generic value, expected and output buffers to pointers to the unqualified value type",
            ),
            | Self::AtomicBuiltinBitInt => (
                "argument to atomic builtin of type '_BitInt' is not supported",
                "Clang's __atomic builtins refuse bit-precise integers, except \
                 __atomic_test_and_set and __atomic_clear; GCC accepts them",
            ),
            | Self::AtomicOperandBytes => (
                "address argument to atomic builtin must be a pointer to 1,2,4,8 or 16 byte type",
                "the __sync builtins operate on objects of at most 16 bytes, in Clang and GCC",
            ),
            | Self::OverflowBuiltinBitInt => (
                "__builtin_mul_overflow does not support 'signed _BitInt' operands of more \
                 than 128 bits",
                "Clang's __builtin_mul_overflow multiplies at most 128 bits of a signed \
                 _BitInt; GCC accepts any width",
            ),
            | Self::BitBuiltinNotInteger => (
                "argument must have an integer type",
                "GCC's and Clang's type-generic bit builtins compute on the bits of an \
                 integer operand",
            ),
            | Self::BitBuiltinSigned => (
                "argument must have an unsigned integer type",
                "GCC's and Clang's type-generic bit builtins count the bits of an unsigned \
                 operand; only __builtin_ffsg and __builtin_clrsbg take a signed one",
            ),
            | Self::BitBuiltinUnsigned => (
                "argument must have a signed integer type",
                "GCC's __builtin_ffsg and __builtin_clrsbg take a signed operand",
            ),
            | Self::BitBuiltinBoolOrEnum => (
                "argument must not have a boolean or enumerated type",
                "GCC refuses bool and enumerated operands of its type-generic bit builtins, \
                 and Clang of the __builtin_stdc_* forms other than the rotations",
            ),
            | Self::BitBuiltinFallback => (
                "the result for zero must have type int",
                "the second argument of __builtin_clzg and __builtin_ctzg is an int, after \
                 the integer promotions",
            ),
            | Self::BitBuiltinRotateCount => (
                "the rotation count must have an integer type",
                "GCC and Clang rotate by an integer count of any type, which GCC requires to \
                 be no enumeration",
            ),
            | Self::AtomicOperandSize => (
                "atomic memory operand must have a power-of-two size",
                "Clang's __sync builtins take a _BitInt only of a power-of-two width; GCC \
                 accepts any",
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
            | Self::UnsupportedInterchangeFloat => (
                "this floating type is not supported on this target",
                "C23 Annex H: _Float128 needs binary128 and _Float64x a format wider than \
                 binary64; this target's long double is binary64 and it has no binary128 type",
            ),
            | Self::UnsupportedFloat16 => (
                "_Float16 is not supported on this target",
                "Clang provides _Float16 only where the target's ABI defines it; WebAssembly's \
                 does not (Clang Language Extensions, \"Half-Precision Floating Point\")",
            ),
            | Self::UnsupportedBFloat16 => (
                "__bf16 is not supported on this target",
                "Clang provides __bf16 only where the target's ABI defines it; WebAssembly's \
                 does not; __fp16, a storage-only type, is available",
            ),
            | Self::ComplexBFloat16 => (
                "'_Complex __bf16' is invalid",
                "GCC and Clang provide __bf16 as a real floating type only",
            ),
            | Self::ComplexFp16 => (
                "'_Complex half' is invalid",
                "Clang Language Extensions: __fp16 is a storage-only real type",
            ),
            | Self::Fp16Parameter => (
                "parameters cannot have __fp16 type; did you forget * ?",
                "Clang Language Extensions: __fp16 is a storage-only type; a pointer to one may \
                 be passed",
            ),
            | Self::Fp16Return => (
                "function return value cannot have __fp16 type; did you forget * ?",
                "Clang Language Extensions: __fp16 is a storage-only type; a pointer to one may \
                 be returned",
            ),
            | Self::UnsupportedFloat128 => (
                "__float128 is not supported on this target",
                "GCC Additional Floating Types: __float128 requires target binary128 support; \
                 Clang rejects it on x86_64-pc-windows-msvc",
            ),
            | Self::InvalidVectorAttribute => (
                "invalid vector attribute",
                "GCC Vector Extensions: vector_size requires an integer or real floating element \
                 and a positive multiple of its size",
            ),
            | Self::InvalidAlignmentAttribute => (
                "invalid alignment attribute",
                "GCC aligned requires a positive integer constant power of two representable by the target",
            ),
            | Self::InvalidStandardAlignment => (
                "invalid alignment specifier",
                "C11 §6.7.5p2-6: alignment requires an eligible object declaration and must be zero or a valid alignment that does not weaken its natural alignment",
            ),
            | Self::UnsupportedScalarStorageOrder => (
                "big-endian scalar storage order is not supported: bcc stores scalars in the \
                 target's little-endian order, which would differ from GCC's",
                "GCC scalar_storage_order: the scalar members of a structure or union defined with \
                 it, or under `#pragma scalar_storage_order`, are stored in the given byte order",
            ),
            | Self::StorageOrderArgumentCount => (
                "wrong number of arguments specified for `scalar_storage_order` attribute",
                "GCC scalar_storage_order takes one argument, \"big-endian\" or \"little-endian\"",
            ),
            | Self::StorageOrderArgument => (
                "attribute `scalar_storage_order` argument must be one of \"big-endian\" or \
                 \"little-endian\"",
                "GCC scalar_storage_order takes one argument, \"big-endian\" or \"little-endian\"",
            ),
            | Self::ExcessiveAlignment => (
                "requested alignment exceeds maximum 268435456",
                "GCC aligned and C11 §6.7.5 alignment requests: GCC rejects an alignment above \
                 2^28 bytes, the most an object file section can carry on its targets",
            ),
            | Self::InvalidModeAttribute => (
                "invalid mode attribute",
                "GCC mode requires an eligible scalar type and a supported scalar machine mode",
            ),
            | Self::IgnoredTransparentUnion => (
                "transparent_union attribute ignored",
                "GNU transparent_union applies to a union definition with at least one member, a \
                 first member of neither floating nor vector type, and every member of the first \
                 member's size and at most its alignment",
            ),
            | Self::TransparentUnionNotUnion => (
                "'transparent_union' attribute only applies to unions",
                "GNU transparent_union is a type attribute of a union; it is ignored on objects, \
                 functions, parameters, members, structures and other types",
            ),
            | Self::TransparentUnionNotDefinition => (
                "transparent_union attribute can only be applied to a union definition; \
                 attribute ignored",
                "GNU transparent_union applies to a union where its members are defined",
            ),
            | Self::TransparentUnionPlacement(kind) => (
                match kind {
                    | super::types::TagKind::Struct =>
                        "attribute 'transparent_union' is ignored, place it after \"struct\" to \
                         apply attribute to type declaration",
                    | super::types::TagKind::Union =>
                        "attribute 'transparent_union' is ignored, place it after \"union\" to \
                         apply attribute to type declaration",
                    | super::types::TagKind::Enum =>
                        "attribute 'transparent_union' is ignored, place it after \"enum\" to \
                         apply attribute to type declaration",
                },
                "GNU attribute syntax: in a declaration that declares only a tag, an attribute \
                 among its declaration specifiers applies to no declarator; write it after the \
                 keyword or after the closing brace of the definition",
            ),
            | Self::InvalidVectorOperand => (
                "invalid vector operation",
                "Clang Language Extensions: vector operators require valid element types and \
                 matching widths; scalar operands must convert without truncation",
            ),
            | Self::InvalidVectorBuiltin => (
                "invalid vector builtin",
                "Clang Language Extensions: shufflevector requires matching vectors and constant \
                 indices; convertvector requires equal lane counts",
            ),
            | Self::InvalidAsm(fault) => fault.explanation(),
            | Self::InvalidInference(fault) => fault.explanation(),
            | Self::ImmediateOutOfRange { .. } => (
                "argument value is outside the valid range",
                "Clang x86 intrinsics: the immediate operand must lie in the builtin's \
                 permitted range",
            ),
            | Self::InvalidImmediate => (
                "invalid intrinsic immediate operand",
                "Clang x86 intrinsics: the immediate operand must be an integer constant \
                 expression in the builtin's permitted range",
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
            | Self::InvalidRegisterName => (
                "invalid register name for the register variable",
                "GNU local register variables: the assembler label names an x86-64 register, \
                 such as `rdi` or `r8`",
            ),
            | Self::UnknownRegisterName => (
                "unknown register name in asm",
                "GNU local register variables: the assembler label names an x86-64 register, \
                 such as `rdi` or `r8d`",
            ),
            | Self::InaccessibleRegister => (
                "the register specified for the variable cannot be accessed by the current target",
                "x86-64 without APX or AVX-512 has no such register",
            ),
            | Self::RegisterNotGeneral => (
                "the register specified for the variable is not general enough to be used as a \
                 register variable",
                "GNU local register variables: the flags hold no variable",
            ),
            | Self::NestedFunctionStorage => (
                "invalid storage class in nested function definition",
                "C99 §6.7.1p5: a function declared in a block has no explicit storage class other \
                 than extern; GCC also rejects extern on a nested definition, which has no linkage",
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
            | Self::VoidReturnMismatch => (
                "void function returns an expression",
                "C99 §6.8.6.4p1: a return statement with an expression shall not appear in a \
                 function returning void",
            ),
            | Self::ReturnMismatch => (
                "non-void function returns without a value",
                "C99 §6.8.6.4p1: a return statement without an expression shall only appear in a \
                 function returning void; C89 permitted it",
            ),
            | Self::ImplicitFunctionDeclaration => (
                "implicit declaration of function",
                "C99 §6.5.1p2: an identifier is a primary expression only if it is declared; C89 \
                 §3.3.2.2 declared an undeclared called identifier as `extern int f();`",
            ),
            | Self::ImplicitInt => (
                "type specifier missing, defaults to `int`",
                "C99 §6.7.2p2: each declaration gives at least one type specifier; C89 defaulted \
                 to `int`",
            ),
            | Self::ImplicitIntReturn => (
                "return type defaults to `int`",
                "C99 §6.7.2p2: each declaration gives at least one type specifier; C89 defaulted \
                 to `int`",
            ),
            | Self::ImplicitIntParameter => (
                "parameter was not declared, defaults to `int`",
                "C99 §6.9.1p6: every identifier in the identifier list shall be declared; C89 \
                 defaulted to `int`",
            ),
            | Self::TypeSpecifierRequired => (
                "a type specifier is required for all declarations",
                "C23 §6.7.3.1p2: each declaration gives at least one type specifier unless its \
                 type is inferred",
            ),
            | Self::IdentifierListDeclaration => (
                "a parameter list without types is only allowed in a function definition",
                "C99 §6.7.5.3p3: an identifier list in a function declarator that is not part of \
                 a definition shall be empty",
            ),
            | Self::DeclarationMissingParameterType => (
                "parameter names without types in a function declaration",
                "C99 §6.7.5.3p3: an identifier list in a function declarator that is not part of \
                 a definition shall be empty",
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
            | Self::JumpIntoStatementExpression | Self::SwitchIntoStatementExpression => Self::statement_expression_explanation(self == Self::JumpIntoStatementExpression),
            | Self::JumpIntoVariableScope => (
                "goto enters the scope of a variably modified identifier",
                "C99 §6.8.6.1p1: a goto cannot jump from outside the scope of a variably modified \
                 identifier to inside that scope",
            ),
            | Self::IndirectJumpIntoVariableScope => (
                "computed goto can enter the scope of a variably modified identifier at its \
                 possible target",
                "C99 §6.8.6.1p1: a goto cannot jump from outside the scope of a variably modified \
                 identifier to inside that scope; GCC Labels as Values: a computed goto may reach \
                 every label whose address is taken",
            ),
            | Self::IndirectJumpIntoStatementExpression => (
                "computed goto can enter a statement expression at its possible target",
                "GCC Statement Exprs: jumping into a statement expression with a computed goto is \
                 undefined; GCC Labels as Values: a computed goto may reach every label whose \
                 address is taken",
            ),
            | Self::IndirectGotoWithoutLabels => (
                "computed goto in a function without label addresses",
                "GCC Labels as Values: a computed goto may only jump to a label of its own \
                 function whose address is taken with `&&`",
            ),
            | Self::InvalidComputedGoto => (
                "computed goto operand is not a pointer",
                "GCC Labels as Values: `goto *expression` converts the operand to `const void *` \
                 as C99 §6.5.16.1p1 simple assignment does",
            ),
            | Self::LabelAddressOutsideFunction => (
                "label address outside a function body",
                "GCC Labels as Values: `&&label` names a label of the function that contains it",
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
            | Self::PredefinedOutsideFunction => (
                "predefined identifier is only valid inside function",
                "C99 §6.4.2.2p1: __func__ names the enclosing function; GCC and Clang extend this \
                 to __FUNCTION__ and __PRETTY_FUNCTION__ and use an empty or `top level` string \
                 outside one",
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
            | Self::ReadOnlyLocation => (
                "assignment of read-only location",
                "C99 §6.5.16p2: assignment and increment operators require a modifiable lvalue; \
                 TCC writes the object anyway",
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
            | Self::IncompleteValue => (
                "value of an object requires a complete type",
                "C99 §6.3.2.1p2: an lvalue of incomplete non-array type has no value to convert to",
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
            | Self::SelectAnySubject => (
                "'selectany' can only be applied to data items with external linkage",
                "Clang: Microsoft's `selectany` lets several translation units define one \
                 external object",
            ),
            | Self::AnnotationArgument => (
                "argument to `__annotation` must be a wide string literal",
                "Clang: Microsoft's `__annotation` records wide strings in the debug \
                 information",
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
            | Self::NonConstantArrayInitializer => (
                "array initialized from non-constant array expression",
                "C99 §6.7.8p16: an array is initialized by a braced list; GCC also accepts a \
                 compound literal of its type for static storage, Clang one without side effects",
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
            | Self::FlexibleArrayInitializer => (
                "initialization of flexible array member is not allowed",
                "C99 §6.7.2.1p16, §6.7.8p2: a flexible array member has no elements; as a GNU \
                 extension only an object with static storage duration may give the flexible \
                 array member of its own structure elements",
            ),
            | Self::FlexibleArrayBraces => (
                "flexible array requires brace-enclosed initializer",
                "C99 §6.7.8p16: an array is initialized by a brace-enclosed list; as in GCC and \
                 Clang, a designated flexible array member takes a list or a string literal, \
                 never a brace-elided element",
            ),
            | Self::FlexibleArrayDesignator => (
                "designator into flexible array member subobject",
                "GNU flexible array member initialization: as in GCC and Clang, a designator may \
                 name the flexible array member but not an element or member inside it",
            ),
            | Self::InitializerOverride => (
                "initializer overrides prior initialization of this subobject",
                "C99 §6.7.8p19: each initializer for a subobject overrides any previously listed \
                 initializer for the same subobject",
            ),
            | Self::PartialInitializerOverride => (
                "initializer partially overrides prior initialization of this subobject",
                "C99 §6.7.8p19: each initializer for a subobject overrides any previously listed \
                 initializer for the same subobject, here part of one an expression initialized",
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
            | Self::ThreadLocalFunction => (
                "thread storage duration applies only to objects",
                "C11 §6.7.1p4: `_Thread_local` does not appear in the declaration specifiers of \
                 a function declaration",
            ),
            | Self::ThreadLocalBlockScope => (
                "thread-local object at block scope must be static or extern",
                "C11 §6.7.1p3: a block-scope declaration with `_Thread_local` also has `static` \
                 or `extern`",
            ),
            | Self::ThreadLocalAfterNonThreadLocal => (
                "thread-local declaration follows a non-thread-local declaration",
                "C11 §6.7.1p3: if `_Thread_local` appears in any declaration of an object, it \
                 appears in every declaration of that object",
            ),
            | Self::NonThreadLocalAfterThreadLocal => (
                "non-thread-local declaration follows a thread-local declaration",
                "C11 §6.7.1p3: if `_Thread_local` appears in any declaration of an object, it \
                 appears in every declaration of that object",
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
            | Self::InlineParameter => (
                "inline is not allowed on a parameter",
                "C99 §6.7.4p2: function specifiers are used only in the declaration of an \
                 identifier for a function",
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
            | Self::VariableMember => (
                "fields must have a constant size",
                "C99 §6.7.5.2p2: members never have a variably modified type (§6.7.2.1p8); \
                 Clang does not support GCC's variably modified members",
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
            | Self::IncompatibleIntrinsicDeclaration => (
                "declaration conflicts with the type of a compiler intrinsic",
                "the compiler declares this Microsoft intrinsic itself; C99 §6.2.7p2: \
                 declarations of the same function require compatible types",
            ),
            | Self::IntrinsicDefinition => (
                "a compiler intrinsic cannot be defined",
                "the compiler implements this Microsoft intrinsic, so a program declares it but \
                 does not define it",
            ),
            | Self::DuplicateDeclaration | Self::TypedefRedeclaration => (
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
            | Self::NonPositiveBitIntWidth => (
                "_BitInt width is not positive",
                "C23 §6.7.3.1p4: the width N of a _BitInt is an integer constant expression of at \
                 least 1 for unsigned _BitInt",
            ),
            | Self::SignedBitIntWidth => (
                "signed _BitInt width is less than 2",
                "C23 §6.7.3.1p4: the width N of a signed _BitInt is at least 2",
            ),
            | Self::BitIntWidthTooLarge => (
                "_BitInt width exceeds BITINT_MAXWIDTH",
                "C23 §6.7.3.1p4: N is at most BITINT_MAXWIDTH: 65535 for GCC and 8388608 for Clang \
                 on x86-64, 128 on WebAssembly",
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

    fn attribute_explanation(self) -> (&'static str, &'static str) {
        match self {
            | Self::CallAttribute | Self::CallAttributeWarning => (
                "function attribute diagnostic",
                "GNU error/warning and Clang diagnose_if/unavailable attach a diagnostic to calls",
            ),
            | Self::NoViableOverload => (
                "no matching function for call",
                "Clang overloadable/enable_if: a candidate needs viable argument conversions and \
                 constant true enable_if predicates",
            ),
            | Self::AmbiguousOverload => (
                "call to overloaded function is ambiguous",
                "Clang overloadable: no unique best viable candidate",
            ),
            | Self::UnknownAttribute => (
                "unknown attribute ignored",
                "Clang -Wunknown-attributes: bcc does not know this attribute and ignores it",
            ),
            | Self::IgnoredAttribute => (
                "attribute ignored",
                "Clang -Wignored-attributes: the attribute does not apply to this declaration, or \
                 bcc drops its effect (see language-standards.md, \"Attribute status\")",
            ),
            | Self::IgnoredConvention => (
                "calling convention is not supported for this target",
                "Clang -Wignored-attributes: x86-64 outside Windows uses one C calling convention",
            ),
            | Self::AttributeArguments => (
                "wrong number or kind of attribute arguments",
                "GCC Attribute Syntax: cleanup takes one identifier; alias, section and \
                 visibility one string literal; constructor and destructor at most one priority; \
                 weak and used none",
            ),
            | Self::AttributeRequiresConstant => (
                "attribute requires an integer constant",
                "GCC Common Function Attributes: a constructor or destructor priority is an \
                 integer constant expression",
            ),
            | Self::InvalidPriority => (
                "constructor and destructor priorities must be integers from 0 to 65535",
                "GCC Common Function Attributes: priorities are integers from 0 to 65535 inclusive",
            ),
            | Self::ReservedPriority => (
                "constructor and destructor priorities from 0 to 100 are reserved for the \
                 implementation",
                "GCC -Wprio-ctor-dtor: priorities 0 to 100 are reserved; bundled Clang accepts \
                 them silently",
            ),
            | Self::AttributeSubject => (
                "attribute only applies to functions and global variables",
                "Clang: alias and section name a symbol, which automatic variables do not have",
            ),
            | Self::WeakInternal => (
                "weak declaration cannot have internal linkage",
                "GCC Common Function Attributes: weak makes an external symbol overridable; \
                 internal and automatic identifiers have no external symbol",
            ),
            | Self::AliasDefinition => (
                "definition cannot also be an alias",
                "GCC Common Function Attributes: an alias declaration takes its definition from \
                 its target; tentative definitions are definitions",
            ),
            | Self::AliasTarget => (
                "alias must point to a defined variable or function",
                "GCC Common Function Attributes: the target names a definition in the same \
                 translation unit, by its emitted symbol name",
            ),
            | Self::AliasKind => (
                "alias and target must both be functions or both be variables",
                "Clang: a function cannot alias a variable, nor a variable a function",
            ),
            | Self::AliasCycle => (
                "alias definition is part of a cycle",
                "Clang: following alias targets must reach a definition",
            ),
            | Self::AliasTypeMismatch => (
                "alias and aliasee have different types",
                "Clang -Wattribute-alias: calls through the alias use the alias's type",
            ),
            | Self::SectionMismatch => (
                "section does not match previous declaration",
                "Clang -Wsection: the latest section request applies",
            ),
            | Self::BssFunction | Self::BssInitializer => (
                "BSS section cannot have non-zero bytes",
                "ELF NOBITS sections contain only zero-initialized storage",
            ),
            | Self::BssFixups => (
                "BSS section cannot have fixups",
                "ELF NOBITS sections cannot contain relocations",
            ),
            | Self::InvalidVisibility => (
                "visibility argument not supported",
                "GCC Common Function Attributes: visibility is default, hidden, internal or \
                 protected",
            ),
            | Self::CleanupNotFunction => (
                "cleanup argument is not a function",
                "GCC Common Variable Attributes: cleanup names a function to call with the \
                 variable's address",
            ),
            | Self::CleanupOverloadSet => (
                "cleanup argument is not a single function",
                "Clang Attributes Reference: cleanup requires one function, rather than an \
                 overload set",
            ),
            | Self::CleanupParameterCount => (
                "cleanup function must take one parameter",
                "Clang: the cleanup function receives only a pointer to the variable",
            ),
            | Self::CleanupParameterType => (
                "cleanup function parameter is incompatible with a pointer to the variable",
                "Clang: a pointer to the variable must convert to the parameter type; GCC calls \
                 the function as if by assignment (C99 §6.5.16.1p1)",
            ),
            | Self::JumpIntoCleanupScope => (
                "cannot jump from this goto statement to its label",
                "Clang: a jump into the scope of a cleanup variable would bypass its \
                 initialization; jumps out of it run the cleanup",
            ),
            | Self::SwitchIntoCleanupScope => (
                "cannot jump from switch statement to this case label",
                "Clang: switch dispatch into the scope of a cleanup variable would bypass its \
                 initialization",
            ),
            | Self::IndirectJumpCleanupScope => (
                "cannot jump from this indirect goto statement to one of its possible targets",
                "Clang: an indirect goto cannot enter or leave the scope of a cleanup variable",
            ),
            | Self::AttributeAfterDefinition => (
                "attribute declaration must precede definition",
                "Clang -Wignored-attributes: an attribute that a declaration adds after the \
                 definition is ignored",
            ),
            | Self::UnevaluatedStringPrefix => (
                "encoding prefix on an unevaluated string literal has no effect",
                "Clang -Winvalid-unevaluated-string: the attribute reads the string's characters",
            ),
            | Self::UnsupportedDeclspec => (
                "__declspec attribute is not supported",
                "Clang -Wignored-attributes: __declspec takes the names Clang lists for it, \
                 spelled exactly",
            ),
            | Self::DllImportOnInline => (
                "'dllimport' attribute ignored on inline function",
                "Clang -Wignored-attributes: MinGW defines inline functions in each image that \
                 uses them",
            ),
            | Self::DllImportRedeclaredInline => (
                "redeclared inline; 'dllimport' attribute ignored",
                "Clang -Wignored-attributes: MinGW defines inline functions in each image that \
                 uses them",
            ),
            | Self::DllAttributeOnRedeclaration => (
                "redeclaration should not add a DLL attribute",
                "Clang -Wdll-attribute-on-redeclaration: declare dllimport or dllexport on the \
                 first declaration",
            ),
            | Self::DllAttributeAfterUse => (
                "redeclaration cannot add a DLL attribute",
                "Clang: code before this redeclaration already reached the entity without it",
            ),
            | Self::DllImportDropped => (
                "redeclared without 'dllimport' attribute: previous 'dllimport' ignored",
                "Clang -Winconsistent-dllimport: every declaration of an imported entity must say \
                 dllimport",
            ),
            | Self::DllExportAdded => (
                "redeclared without 'dllimport' attribute: 'dllexport' attribute added",
                "Clang -Winconsistent-dllimport: on the Microsoft ABI a definition of an imported \
                 entity exports it",
            ),
            | Self::DllInternal => (
                "must have external linkage when declared dllimport or dllexport",
                "Microsoft Learn, dllexport, dllimport: only external symbols cross a DLL boundary",
            ),
            | Self::DllThreadLocal => (
                "cannot be thread local when declared dllimport or dllexport",
                "Microsoft Learn, dllexport, dllimport: each image has its own thread-local \
                 storage",
            ),
            | Self::DllImportFunctionDefinition => (
                "dllimport cannot be applied to non-inline function definition",
                "Microsoft Learn, dllexport, dllimport: an imported function is defined in \
                 another image",
            ),
            | Self::DllImportDataDefinition => (
                "definition of dllimport data",
                "Microsoft Learn, dllexport, dllimport: imported data is defined in another image",
            ),
            | Self::DllExportHidden => (
                "hidden visibility cannot be applied to 'dllexport' declaration",
                "Clang: an exported symbol is visible to other images",
            ),
            | Self::DllImportVisibility => (
                "non-default visibility cannot be applied to 'dllimport' declaration",
                "Clang: an imported symbol comes from another image",
            ),
            | _ => ("invalid attribute", "GCC Attribute Syntax"),
        }
    }

    fn statement_expression_explanation(goto: bool) -> (&'static str, &'static str) {
        if goto {
            (
                "goto jumps into a statement expression",
                "GCC Statement Exprs: jumping into a statement expression is not permitted; jumps \
                 within or out of one are permitted",
            )
        } else {
            (
                "switch dispatch jumps into a statement expression",
                "GCC Statement Exprs: an outer switch cannot dispatch to a case or default label \
                 inside a statement expression",
            )
        }
    }
}

impl SemanticErrorKind {
    /// The warning group that controls this diagnostic, if any.
    pub(crate) const fn group(self) -> Option<crate::configuration::DiagnosticGroup> {
        use crate::configuration::DiagnosticGroup as G;
        match self {
            | Self::PointerConstraint(group) => Some(group),
            | Self::ImplicitFunctionDeclaration => Some(G::ImplicitFunctionDeclaration),
            | Self::ImplicitLibraryFunction(_) => Some(G::ImplicitLibraryFunction),
            | Self::ImplicitInt | Self::ImplicitIntReturn | Self::ImplicitIntParameter =>
                Some(G::ImplicitInt),
            | Self::ReturnMismatch => Some(G::ReturnMismatch),
            | Self::VoidReturnMismatch => Some(G::VoidReturnMismatch),
            | Self::DeclarationMissingParameterType => Some(G::DeclarationMissingParameterType),
            | _ => self.attribute_group(),
        }
    }
}

impl SemanticError {
    /// GCC 14's, Clang 23's and TCC's own words for the C type-safety
    /// diagnostics those compilers make errors or warnings by default, in
    /// their diagnostic formats, with the `-W` option each names (none for
    /// TCC, and none for a Clang error that no option controls); other
    /// diagnostics, and the other formats, keep bcc's.
    fn compiler_message<'d>(
        &self,
        context: &Context<'_>,
        arena: &'d Bump,
    ) -> Option<(&'d str, Option<&'static str>)> {
        use crate::configuration::DiagnosticFormat as F;
        let format = context.configuration.diagnostic_format();
        let name = self.name.map_or("", |name| context.string_cache.at(name));
        let c99 = context.configuration.standard() >= crate::configuration::CStandard::C99;
        let group = self.kind.group().and_then(crate::configuration::DiagnosticGroup::name);
        let iso = if c99 {
            "; ISO C99 and later do not support implicit int"
        } else {
            ""
        };
        let message = match (format, self.kind) {
            // The conversion's words, which semantic analysis put together
            // with the types (`Analyzer::conversion_wording`); GCC has one
            // group for all incompatible pointers.
            | (F::Gcc | F::Clang, SemanticErrorKind::PointerConstraint(group))
                if self.name.is_some() =>
            {
                let option = if format == F::Gcc
                    && group == crate::configuration::DiagnosticGroup::IncompatibleFunctionPointerTypes
                {
                    crate::configuration::DiagnosticGroup::IncompatiblePointerTypes.name()
                } else {
                    group.name()
                };
                return Some((arena.alloc_str(name), option));
            },
            | (F::Gcc | F::Tcc, SemanticErrorKind::ImplicitFunctionDeclaration) =>
                format_in!(arena, "implicit declaration of function '{name}'"),
            | (F::Gcc, SemanticErrorKind::ImplicitLibraryFunction(_)) if c99 =>
                format_in!(arena, "implicit declaration of function '{name}'"),
            | (F::Gcc, SemanticErrorKind::ImplicitLibraryFunction(_)) => {
                let message = format_in!(
                    arena,
                    "incompatible implicit declaration of built-in function '{name}'"
                );
                return Some((message, Some("builtin-declaration-mismatch")));
            },
            | (F::Tcc, SemanticErrorKind::ImplicitLibraryFunction(_)) =>
                format_in!(arena, "implicit declaration of function '{name}'"),
            | (F::Clang, SemanticErrorKind::ImplicitFunctionDeclaration) if c99 => format_in!(
                arena,
                "call to undeclared function '{name}'; ISO C99 and later do not support implicit \
                 function declarations"
            ),
            | (F::Clang, SemanticErrorKind::ImplicitFunctionDeclaration) =>
                format_in!(arena, "implicit declaration of function '{name}'"),
            | (F::Clang, SemanticErrorKind::ImplicitLibraryFunction(ty)) if c99 => format_in!(
                arena,
                "call to undeclared library function '{name}' with type '{}'; ISO C99 and later \
                 do not support implicit function declarations",
                context.string_cache.at(ty)
            ),
            | (F::Clang, SemanticErrorKind::ImplicitLibraryFunction(ty)) => format_in!(
                arena,
                "implicitly declaring library function '{name}' with type '{}'",
                context.string_cache.at(ty)
            ),
            | (F::Gcc, SemanticErrorKind::ImplicitInt) =>
                format_in!(arena, "type defaults to 'int' in declaration of '{name}'"),
            | (F::Gcc, SemanticErrorKind::ImplicitIntReturn) => "return type defaults to 'int'",
            | (F::Gcc, SemanticErrorKind::ImplicitIntParameter) =>
                format_in!(arena, "type of '{name}' defaults to 'int'"),
            | (F::Clang, SemanticErrorKind::ImplicitInt | SemanticErrorKind::ImplicitIntReturn) =>
                format_in!(arena, "type specifier missing, defaults to 'int'{iso}"),
            | (F::Clang, SemanticErrorKind::ImplicitIntParameter) => format_in!(
                arena,
                "parameter '{name}' was not declared, defaults to 'int'{iso}"
            ),
            | (F::Gcc, SemanticErrorKind::ReturnMismatch) =>
                "'return' with no value, in function returning non-void",
            | (F::Gcc, SemanticErrorKind::VoidReturnMismatch) =>
                "'return' with a value, in function returning void",
            | (F::Clang, SemanticErrorKind::ReturnMismatch) =>
                format_in!(arena, "non-void function '{name}' should return a value"),
            | (F::Clang, SemanticErrorKind::VoidReturnMismatch) =>
                format_in!(arena, "void function '{name}' should not return a value"),
            | (F::Tcc, SemanticErrorKind::ReturnMismatch) => "'return' with no value",
            | (F::Tcc, SemanticErrorKind::VoidReturnMismatch) => "void function returns a value",
            | (F::Gcc, SemanticErrorKind::DeclarationMissingParameterType) =>
                "parameter names (without types) in function declaration",
            | (F::Clang, SemanticErrorKind::TypeSpecifierRequired) =>
                return Some(("a type specifier is required for all declarations", None)),
            | _ => return None,
        };
        Some((message, if format == F::Tcc { None } else { group }))
    }

    /// TCC's own words for the diagnostics its users meet most, in TCC's
    /// diagnostic format (TCC mob, `tccgen.c` and `tccpp.c`); other
    /// diagnostics keep bcc's.
    fn tcc_message<'d>(&self, context: &Context<'_>, arena: &'d Bump) -> Option<&'d str> {
        if context.configuration.diagnostic_format() != crate::configuration::DiagnosticFormat::Tcc
        {
            return None;
        }
        let name = self.name.map(|name| context.string_cache.at(name));
        Some(match (self.kind, name) {
            | (SemanticErrorKind::UndeclaredIdentifier, Some(name)) =>
                format_in!(arena, "'{name}' undeclared"),
            | (SemanticErrorKind::DuplicateDeclaration, Some(name)) =>
                format_in!(arena, "redeclaration of '{name}'"),
            | (SemanticErrorKind::TypedefRedeclaration, Some(name)) =>
                format_in!(arena, "incompatible redefinition of '{name}'"),
            | (SemanticErrorKind::IncompatibleDeclaration, Some(name)) =>
                format_in!(arena, "incompatible types for redefinition of '{name}'"),
            | (SemanticErrorKind::DuplicateDefinition, Some(name)) =>
                format_in!(arena, "redefinition of '{name}'"),
            | (SemanticErrorKind::UndefinedLabel, Some(name)) =>
                format_in!(arena, "label '{name}' used but not defined"),
            | (SemanticErrorKind::BreakOutsideLoopOrSwitch, _) => "cannot break",
            | (SemanticErrorKind::ContinueOutsideLoop, _) => "cannot continue",
            | (SemanticErrorKind::ExpectedModifiableLvalue, _) => "lvalue expected",
            | (
                SemanticErrorKind::PointerConstraint(
                    crate::configuration::DiagnosticGroup::IncompatiblePointerTypes,
                ),
                _,
            ) => "assignment from incompatible pointer type",
            | _ => return None,
        })
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
        let message = if let SemanticErrorKind::ImplicitLibraryFunction(ty) = self.kind {
            format_in!(
                arena,
                "implicitly declaring library function {} with type `{}`",
                quote_spelling(self.name.map_or("", |name| context.string_cache.at(name))),
                context.string_cache.at(ty)
            )
        } else if matches!(
            self.kind,
            SemanticErrorKind::CallAttribute | SemanticErrorKind::CallAttributeWarning
        ) {
            self.name.map_or(message, |name| {
                arena.alloc_str(context.string_cache.at(name))
            })
        } else if matches!(
            self.kind,
            SemanticErrorKind::BssFunction
                | SemanticErrorKind::BssInitializer
                | SemanticErrorKind::BssFixups
        ) {
            format_in!(
                arena,
                "BSS section '{}' cannot have {}",
                self.name.map_or("", |name| context.string_cache.at(name)),
                if self.kind == SemanticErrorKind::BssFixups {
                    "fixups"
                } else {
                    "non-zero bytes"
                }
            )
        } else if let SemanticErrorKind::ConstantConversion(text)
        | SemanticErrorKind::FloatOverflow(text)
        | SemanticErrorKind::LiteralConversion(text) = self.kind
        {
            arena.alloc_str(context.string_cache.at(text))
        } else if let SemanticErrorKind::ImmediateOutOfRange { value, low, high } = self.kind {
            format_in!(
                arena,
                "argument value {value} is outside the valid range [{low}, {high}]"
            )
        } else if let SemanticErrorKind::PragmaPackShow(value) = self.kind {
            format_in!(arena, "value of #pragma pack(show) == {value}")
        } else if matches!(
            self.kind,
            SemanticErrorKind::ReturnMismatch | SemanticErrorKind::VoidReturnMismatch
        ) {
            // The name, the function's, is for Clang's wording.
            message
        } else if let Some(name) = self.name {
            format_in!(
                arena,
                "{message}: {}",
                quote_spelling(context.string_cache.at(name))
            )
        } else {
            message
        };
        let mut suffixed = false;
        let message = if self.kind == SemanticErrorKind::PredefinedOutsideFunction {
            suffixed = true;
            format_in!(
                arena,
                "{message} [-Wpredefined-identifier-outside-function]"
            )
        } else {
            message
        };
        let group = self.kind.group();
        let message =
            if let Some(group) = group.and_then(crate::configuration::DiagnosticGroup::name) {
                suffixed = true;
                format_in!(arena, "{message} [-W{group}]")
            } else {
                message
            };
        let (message, suffixed) = match self.compiler_message(context, arena) {
            | Some((text, Some(group))) => (format_in!(arena, "{text} [-W{group}]"), true),
            | Some((text, None)) => (text, false),
            | None => match self.tcc_message(context, arena) {
                | Some(tcc) => (tcc, false),
                | None => (message, suffixed),
            },
        };
        let explanation = Explanation::new(arena, message);
        let explanation = if suffixed {
            explanation.option_suffix()
        } else {
            explanation
        };
        let mut diagnostic = explanation
            .label(self.kind.label())
            .note(note)
            .at(self.severity(), source);
        if let Some(previous) = self.previous {
            let label = match self.kind {
                | SemanticErrorKind::DuplicateLabel | SemanticErrorKind::DuplicateCase =>
                    "previous label is here",
                | SemanticErrorKind::JumpIntoVariableScope
                | SemanticErrorKind::SwitchIntoVariableScope
                | SemanticErrorKind::IndirectJumpIntoVariableScope =>
                    "variably modified identifier is declared here",
                | SemanticErrorKind::JumpIntoStatementExpression
                | SemanticErrorKind::SwitchIntoStatementExpression
                | SemanticErrorKind::IndirectJumpIntoStatementExpression =>
                    "statement expression begins here",
                | SemanticErrorKind::JumpIntoCleanupScope
                | SemanticErrorKind::SwitchIntoCleanupScope => "cleanup variable is declared here",
                | SemanticErrorKind::IndirectJumpCleanupScope => "possible target is here",
                | SemanticErrorKind::AliasKind | SemanticErrorKind::AliasTypeMismatch =>
                    "aliasee is declared here",
                | SemanticErrorKind::SectionMismatch => "previous attribute is here",
                | SemanticErrorKind::InitializerOverride
                | SemanticErrorKind::PartialInitializerOverride =>
                    "previous initialization is here",
                | SemanticErrorKind::FlexibleArrayBraces
                | SemanticErrorKind::FlexibleArrayDesignator =>
                    "initialized flexible array member is here",
                | SemanticErrorKind::AttributeAfterDefinition => "previous definition is here",
                | _ => "previous declaration is here",
            };
            diagnostic = diagnostic.secondary(previous, label);
        }
        diagnostic
    }
}

impl fmt::Display for SemanticError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.kind.explanation().0)
    }
}

impl std::error::Error for SemanticError {}

impl GetSeverity for SemanticError {
    fn severity(&self) -> ErrorSeverity {
        if let Some(severity) = self.extension_severity {
            return severity;
        }
        if matches!(
            self.kind,
            SemanticErrorKind::ImplicitLibraryFunction(_)
                | SemanticErrorKind::QualifiedFunction
                | SemanticErrorKind::MainSignature
                | SemanticErrorKind::TentativeArrayAssumedOne
                | SemanticErrorKind::UnusedStaticFunction
                | SemanticErrorKind::PragmaPackMalformed
                | SemanticErrorKind::PragmaPackPopEmpty
                | SemanticErrorKind::PragmaPackPopNameAlignment
                | SemanticErrorKind::PragmaPackUnterminated
                | SemanticErrorKind::PragmaPackShow(_)
                | SemanticErrorKind::PragmaPackIgnored
                | SemanticErrorKind::EmptyCaseRange
                | SemanticErrorKind::ReadOnlyLocation
                | SemanticErrorKind::PredefinedOutsideFunction
                | SemanticErrorKind::InvalidAtomicOrder
                | SemanticErrorKind::InvalidAtomicFailureOrder
                | SemanticErrorKind::AtomicBufferQualifiers
                | SemanticErrorKind::UnknownAttribute
                | SemanticErrorKind::IgnoredAttribute
                | SemanticErrorKind::IgnoredConvention
                | SemanticErrorKind::ReservedPriority
                | SemanticErrorKind::AliasTypeMismatch
                | SemanticErrorKind::SectionMismatch
                | SemanticErrorKind::InvalidVisibility
                | SemanticErrorKind::IgnoredTransparentUnion
                | SemanticErrorKind::TransparentUnionNotUnion
                | SemanticErrorKind::TransparentUnionNotDefinition
                | SemanticErrorKind::TransparentUnionPlacement(_)
                | SemanticErrorKind::InitializerOverride
                | SemanticErrorKind::PartialInitializerOverride
                | SemanticErrorKind::ConstantConversion(_)
                | SemanticErrorKind::FloatOverflow(_)
                | SemanticErrorKind::PedanticOverflow
                | SemanticErrorKind::LiteralConversion(_)
                | SemanticErrorKind::ExcessString
                | SemanticErrorKind::AttributeAfterDefinition
                | SemanticErrorKind::UnevaluatedStringPrefix
                | SemanticErrorKind::UnsupportedDeclspec
                | SemanticErrorKind::DllImportOnInline
                | SemanticErrorKind::DllImportRedeclaredInline
                | SemanticErrorKind::DllAttributeOnRedeclaration
                | SemanticErrorKind::DllImportDropped
                | SemanticErrorKind::DllExportAdded
        ) {
            ErrorSeverity::Warning
        } else {
            ErrorSeverity::Error
        }
    }
}

impl GetPosition for SemanticError {
    fn position(&self, context: &Context<'_>) -> SourcePosition {
        self.source_vectors.position(context)
    }
}

impl GetSourceVectors for SemanticError {
    fn source_vectors(&self, _context: &mut Context<'_>) -> SourceVectors {
        self.source_vectors
    }
}
