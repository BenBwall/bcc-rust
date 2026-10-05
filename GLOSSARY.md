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
The compilation-wide state shared by translation phases, including interned spellings, source files, source vectors, and pending diagnostics.
_Avoid_: Parser context

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
The boundary between reserved address space and memory made available for use: a growing arena or buffer commits pages just ahead of written data, rather than its full reserved capacity.

**Tail vector**:
A temporary, growable sequence occupying the unused tail of an arena while its final length is unknown. When finished, only its written contents remain in the arena; unfinished contents are abandoned.

**File-scope typedef set**:
The parser's classification of names whose latest file-scope declaration is a typedef, retained through parsing. An ordinary file-scope declaration removes that name; nested scopes may temporarily shadow it without changing the file-scope classification.

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
translation-unit arena and refer to their children by reference and slice; a
node never changes once allocated. The raw Rust debug form is an opt-in debug
view, not the normal consumer interface.

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
The single driver that owns the buffered token cursor, control stack, typed child return, syntax-node count, file-scope name classification, and diagnostic/recovery state for language parsing. It allocates syntax nodes in the translation-unit arena and its working memory in the parse arena.
_Avoid_: Recursive-descent parser

**ParseFrame** *(implemented)*:
A resumable state machine for one grammar family. Current families cover external declarations, declarations, declarators, parameters, tags, function definitions, compound statements, statements, expressions, type names, and initializers.
_Avoid_: Grammar call

**ExpressionFrame** *(implemented)*:
The parse frame that owns the Double-E operator and operand stacks for a language expression and returns an expression result to its parent frame.

**ParseAction** *(implemented)*:
A small owned instruction returned by a frame to the driver: consume input, push a child frame, reduce a value, reprocess lookahead, or recover at a synchronization set. After a forward phase change that needs no driver work, a frame may instead continue: frame dispatch runs it again at once with the same lookahead.

**ParseValue** *(implemented)*:
The typed result passed from a completed child frame to its parent. Variants cover declarations, function definitions, compound statements, statements, expressions, constant expressions, type names, and initializers.

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
A configured ceiling for external roots, syntax nodes, or active frame depth.
Crossing a ceiling emits a stable resource diagnostic, clears transient parser
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
