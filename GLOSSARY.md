# bcc-rust Compiler Domain

This glossary defines the canonical language for bcc-rust's C front end.

## Translation pipeline

**Translation phase**:
A stage that consumes one source representation and yields the next while retaining enough provenance to diagnose the original input.

**Initial processing**:
The character-level phase that normalizes source text before preprocessing-token recognition: line endings, trigraphs, and escaped newlines (C99 translation phases 1 and 2). Comments are not recognized here, so a spliced `/` and `*` still open one and string literals never contain one.

**Preprocessing token**:
A lexical unit recognized before macro expansion and directive handling; it preserves spellings and categories needed by the C preprocessor. Recognition replaces each comment with whitespace.
_Avoid_: Token

**Batch pipeline**:
How the front end schedules translation phases 1 through 7. Each opened source file is lexed in full, the whole translation unit is preprocessed, and only then is it parsed; preprocessing diagnostics precede parser diagnostics.

**Header name**:
The `<…>` or `"…"` operand of `#include` (C99 §6.4.7). It is interpreted in the directive from ordinary preprocessing tokens, using the source text between delimiters for a written operand or combined token spellings for a macro-expanded one.

**Preprocessing**:
The phase that expands macros, executes directives, resolves includes and conditional groups, and converts surviving preprocessing tokens into parser-facing tokens.

**Token**:
A parser-facing lexical unit whose category already distinguishes identifiers, keywords, operators, and typed literals.
_Avoid_: Preprocessing token

**Source vector**:
A segment of original-source provenance attached to generated characters, preprocessing tokens, tokens, and diagnostics. A value may carry multiple ordered source vectors when preprocessing combines or transforms input; provenance needed after preprocessing is retained for the translation unit.
_Avoid_: Source span

**Context**:
The compilation-wide state shared by translation phases, including interned spellings, source files, source vectors, and pending diagnostics. The object running a phase (the parser, or the preprocessor while it reads input) holds the context exclusively for as long as it runs, rather than receiving it with each call.
_Avoid_: Parser context

**Language mode** (`LanguageMode`):
An ordered ISO C revision and an independent GNU-dialect bit. C89/C90 share
one revision; the C95 amendment is distinct. MSVC feature flags are independent
of the language mode.

**Compiler configuration** (`CompilerConfiguration`):
The value object held by `Context` that owns language mode, MSVC feature flags,
extension diagnostic policy, and derived feature bits. Feature acceptance means
the syntax may be consumed, including as an extension; native availability means
it belongs to the selected revision or dialect. Neither means its implementation
is complete; the language-standards.md matrix records that status.

**Extension policy** (`ExtensionPolicy`):
How an accepted but non-native feature is diagnosed: Allow (silent, the
default), Warn (`-pedantic`), or Deny (`-pedantic-errors`). Each diagnostic
names the feature's origin: a later ISO revision, a removed earlier one, GNU,
or MSVC. GNU and MSVC features stay non-ISO when enabled. Deny reports an
error but keeps the syntax, so the parser still builds and recovers it.

## Storage and lifetimes

**Translation-unit arena** (`'tu`):
Storage retained across preprocessing and parsing for one translation unit, including source text, diagnostic data, and the syntax tree. It ends when that translation unit is finished; retained provenance shares its lifetime in dedicated regions.

**Phase arena**:
Storage owned by one translation phase for its working state and released when that phase ends. The preprocessing arena (`'pp`) spans preprocessing, while the parse arena (`'parse`) spans parsing; neither owns the retained syntax tree.

**Preprocessing arena** (`'pp`):
The phase arena for lexed files, macro definitions, and include and conditional state. It ends after the whole translation unit has been preprocessed, before parsing begins; source provenance that must survive it is retained separately.

**Expansion arena** (`'x`):
Resettable storage for temporary macro-expansion work within preprocessing. It is reused between completed top-level expansions while the preprocessing arena keeps state that must survive them.

**Expansion scope** (`'x`):
The lifetime of one active expansion's temporary references and state. It ends before the expansion arena is reset; references that must survive an expansion belong to a longer-lived arena.

**Parse arena** (`'parse`):
The phase arena for parser frames, open scopes, and recovery state. It ends when parsing finishes; syntax reachable from the parsed translation unit remains in the translation-unit arena.

**Dedicated region**:
Independent storage for a buffer that exists once per compilation and must grow without moving. It lasts as long as that buffer is needed; per-file temporary buffers instead belong in a phase arena.

**Commit follows use**:
The boundary between reserved address space and memory made available for use: a growing arena or buffer commits pages just ahead of written data, rather than its full reserved capacity. Its address space is itself reserved only when it is first used. On Linux, regions commit whole 2 MiB transparent huge pages, so commit may run up to one huge page ahead of written data, and a region in use commits at least one huge page.

**Tail vector**:
A temporary, growable sequence occupying the unused tail of an arena while its final length is unknown. When finished, only its written contents remain in the arena; unfinished contents are abandoned.

**File-scope typedef set**:
The parser's classification of names whose latest file-scope declaration is a typedef, retained through parsing in a dedicated region rather than the parse arena. An ordinary file-scope declaration removes that name; nested scopes may temporarily shadow it without changing the file-scope classification.

**Chunking seam**:
The boundary reserved for a future pipeline that preprocesses and parses successive token chunks. State that crosses a chunk boundary remains in its phase arena; expansion storage may reset after active expansions finish, and token storage after parsing consumes the chunk.

## C syntax

**Translation unit**:
The complete sequence of external declarations produced from one preprocessed C input. It is not identical to a physical source file because inclusion and macro expansion may contribute input.
_Avoid_: Source file

**Parsed translation unit** *(implemented as `ParsedTranslationUnit`)*:
The parser result: the source-ordered external roots, which borrow the syntax
tree from the translation-unit arena. It is the shared caller and behavior-test
seam.

**Syntax tree**:
The declarations, definitions, statements, expressions, type names, and
initializers reachable from the parsed roots. Nodes live in the
translation-unit arena and refer to their children by reference and by
immutable list; a node never changes once allocated. The raw Rust debug form
is an opt-in debug view, not the normal consumer interface.

**External declaration**:
A top-level declaration or function definition within a translation unit.

**Function definition**:
An external declaration consisting of declaration specifiers, a function
declarator, an optional old-style parameter declaration list, and a compound
statement body. A function prototype ending in a semicolon remains a
declaration.

**Declaration**:
A construct that introduces or describes identifiers, types, storage duration, or linkage through declaration specifiers and optional init-declarators.
_Avoid_: Declarator

**Declaration specifiers**:
The storage-class, type-specifier, type-qualifier, and function-specifier portion shared by the declarators in a declaration.

**Declarator**:
The part of a declaration that binds an identifier, when present, and derives pointer, array, or function shape from declaration specifiers.
_Avoid_: Declaration

**Type name**:
A declaration-specifier sequence plus an optional abstract declarator that denotes a type without declaring an identifier.
_Avoid_: Typedef name

**Initializer**:
The expression or brace-enclosed initializer list associated with an object declarator.

**Statement**:
A function-body construct controlling evaluation, selection, iteration, jumps, labels, or compound sequencing.

**Compound statement**:
A brace-delimited statement containing an ordered sequence of block items and
introducing block scope.
_Avoid_: Block statement

**Block item**:
One declaration or statement in a compound statement. Block items preserve
source order because C99 permits declarations and statements to interleave.

**Expression**:
A syntax tree for operators and operands that may compute a value, designate an object or function, or produce side effects.

**Abstract syntax tree (AST)**:
The structured syntax representation produced by language parsing. It records grammatical form and provenance without deciding every semantic property of the program.

**Index handle**:
A small typed numeric key into one specific structure, kept where identity or compactness matters more than direct access: interned strings, literal values, source files, and source-vector ranges. Syntax nodes are references, not handles.

## Names, scopes, and ambiguity

**Typedef name**:
An identifier currently bound by a `typedef` declaration and therefore usable as a type specifier.
_Avoid_: Type name

**Scope**:
The region in which a binding is visible and may shadow an outer binding. Parser-visible scope state must distinguish typedef names from ordinary identifiers at each token position.

**Typedef ambiguity**:
The C grammar choice that depends on whether an identifier is currently a typedef name, including declaration-versus-expression and cast-versus-grouping decisions.

## Expression reduction

**Preprocessor expression**:
The integer expression accepted by conditional preprocessing directives. It has its own legal operands, evaluation values, diagnostics, and treatment of undefined identifiers.

**Preprocessor expression evaluator**:
The current two-state, operator-stack and operand-stack reducer that evaluates preprocessor expressions during conditional preprocessing.

**Double-E reducer**:
A non-recursive precedence reducer that alternates operand-expected and operator-expected states while reducing separate operator and operand stacks. The preprocessor evaluator and language parser each own their reducer implementation because their operands, legal operators, outputs, and diagnostics differ.

**Expression dialect**:
The grammar and reduction policy of one expression parser. The preprocessing dialect evaluates integer values, while the independently implemented language dialect constructs C expression AST nodes and accepts the full language operator set.

## Parser stack-machine vocabulary

The completed Phase 05 parser implements this vocabulary through whole
translation units, declarations, function definitions, statements,
expressions, type names, initializers, and recovery.

**ParserMachine** *(implemented as `Parser`)*:
The single driver that holds the context and owns the token cursor, control stack, typed child return, syntax-node count, file-scope name classification, and diagnostic/recovery state for language parsing. It allocates syntax nodes in the translation-unit arena and its working memory in the parse arena.
_Avoid_: Recursive-descent parser

**ParseFrame** *(implemented)*:
A resumable state machine for one grammar family. Current families cover external declarations, declarations, declarators, parameters, tags, function definitions, compound statements, statements, expressions, type names, and initializers. Three delimiter-owning families serve later and vendor syntax, running their expression, type-name, and compound children on the same stack: `ModernFrame` (ISO C11 through C2y keyword operands such as `_Alignas`, `_Alignof`, `_Atomic(...)`, `typeof`, `_BitInt(...)`, and `_Countof`; generic selections; static assertions; and attribute specifiers in all three syntaxes, `[[...]]`, `__attribute__((...))`, and `__declspec(...)`), `GnuFrame` (GNU assembly, builtins, and `__label__` declarations), and `MsvcFrame` (MSVC SEH and inline-assembly statements). Other extensions add phases to the ordinary families.
_Avoid_: Grammar call

**ExpressionFrame** *(implemented)*:
The parse frame that owns the Double-E operator and operand stacks for a language expression and returns an expression result to its parent frame.

**ParseAction** *(implemented)*:
A small owned instruction returned by a frame to the driver: consume input, push a child frame, reduce a value, reprocess lookahead, or recover at a synchronization set. After a forward phase change that needs no driver work, a frame may instead continue: frame dispatch runs it again at once with the same lookahead.

**ParseValue** *(implemented)*:
The typed result passed from a completed child frame to its parent. Variants cover declarations, function definitions, compound statements, statements, expressions, constant expressions, type names, and initializers, plus the extension frames' results: a modern value (keyword operand, generic selection, static assertion, or attribute specifier) and a GNU value (assembly, builtin, or local-label list). `MsvcFrame` returns an ordinary statement.

**Deferred child** *(historical phase seam)*:
A present grammar child whose parser belonged to a later phase, retained as a typed source-backed slot rather than confused with syntactic absence. Phase 04 removed these seams from supported C99 grammar paths.
_Avoid_: Skipped syntax

**Synchronization set** *(implemented)*:
The tokens at which a particular frame can safely resume or unwind after malformed input, paired with a legal recovery target.
_Avoid_: Global recovery point

**Recovered declaration** *(implemented at the external-declaration boundary)*:
A declaration AST retained after one or more hard syntax diagnostics and local
repair. Migrated frames finish synchronization and provenance collection, then
yield `ExternalDeclaration::RecoveredDeclaration` with the declaration's
syntax. Later analysis may inspect the repaired tree to find additional
problems, while the distinct variant prevents it from being mistaken for fully
valid syntax.

**Recovered function definition**:
A function-definition AST retained after a hard error in its head, old-style declaration list, or body. The distinct external-declaration variant preserves inspectable syntax without presenting it as valid.

**Error node** *(implemented)*:
A provenance-only syntax placeholder for malformed input from which no
meaningful AST can be recovered. Pure top-level garbage yields
`ExternalDeclaration::Error`; meaningful malformed declarations retain typed
recovered syntax instead.

**Structured parser diagnostic**:
A parser diagnostic with a symbolic code, severity, active frame, expected and
found syntax categories, the found token's source spelling, primary provenance,
optional ranges and related locations, an optional `;` insertion point, and
recovery summary. Diagnostics are delivered in emission FIFO. The code, frame,
and recovery owner are machine-facing; rendering shows only source spellings
and fixed wording.

**Rendered diagnostic**:
The user-facing form of any phase's error: a message, a primary source range
with an optional label, secondary labelled ranges, notes (typically the C99
rule), and help. Rendering never exposes internal representation.
_Avoid_: Debug-formatted error

**Parser resource limit**:
A configured ceiling for external roots, syntax nodes, active frame depth, or
stored source-vector segments. Crossing a ceiling emits a stable resource diagnostic, clears transient parser
state, and returns an explicit external error root rather than panicking.

**Syntax inspection view**:
The deterministic, iterative, source-oriented rendering selected by
`--syntax-tree`. `--raw-syntax` is the separate storage-debugging view.

## Compiler boundaries

**Syntax parsing**:
The phase that recognizes the token stream and builds declarations, declarators, statements, expressions, and other AST forms while using only the name classification required by C grammar.
_Avoid_: Semantic analysis

**Semantic analysis**:
The later phase that resolves types and bindings, checks constraints and conversions, and determines properties not required to choose a grammatical form.
_Avoid_: Syntax parsing

**Backend**:
The post-front-end work that lowers validated program meaning into an executable target representation.
_Avoid_: Parser

## GNU syntax ownership

**GNU assembly node**:
Syntax for a GNU assembly statement, file assembly or declarator assembly label.
Original template, qualifier, constraint and clobber tokens accompany parsed C
operand expressions and label identifiers; target validation belongs to analysis.

**GNU builtin node**:
A reserved builtin production with typed expression/type operands and, for
`__builtin_offsetof`, a member path. It is distinct from an ordinary function call.

**Statement expression**:
The GNU `({ block-items })` expression. Its compound child owns block scope;
its expression owner retains the enclosing parentheses. Value/type analysis is deferred.

**Nested function definition**:
A GNU block item containing a complete function definition. Its parser frames
isolate function-local label/switch state while preserving enclosing typedef visibility.

**Extension marker**:
GNU `__extension__` syntax wrapping an expression or declaration and suppressing
pedantic extension diagnostics within that owner. It does not repair malformed syntax.

## Semantic analysis results

**Semantic translation unit** (`SemanticTranslationUnit<'tu>`):
The retained result of semantic analysis after parsing: a canonical type graph,
nominal tags and members, resolved declaration occurrences, scope identities,
resolved type names, parameter metadata, typed expressions and contextual
conversions. It borrows the translation-unit arena.
Syntax remains immutable and separately inspectable.

**Semantic working arena** (`'s`):
A phase arena for semantic continuations, integer-evaluation values, hash-cons
lookup, visible-binding lookup and scope restoration. It is dropped before the
semantic translation unit is returned; no retained result borrows it.

**Canonical type identity** (`TypeId`):
An unqualified type graph index plus a compact qualifier set. Structurally identical
derived types share their unqualified identity. Distinct tagged types retain nominal
identity even when their members/layout are equal. Identity is stronger than C type
compatibility, which may form a composite type across different array/prototype shapes.

**Nominal tag** (`Tag`):
The identity and completion state of one structure, union or enumeration. Tag lookup
has a separate namespace from ordinary bindings; member names belong to their own
nominal aggregate. Completion adds retained members and layout without changing identity.

**Semantic binding** (`Binding`):
One source declaration occurrence with its resolved type, semantic scope, ordinary
binding kind, linkage and storage duration. A semantic scope is independent of the
parser's typedef-name classification; redeclarations merge compatible linked types.

**Target layout** (`TargetLayout`):
The explicit scalar, pointer and alias representation used by token conversion and
semantic layout, defaulting to x86-64 System V LP64. It describes the C target rather
than Rust's host ABI.

**Unanalyzed type** (`TypeKind::Unknown`):
A result for failed operands or accepted syntax whose semantics are outside the
implemented stage. It suppresses dependent constraints without pretending that an
extension has a C99 scalar representation. Unknown does not certify valid input.

**Typed expression result** (`ExpressionInfo`):
A retained side-table record borrowing an immutable syntax expression. It carries
the original type/category, binding and bit-field metadata where applicable,
constant eligibility and available constant values. Records have deterministic
child-before-parent order; scratch lookup is keyed by syntax identity.

**Value category** (`ValueCategory`):
An expression's relationship to an object or value before contextual conversions:
lvalue, modifiable lvalue, function designator or rvalue. A modifiable lvalue is
the assignable subset of lvalues, including complete non-const object constraints.

**Contextual conversion** (`Conversion`):
A retained operation on an expression at a use site: lvalue conversion, decay,
arithmetic/assignment conversion or default argument promotion. It records the
destination type without changing the syntax expression's original category.

**Integer constant expression** (ICE):
An integer expression satisfying C99 operand and operator restrictions as well as
having an evaluable value. A folded integer value alone does not establish ICE
eligibility. Enumerators, bit-fields, case labels and designators require ICEs.

**Constant-expression class** (`ConstantClass`):
Arithmetic, address or nonconstant eligibility for static initialization, distinct
from strict ICE eligibility. Address eligibility identifies permitted static
designations without evaluating the stored value of an object.

**Initializer current object** (`Current`):
An arena cursor identifying the subobject to receive the next initializer under
its containing brace pair. Designators reset the path; brace elision descends it;
sequential initialization advances and unwinds it. It validates syntax and infers
array bounds without materializing backend stores.

**Defining function derivation**:
The first type derivation outward from a function definition's identifier,
including through grouping parentheses. Its immutable suffix identity selects
the body parameters and K&R signature. A function suffix in the return type,
or a function type obtained solely from a typedef, is not that derivation.

**Definition record** (`Definition`):
A finalized definition associated with a semantic binding occurrence, separate
from declarations of the same entity. It distinguishes explicit object/function
definitions, inline-only bodies, completed tentative objects and synthesized
function-name arrays. Tentative records use the final composite object type.

**Variable scope path**:
A persistent chain of declarations of variably modified identifiers currently
in scope. It includes VLA objects, pointers to VLAs and variably modified typedefs.
A label or jump retains a path snapshot after lexical scope exit; path comparison
detects entry into a declaration's scope without recursive tree traversal.

**Function label identity**:
A label's identity in its separate function-wide namespace. GNU local-label
declarations introduce lexical identities that can shadow an ordinary label or
be referenced by a nested function. Label identities are independent of ordinary
object/function bindings and of label spellings in other functions.

**Inline definition**:
In the C99 model, a function body with external linkage whose file-scope
declarations all specify inline without extern. It provides an inline-only body,
not an external definition. Later declarations can change that classification;
static inline functions have internal linkage and are not inline definitions in
this specific sense.
