# bcc-rust Compiler Domain

This glossary defines the canonical language for bcc-rust's C front end and its agreed parser direction.

## Translation pipeline

**Translation phase**:
A stage that consumes one source representation and yields the next while retaining enough provenance to diagnose the original input.

**Initial processing**:
The character-level phase that normalizes source text before preprocessing-token recognition, including line endings, trigraphs, escaped newlines, and comments.

**Preprocessing token**:
A lexical unit recognized before macro expansion and directive handling; it preserves spellings and categories needed by the C preprocessor.
_Avoid_: Token

**Preprocessing**:
The phase that expands macros, executes directives, resolves includes and conditional groups, and converts surviving preprocessing tokens into parser-facing tokens.

**Token**:
A parser-facing lexical unit whose category already distinguishes identifiers, keywords, operators, and typed literals.
_Avoid_: Preprocessing token

**Source vector**:
A segment of original-source provenance attached to generated characters, preprocessing tokens, tokens, and diagnostics. A value may carry multiple source vectors when preprocessing combines or transforms input.
_Avoid_: Source span

**Context**:
The compilation-wide state shared by translation phases, including interned spellings, source files, source vectors, and pending diagnostics.
_Avoid_: Parser context

## C syntax

**Translation unit**:
The complete sequence of external declarations produced from one preprocessed C input. It is not identical to a physical source file because inclusion and macro expansion may contribute input.
_Avoid_: Source file

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

**Arena**:
An owning collection for compiler-domain objects whose relationships are represented by compact handles rather than nested ownership.

**Index handle**:
A typed numeric reference to one object or contiguous object range owned by an arena. Handles distinguish domains such as expressions, statements, declarations, and types.
_Avoid_: Pointer

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

The Phase 04 parser implements this vocabulary through declarations, function
definitions, statements, expressions, type names, and initializers.
`parser-roadmap.md` keeps the authoritative boundary for the remaining closure
work.

**ParserMachine** *(implemented as `Parser`)*:
The single driver that owns the buffered token cursor, control stack, typed child return, syntax arenas, file-scope name classification, and diagnostic/recovery state for language parsing.
_Avoid_: Recursive-descent parser

**ParseFrame** *(implemented)*:
A resumable state machine for one grammar family. Current families cover external declarations, declarations, declarators, parameters, tags, function definitions, compound statements, statements, expressions, type names, and initializers.
_Avoid_: Grammar call

**ExpressionFrame** *(implemented)*:
The parse frame that owns the Double-E operator and operand stacks for a language expression and returns an expression result to its parent frame.

**ParseAction** *(implemented)*:
A small owned instruction returned by a frame to the driver: consume input, push a child frame, reduce a value, reprocess lookahead, or recover at a synchronization set.

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
yield `ExternalDeclaration::RecoveredDeclaration` with the declaration's arena
handle. Later analysis may inspect the repaired tree to find additional
problems, while the distinct variant prevents it from being mistaken for fully
valid syntax.

**Recovered function definition**:
A function-definition AST retained after a hard error in its head, old-style declaration list, or body. The distinct external-declaration variant preserves inspectable syntax without presenting it as valid.

**Error node** *(reserved)*:
A provenance-only syntax placeholder for malformed input from which no
meaningful AST can be recovered. Current declaration recovery always constructs
a recovered declaration, so `ExternalDeclaration::Error` is reserved for a
future unrecoverable grammar path.

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
