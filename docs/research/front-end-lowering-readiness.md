Research date: 2026-10-11.

# Front-end readiness for IR lowering

This note answers one question before the middle-end IR is designed: what a
lowering pass from the front end's output needs, and what the front end
already retains. It is based on reading the source at worktree commit
`8d18e013` (branch `claude/middle-end-ir`). No compiler was run: the only
prebuilt binaries in the main checkout date from 2026-09-30, which is before
the semantic-analysis stack merged, so they do not reflect this code. Line
numbers refer to that commit.

Paths are abbreviated: `sema.rs` is `src/translation_phases/semantic_analysis.rs`,
`sema/x.rs` is a file under `src/translation_phases/semantic_analysis/`,
`syntax.rs` and `declaration_syntax.rs` are under
`src/translation_phases/parsing/`.

## Summary

The front end has most of the facts lowering needs: a typed record for every
expression it visits, explicit conversion records, resolved bindings, a
hash-consed type graph with target layout, record member offsets including
bit-fields, folded constants, and definition records for tentative
definitions. The gaps are mostly in how those facts are connected:

- Lowering cannot name the semantic types today. `types`, `expressions`,
  `functions`, `integer` and `constants` are private modules, and only the
  error types are re-exported (`sema.rs:8-33`, `sema.rs:37-40`).
- No retained index connects syntax to semantics. The expression-identity map
  and the member, parameter and type-name maps all live in the scratch arena
  and are dropped when analysis returns (`sema.rs:397-416`, `sema.rs:452-478`).
- Nothing links a function definition or declarator to its binding, so
  lowering cannot find, for example, the parameter bindings of the function
  it is lowering without searching.
- Nothing reliably says whether the translation unit may be lowered. The
  semantic error counter is dropped (`sema.rs:406`). Unmodeled extensions
  quietly become `Unknown` types with no diagnostic.

Each of these is cheap to fix. Gaps in the language features themselves (VLA
extents, statement expressions, opaque builtins, binary128 arithmetic,
initializer plans, address-constant relocation) matter less for a first
prototype that handles ints, pointers, control flow, calls and structs.

## 1. What `'tu` retains after analysis

### The retained structure

`pipeline::analyze_translation_unit` (`src/pipeline.rs:42-47`) returns
`SemanticTranslationUnit<'tu>` (`sema.rs:192-211`):

| Field | Contents | Keyed by |
| --- | --- | --- |
| `types: Types<'tu>` | `nodes: &[TypeKind]`, `tags: &[&Tag]`, `target: TargetLayout` (`sema/types.rs:213-217`) | `TypeId.index` |
| `bindings: &[Binding]` | every declaration occurrence: name, type, scope, kind, linkage, duration, folded value (`sema.rs:163-171`) | binding index |
| `definitions: &[Definition]` | `{binding, kind}` with kind Object, Function, Inline, Tentative or FunctionName (`sema/functions.rs:43-58`) | binding index |
| `scopes: &[Scope]` | parent and kind (`sema.rs:185-189`) | scope index |
| `type_names: &[(SourceVectors, TypeId)]` | resolved type names | provenance |
| `parameters: &[(SourceVectors, &[Parameter])]` | parameter lists | provenance |
| `expressions: &[ExpressionInfo]` | typed results in postorder (`sema/expressions.rs:82-109`) | `info.expression` pointer |
| `conversions: &[Conversion]` | `{expression, ty, kind}` (`sema/expressions.rs:136-140`) | `conversion.expression` pointer |
| `tag_declarations: &[(usize, usize)]` | tag declaration occurrences | none |

The syntax tree survives separately in `'tu` as `ParsedTranslationUnit`
(`parsing.rs:280-287`). Nodes are immutable `&'tu` references, so pointer
identity is stable for the whole compilation. Literal text, wide-literal units
and interned names live in `Context<'tu>` (`literal_units`,
`wide_literal_units` in `context/literals.rs:28-35`, and `string_cache.at`).

What is dropped with the scratch arena (`sema.rs:379-427`):
`expression_indices` (syntax identity to expression record), `member_indices`
(record and member name to field index), `parameters` (function suffix to
parameter list and prototype scope), `resolved_type_names`,
`register_bindings`, the visible and external lookup maps, the
linkage-entity table (`functions::State`, `sema/functions.rs:130-140`), label
maps and switch case lists (`sema/statements.rs:70-122`), and the
`semantic_errors` counter.

### From an expression node to its facts

Each `ExpressionInfo` carries:

- `ty`, the expression's own type before contextual conversion.
- `category`, one of Lvalue, ModifiableLvalue, FunctionDesignator or Rvalue
  (`sema/expressions.rs:48-53`).
- `binding`, the resolved declaration for identifiers (`Some(index into bindings)`).
- `bit_field`, the width when the expression designates a bit-field.
- `operation_type`, the arithmetic type of a compound assignment.
- `register`, `static_address`, `vector_element`.
- `integer`, `floating` and `unfolded_binary128` for folded values.
- `ice` and `constant` (None, Arithmetic or Address).
- `selected_expression`, the operand that `_Generic` or
  `__builtin_choose_expr` selected.

Parentheses copy the inner record under the outer node's identity
(`sema/expressions.rs:870-875`). A recovered expression is forced to
`Unknown` with no value (`sema/expressions.rs:1014-1021`).

Conversions are a separate flat list. One expression can own several records,
in the order they were applied. The value-use helper `converted`
(`sema/expressions.rs:294-318`) records exactly one of:

- `ArrayDecay` to `T *`
- `FunctionDecay` to a function pointer
- `Lvalue`, to the unqualified non-atomic type

It records nothing for an rvalue. Operator code then appends `Arithmetic`
(promotions or usual arithmetic conversions), `Assignment` (assignment,
argument, return, initializer and cast targets) or `DefaultArgument`
(`sema/expressions.rs:285-291`; recording sites listed in section 2). Every
conversion attaches to the operand node the parent sees, so for `(x) + 1` it
attaches to the parenthesized node. Lowering should therefore look up
conversions on the exact child pointer it holds.

This gives lowering a usable contract: an `Lvalue` record marks a load. The
sites that read an lvalue's value record one: operands, conditions, `->`
bases, `*` operands, the target of `++`/`--` and of compound assignment,
call arguments, return values and initializer leaves. The sites that use only
an address record none: the left side of `=`, `.` bases, `&` operands and
`sizeof` operands.

### Lookup cost and the API a lowering pass would call

Nothing retained is keyed for O(1) lookup by syntax node. Analysis itself uses
`expression_indices: ArenaMap<usize, usize>` keyed by node address
(`sema/expressions.rs:144-151`, `sema/expressions.rs:1090-1097`), but that map
is scratch. The roadmap says a backend should build its own index
(`semantic-analysis.md`, "Retained expressions and conversions"), and the
inspection printer does exactly that (`sema/inspection.rs:131-133`).

A lowering pass therefore makes one O(n) pass, then does expected-O(1)
lookups with no re-analysis:

```rust
// Once per translation unit, using arena maps to respect .clippy.toml.
for (i, info) in sema.expressions.iter().enumerate() {
    expr_index.insert(ptr::from_ref(info.expression).addr(), i);
}
for (i, c) in sema.conversions.iter().enumerate() {
    conv_index.entry(ptr::from_ref(c.expression).addr()).or_default().push(i); // push order
}
// Per node e:
let info = &sema.expressions[expr_index[&ptr::from_ref(e).addr()]];
let ty = info.ty;                        // type before conversion
let cat = info.category;                 // lvalue / rvalue / function designator
let decl = info.binding.map(|b| &sema.bindings[b]);
let chain = &conv_index[&addr(e)];       // e.g. [Lvalue -> int, Arithmetic -> long]
let kind = sema.types.nodes[ty.index];   // TypeKind
let layout = sema.types.layout(ty);      // size and alignment, or None
```

Today this compiles only from inside the `semantic_analysis` module, because
`TypeKind`, `ExpressionInfo`, `ConversionKind`, `ValueCategory`, `Integer` and
`Definition` sit in private modules. Making them public is gap G1.

The binding lookup is already O(1) once the record is found. `info.binding`
is a direct index, and `Binding` gives kind, linkage, duration, scope and the
type at the point of use.

## 2. Construct-by-construct availability

Each entry gives a verdict: **Ready** (everything needed is retained),
**Derivable** (lowering can compute it from retained data, with the stated
work), or **Gap**.

### Arithmetic and comparisons

- **Integer and floating arithmetic: Ready.** The usual arithmetic
  conversions record an `Arithmetic` conversion on both operands to the
  common type (`sema/expressions.rs:372-458`, recorded at 455-456). They keep
  integer rank even where long and long long have equal width. The result
  type is `info.ty`. Unary `+ - ~` record the promoted type
  (`sema/expressions.rs:1290-1294`). Each shift operand gets its own
  promotion record (`sema/expressions.rs:1770-1773`). Signedness and width
  come from `TargetLayout::integer` (`src/target.rs:293-312`). An enum uses
  its tag's `compatible` scalar (`sema/types.rs:177-178`).
- **Comparisons: Ready, with one gap.** Arithmetic operands get `Arithmetic`
  records. Pointer comparisons record nothing beyond decay and load, and the
  result is `int` (`sema/expressions.rs:1784-1823`). A null pointer constant
  compared with a pointer, as in `p == 0`, gets no conversion to the pointer
  type, so lowering must treat an integer operand of a pointer comparison as
  null.
- **`&&` and `||`: Ready.** Both operands get a decay or load record. The
  result is `int` and no conversion to `_Bool` is recorded
  (`sema/expressions.rs:1778-1783`), so lowering compares each operand with
  zero. Short-circuit evaluation is structural.
- **`!`: Gap (a trap).** `type_unary` records an `Arithmetic` conversion of
  the operand to `int` for `LogicalNot` (`sema/expressions.rs:1288-1294`). For
  `!0.5` or `!p` that record is wrong: it says "truncate to int", but the
  operator compares with zero. A lowering pass that applies conversion chains
  literally would compute `!0.5 == 1`.
- **`?:`: Ready, with caveats.** The condition gets a decay or load record.
  Both arms get a record to the result type whatever the arm's category. The
  kind is `Arithmetic` even when the result is a pointer, void or struct
  (`sema/expressions.rs:1905-1965`). With the GNU `a ?: b` form, the "then"
  arm is the same node as the condition (`syntax.rs:270-272`), so that node
  receives two load records and an arithmetic record. Lowering must
  deduplicate them and evaluate the condition once.
- **Comma: Ready.** The result type is the converted right operand. The left
  operand also gets a load record (`sema/expressions.rs:1675-1680`), which
  lowering may ignore for anything not `volatile`.

### Pointers, subscripts, `++`/`--`, casts

- **Pointer arithmetic: Derivable.** For `p + i` and `p - i`, `info.ty` is
  the pointer type (`sema/expressions.rs:1715-1760`). Scaling is
  `types.layout(pointee).size`. The integer operand has no conversion record,
  so lowering sign- or zero-extends it from its own converted type. Pointer
  difference has type `ptrdiff_t` and divides by the element size. GNU
  `void *` and function-pointer arithmetic are not accepted
  (`pointer_arithmetic_target`, `sema/expressions.rs:272-274`).
- **Subscripts: Derivable.** `info.ty` is the element type and the category
  is lvalue. The array operand carries `ArrayDecay` and a pointer operand
  carries `Lvalue`. The index has no conversion record
  (`sema/expressions.rs:1684-1708`), so it is extended exactly as for pointer
  arithmetic. Multidimensional arrays work naturally because `a[i]` has an
  array type and decays when used as the base of the next subscript.
- **`++` and `--`: Derivable.** The operand gets a load record and the result
  type is the converted operand type (`sema/expressions.rs:1263-1278`).
  Neither the operation type nor the special cases are recorded: integer
  promotion for `char`, scaling for pointers, truth semantics for `_Bool`,
  width truncation for bit-fields. Lowering derives them from `info.ty` and
  `info.bit_field`.
- **Casts: Ready.** The operand gets its decay or load record, then an
  `Assignment` record to the target type (`sema/expressions.rs:1370`). The
  result type is the unqualified target. The record kind does not
  distinguish an explicit cast from an implicit conversion. A cast to
  `_Bool` must lower as `!= 0`. GNU union casts yield `Unknown`
  (`sema/expressions.rs:1329-1332`).

### Assignment

- **Simple assignment: Ready.** The right operand gets a decay or load record
  plus an `Assignment` record to the unqualified non-atomic left type
  (`sema/expressions.rs:1652-1673`). The left operand has no record, which
  marks it as an address-only use.
- **Compound assignment: Ready, but encoded unusually.** The left operand
  gets a load record. Both operands get `Arithmetic` records to the operation
  type, and `info.operation_type` holds that type. The final conversion back
  to the left type is recorded as an `Assignment` keyed on the compound
  expression node itself (`sema/expressions.rs:1893-1900`). For `p += i`,
  the operation type is the pointer type. The conversion keyed on `e` means
  "convert the operation result before the store", not "convert e's value".
  Lowering must special-case it, and must evaluate the left operand once
  (§6.5.16.2p3). An atomic left operand is visible as `TypeKind::Atomic` in
  the left operand's type.

### `sizeof` and `alignof`

- **Complete types: Ready.** `info.integer` holds the value, the type is
  `size_t`, and the result is an ICE (`sema/expressions.rs:1149-1184`).
- **VLAs: Gap.** `types.layout` returns `None` for `ArrayBound::Variable`
  (`sema/types.rs:228-268`). The result then has no value, and nothing links
  the type to its extent expression. `ArrayBound::Variable`
  (`sema/types.rs:53-59`) is hash-consed, so `int a[n]` and `int b[m]` share
  one `TypeId`. Lowering must find the declarator's bound expression in
  syntax and save the value at the declaration point. A typedef of a VLA and
  a pointer to a VLA make this awkward.

### Member access and bit-fields

- **Member offsets: Derivable at O(fields) per access.** `ExpressionInfo`
  keeps `ty`, `category` and `bit_field` (the width), but not which field was
  selected. `type_member` resolves it through the scratch `member_indices`
  map (`sema/expressions.rs:1480-1486`). Afterwards lowering must find the
  record tag from the base type (or the pointee for `->`) and scan
  `Tag::fields` for the member name. Each `Field` carries the byte `offset`,
  `bit_offset`, `width`, type, and the qualifiers along an
  anonymous-member path (`sema/types.rs:150-165`). Anonymous members are
  already flattened into `fields`.
- **Bit-fields: Derivable.** `offset` is the byte containing the first bit
  and `bit_offset` is the position within that byte, 0 to 7
  (`sema/declarations.rs:1111-1162`). The ABI storage unit (its start and
  size) is not retained, so lowering must pick a covering load width that
  does not read past the record. Signedness comes from the member type, and
  plain `int` is signed. Microsoft layout is applied by
  `TargetLayout.ms_bitfields`, but the access pattern still has to be
  chosen.
- **Record layout: Ready.** `Tag::layout` gives size and alignment,
  `Tag::members` gives declaration-order members, and `complete`, `tainted`
  and `contains_flexible` are recorded (`sema/types.rs:167-182`).

### Calls

- **Typed calls: Ready.** The callee gets `FunctionDecay`, or `Lvalue` for a
  function-pointer variable. The callee's pointee is
  `TypeKind::Function { result, parameters, prototype, variadic }`.
  - Prototyped arguments get a load record plus an `Assignment` record to
    the parameter type.
  - Extra variadic arguments, and all arguments to an unprototyped function,
    get `DefaultArgument` with the promoted type: float becomes double and
    bit-field and enum rules apply (`sema/expressions.rs:1521-1613`).
  - The result type is `result.unqualified()`.
  - In C89 or GNU modes, a call to an undeclared name binds an unprototyped
    int function (`sema/expressions.rs:548-616`).
- **ABI classification: not front-end data.** Neither SysV eightbyte
  classes nor Win64 by-reference rules exist anywhere. `TargetLayout`
  supplies only sizes and alignments (`src/target.rs:182-208`). That is
  backend work.
- **Varargs: partial.** `va_start`, `va_arg`, `va_end` and `va_copy` are
  typed builtins, and the va_list type and layout come from the target
  (`sema/builtins.rs:32-43`, `sema/builtins.rs:66-101`). Their lowering is
  entirely the backend's job.

### Literals

- **Narrow string literals: Ready.** The type is `char[N]`, the category is
  lvalue, and `static_address` is set (`sema/expressions.rs:863-869`).
  Bytes come from `Context::literal_units`: a character expands to its UTF-8
  units and a numeric escape is one byte.
- **Wide string literals: Ready.** They use `wide_literal_units`, which
  gives UTF-32 units on Linux and UTF-16 with surrogates on Windows.
- **Encoded literals (`u8""`, `u""`, `U""`): Gap.** `string_type` returns
  `None` for them, so their type is `Unknown`
  (`sema/expressions.rs:1101-1126`).
- **Compound literals: Ready.** `info.ty` is the completed type and the
  category is an object category. `static_address` is true only at file
  scope (`sema/expressions.rs:979-1005`), which is also how to tell static
  from automatic storage. The initializer has the same gaps as declarations
  (next section).

### Initializers

- **Scalar initializers: Ready.** A scalar leaf gets a load record and an
  `Assignment` record to the target subobject's type
  (`sema/initializers.rs:229`).
- **Folded values for static objects: Ready.** `info.integer` and
  `info.floating` carry the value before the leaf's `Assignment` conversion,
  so lowering applies that cast itself (`Integer::cast`,
  `sema/integer.rs:110`). A static const integer object also carries its
  converted value in `Binding.value` (`sema/initializers.rs:141-163`).
- **Aggregate placement: Gap.** Initializer traversal implements brace
  elision, designators, unions, unnamed bit-fields and flexible members, but
  it retains no placement plan (`semantic-analysis.md`, "Boundaries carried
  forward"). Lowering must re-run the §6.7.8p17-22 current-object walk to
  learn each leaf's offset. It must also detect string-literal array
  initializers itself, since `initialize_string`
  (`sema/initializers.rs:543`) records no conversion. Zero-filling and the
  difference between static and automatic materialization are also left to
  lowering.
- **Completed array types: Ready.** An array completed by its initializer is
  written back to `bindings[i].ty` (`sema/initializers.rs:134`).
- **Address constants: Gap.** For `static int *p = &a[2];`, the record says
  only `constant == Address` and `static_address`. The base and offset come
  from `address_parts` (`sema/expressions/address.rs:47-189`), an analyzer
  method that depends on the scratch member map and does not handle
  compound-literal bases. Lowering must reimplement it to emit a symbol plus
  offset.
- **GNU range designators (`[a ... b]`): Gap.** They are unmodeled, so the
  completed type is unknown.

### Statements

- **Conditions in `if`, `while`, `do` and `for`: Ready.** They get a decay
  or load record (`check_condition`, `sema/expressions.rs:677-707`). Lowering
  compares the result with zero.
- **`switch`: Derivable.** The control expression gets a load record plus an
  `Arithmetic` record to the promoted type (`sema/statements.rs:159-175`).
  Each case expression's `info.integer` holds its unconverted ICE value.
  Duplicate and overlap checking has already been done, but the converted
  and sorted case list is scratch. Lowering converts each value with
  `Integer::cast` to the promoted type and builds its own table. GNU case
  ranges are `StatementType::CaseRange` (`syntax.rs:145`).
- **`break`, `continue`, named break and continue, `goto` and labels:
  Derivable from syntax.** Placement errors are already diagnosed. Ordinary
  labels have function scope, so a per-function name map is enough. GNU
  `__label__` local labels have lexical scope, and their resolution is
  scratch-only (`sema/statements.rs:70-90`).
- **`return`: Ready.** The value gets a load record and an `Assignment`
  record to the unqualified result type (`sema/statements.rs:251-306`). The
  implicit `return 0` from hosted `main` (§5.1.2.2.3) is lowering's job.
- **Computed `goto`, label addresses and statement expressions: Gap.**
  Statement expressions are typed internally, but the expression itself has
  type `Unknown` because it falls into the "unmodeled" arm
  (`sema/expressions.rs:1006-1007`). Label addresses also fall into that
  arm. Computed-goto targets are opaque. Inline `asm` statements are syntax
  only.

### Declarations, storage and linkage

- **Bindings: Ready.** Each binding carries linkage and duration
  (`sema.rs:135-171`, decided in `sema/declarations.rs:741-768`).
- **Linked entities: Derivable.** Several occurrences of one linked entity
  are separate bindings. A use refers to the visible occurrence, whose type
  may be the composite seen at that point. Linked entities share a symbol
  name, but the entity table is scratch (`sema/functions.rs:88-98`).
- **Local `static` objects: Ready.** They are `Linkage::None` with
  `Duration::Static`, so lowering must invent a unique symbol.
- **`register`: Gap.** It is visible only as `ExpressionInfo.register`;
  `Binding` does not record it.
- **Definitions: Ready.** `definitions` lists object and function
  definitions, inline-only bodies (`Inline`, `sema/functions.rs:1129`),
  completed tentative definitions (`Tentative`, using the final composite
  binding; an incomplete external array becomes `[1]`,
  `sema/functions.rs:1012-1060`), and the implicit `__func__` arrays with
  their string (`FunctionName`, `sema/functions.rs:708-735`). An extern
  declaration without an initializer gets no record, as intended.
- **Definition-to-syntax links: Gap.** `Definition` holds only
  `{binding, kind}`. It does not point at the `FunctionDefinition` or
  `InitDeclarator` it came from, so lowering cannot find a body or an
  initializer without searching.
- **Function definitions: Gap.** `FunctionContext { syntax, binding, result,
  parameters }` (`sema/functions.rs:63-69`) is exactly what lowering needs,
  but it is scratch. Parameter bindings are created in the body scope
  (`sema/functions.rs:384-412`) with no retained list. For old-style
  definitions, the promoted incoming types appear only in the composite
  function type (`sema/functions.rs:593-690`). The body must convert from
  those promoted types to the declared parameter types, for example from
  double to float.
- **Symbol attributes: Gap.** GNU asm labels (`DirectDeclarator::AsmLabel`,
  `declaration_syntax.rs:936`) remain in the syntax but are not attached to
  the binding. Glibc's `__REDIRECT` symbol renaming depends on them.
  Visibility, weak, section, noreturn and similar attributes are not
  modeled.

### VLAs

- **Bound expressions: Derivable.** Bound expressions are typed and visible
  in declarator syntax (`sema.rs:1026-1075`).
- **Extents: Gap.** The type cannot carry its extent, as described under
  `sizeof`.
- **Jumps into VLA scope: Ready.** They are diagnosed, so lowering can rely
  on them never happening.
- **Lifetimes: Gap.** Deallocating at block exit needs a map from compound
  statement to scope. `scopes` are retained, but no syntax node points at
  its scope.

### `_Complex`, `long double`, binary128

- **`_Complex`: Ready.** The complex scalars, their layout, and `__real__`
  and `__imag__` lvalues are modeled. Floating constants are a `LongDouble`
  real and imaginary pair (`sema/constants.rs:23-26`). Complex arithmetic
  lowering (`__muldc3` and similar) is the backend's job.
- **`long double`: Ready.** It is the x87 80-bit format on the Linux and
  MinGW targets and binary64 on MSVC (`src/target.rs:83-89`). Folded values
  pass through the native x87 bridge, so a cross-compiling backend must
  re-encode them.
- **binary128 (`__float128`): Gap.** Literal bits are exact, but arithmetic,
  comparisons and casts are not folded (`unfolded_binary128`). A static
  initializer needing a computed binary128 value has nothing to emit.

### GNU and MSVC extensions

- **Atomics: Ready.** `TypeKind::Atomic` and its layout are modeled, and
  the C11, GNU and sync builtins are typed (`sema/atomics.rs:161`).
  Memory-order arguments carry `info.integer` when they are constant. No
  lowering exists.
- **Vector types: Ready.** Vectors (`TypeKind::Vector`) and their
  operations are typed, comparison masks included. The roughly 384 x86
  builtins have signatures (`sema/x86_builtins.rs`,
  `sema/x86_builtin_table.rs`). Lowering must map each name to an
  instruction. Vector lane constants are not folded.
- **Other `__builtin_*` calls: Gap.** An undeclared builtin, such as
  `__builtin_expect`, `__builtin_memcpy`, `__builtin_unreachable` or
  `__builtin_return_address`, is bound as an opaque function whose result is
  `Unknown` (`semantic-analysis.md`, Stage 2). Only `__builtin_constant_p`,
  `classify_type`, `choose_expr`, `types_compatible_p`, `offsetof`,
  `bit_cast`, `convertvector`, `shufflevector`, the va_* builtins and the
  modeled atomic, x86 and vector builtins are typed.
- **Typed but unmodeled forms: Gap.** `_Alignas`, `aligned`, `packed`,
  `mode`, thread-local storage and pointer-size modifiers make the affected
  type `Unknown` without a diagnostic. So do `countof`, `nullptr` and
  unmodeled builtins (`sema.rs:690-700`, `sema/traversal.rs:195-199`).
- **`_Generic` and `__builtin_choose_expr`: Ready.** `selected_expression`
  names the operand to lower.

## 3. Gaps, ranked for a first prototype

The prototype target is simple C functions using ints, pointers, control
flow, calls and structs. Tier A blocks it, tier B is needed shortly after,
and tier C is deferrable.

### Tier A: blocks the prototype

**G1. Semantic types are not nameable outside `semantic_analysis`.**
`mod types`, `mod expressions`, `mod functions`, `mod integer` and
`mod constants` are private, and `sema.rs` imports them with private `use`
statements (`sema.rs:8-33`, `sema.rs:41-61`). A lowering module elsewhere can
read `sema.bindings[i].ty.index` but cannot match on `TypeKind` or
`ConversionKind`. *Fix:* add `pub(crate) use` re-exports of `TypeKind`,
`TypeId`, `Tag`, `Field`, `Member`, `ArrayBound`, `ExpressionInfo`,
`ValueCategory`, `Conversion`, `ConversionKind`, `ConstantClass`,
`Definition`, `DefinitionKind`, `Integer` and `Floating`. Alternatively,
give `SemanticTranslationUnit` a small query facade.

**G2. No reliable "may lower" signal.** Analysis counts its errors in
`Analyzer::semantic_errors` (`sema.rs:406`, `sema.rs:545-567`), but the
counter is not retained. `Context::pending_error_count`
(`context.rs:1022-1027`) counts warnings too, and the only error count is
computed while the CLI reporter drains the queue
(`cli/diagnostic_reporter.rs:220-231`). Unmodeled constructs are a second
problem: they silently produce `Unknown` types under taint, with no
diagnostic. *Fix:*

- Add `Context::error_count()`, maintained when an Error-severity item is
  queued. Errors are never withheld (`context.rs:1250-1265`).
- Retain the semantic error count in `SemanticTranslationUnit`.
- Have lowering emit an "unsupported construct" error whenever it reaches
  an unanalyzed type (see section 4).

**G3. No retained syntax-to-semantics index.** Records are keyed by node
address, but the address-to-index map is scratch (`sema.rs:413`), and
conversions for one node are scattered through the list. A Lvalue record is
added when the parent is typed. A return or initializer `Assignment` record
is added later. *Fix:* at the end of `analyze`, stable-sort conversions by
owning record and store a `(first, count)` range in each `ExpressionInfo`.
Then either retain the address map in `'tu`, or have the parser assign a
dense `u32` expression id. `Expression` is pinned at 48 bytes
(`parsing/tests/node_sizes.rs:94`) with about 3 bytes of padding, so an id
would grow it. The workaround costs about 20 lines: build arena maps once,
as `sema/inspection.rs:131-133` does.

**G4. No map from function definition or declarator to binding.**
`Definition` lacks syntax. `FunctionContext` and parameter bindings are
scratch. A declarator's binding can be found only by matching
`Binding.name.source_vectors` with `Declarator::identifier()`
(`declaration_syntax.rs:1050`). That matching is fragile: `__func__` is
bound with the function name's provenance (`sema/functions.rs:719-731`), and
K&R parameters bind from the declaration list. *Fix:* retain
`functions: &[FunctionInfo { syntax: &FunctionDefinition, binding, parameters: &[usize], body_scope }]`
and a binding index per `InitDeclarator`, either in a side table keyed by
declarator identity or by adding the syntax reference to `Definition`.

**G5. The `!` conversion record is wrong.** `type_unary` records
`Arithmetic -> int` for `LogicalNot` (`sema/expressions.rs:1288-1294`), so
literal application miscompiles `!0.5`. *Fix:* record nothing for `!`, as
`&&` and `||` already do, or add a `Truth` conversion kind. Add a test that
pins the record.

**G6. The selected member is not retained.** Every `.` and `->` needs a scan
of `Tag::fields` by name. *Fix:* add `field: Option<u32>`, the index into
`Tag::fields`, to `ExpressionInfo`. `type_member` already has it
(`sema/expressions.rs:1483-1486`).

### Tier B: needed soon after first light

**G7. Retained `Types` is thin.** It has only `layout()`
(`sema/types.rs:219-224`). There is no `unanalyzed`, `complete_object`,
`variably_modified` or `alignment`, because `array_tails` and
`variably_modified` are scratch (`sema/types.rs:303-304`). There is no
interning either. `integer_type`, which handles enum and bit-field cases, is
an analyzer method (`sema/integer.rs:614`). *Fix:* retain those two `Vec`s
(one entry per type node) and move these queries to `Types`.

**G8. Compound-assignment encoding.** The final conversion is keyed on the
compound expression (`sema/expressions.rs:1896`). *Fix:* add a distinct
`ConversionKind::StoreBack`, or document the rule beside `operation_type`.
Deduplicate the double records on `a ?: b` (`sema/expressions.rs:1910-1915`).

**G9. No initializer plan.** *Fix:* during `check_initializer`, retain one
leaf per initialized scalar, each carrying the initializer node, the byte
offset from the object start, the bit-field data, the target type, and
whether it is a string-literal array copy. Lowering then zero-fills or
memsets, stores the leaves, and for static objects folds them into bytes.
Without this, lowering duplicates `sema/initializers.rs` (673 lines).

**G10. Address constants for static data.** *Fix:* retain
`(AddressBase, offset)` on records with `constant == Address`. It is computed
already for difference folding. Add compound literals as a base kind.

**G11. Builtins commonly found in real code.** `__builtin_expect`,
`__builtin_unreachable`, `__builtin_trap`, the `memcpy` and `memset`
families, `__builtin_bswap*`, `__builtin_clz`, `__builtin_ctz`,
`__builtin_popcount` and `__builtin_alloca` all yield `Unknown`. *Fix:*
give them typed signatures, like `x86_builtin_table.rs`.

**G12. Entity and symbol data.** *Fix:* retain an entity index per linked
binding, plus the asm label and the attributes that affect emission. The
entity table is built during analysis and then discarded.

**G13. Bit-field storage units.** *Fix:* retain the storage-unit offset and
size per bit-field `Member`. Layout computes them anyway
(`sema/declarations.rs:1100-1162`). Lowering can then use the ABI container
access.

### Tier C: deferrable

- **G14. VLAs.** Make `ArrayBound::Variable` carry a per-declarator id with a
  retained table of `(bound expression, element, scope)`. Compatibility must
  ignore the id. Also add a compound-statement-to-scope map for
  deallocation, and evaluate variably modified parameter types on function
  entry.
- **G15. Unmodeled expressions and statements.** These are statement
  expressions, label addresses, computed `goto`, union casts, range
  designators, encoded string literals, `nullptr` and `countof`.
- **G16. Floating constants.** binary128 arithmetic needs folding.
  Re-encoding extended floating constants should not depend on the host x87
  bridge.
- **G17. Alignment and layout attributes.** `_Alignas`, `aligned`, `packed`
  and `#pragma pack` currently make types unavailable.
- **G18. GNU local labels and nested functions.** Both need retained scope
  resolution and nonlocal-jump mechanics.
- **G19. `volatile` comma operands and expression statements.** Lowering
  should load a discarded `volatile` lvalue, as GCC does. Sema records no
  load for `x;`.

## 4. Recovered and erroneous syntax

The front end deliberately keeps going after errors, so lowering needs a gate
of its own.

**Diagnostics.** Every phase appends to one FIFO in `Context`.
Preprocessing, parsing, extension-policy and semantic diagnostics all go
there (`sema.rs:545-567`). Errors are never withheld, even in system headers
(`context.rs:1250-1265`). The simplest gate today is driver-level: drain
the queue through `DiagnosticReporter::report_pending` (`cli.rs:857-866`)
and refuse to lower if the reporter's `errors` count is nonzero
(`cli/diagnostic_reporter.rs:226-229`). An extension diagnostic under
`-pedantic-errors` has Error severity and is counted. A library-level gate
needs G2's `Context::error_count()`.

**Syntax recovery markers.** These are useful for localized refusal, but not
sufficient as a gate, because preprocessing errors and some semantic errors
leave no syntax marker:

- `ExternalDeclaration::RecoveredDeclaration`,
  `RecoveredFunctionDefinition` and `Error` (`syntax.rs:44-58`).
- `recovered: bool` on `FunctionDefinition`, `Statement`, `Expression`,
  `Declaration` and `Initializer` (`syntax.rs:88`, `syntax.rs:126`,
  `syntax.rs:256`; `declaration_syntax.rs:66`).
- `ExpressionType::Error` (`syntax.rs:346`) and `ExpressionSlot::Missing`
  (`syntax.rs:104-108`).

Semantic analysis handles a recovered subtree with `taint`
(`sema.rs:682-685`, `sema/traversal.rs:40`, `sema.rs:780-781`). A tainted
subtree suppresses new diagnostics and records no definitions, uses or
labels (`sema/functions.rs:798`, `sema/functions.rs:914`). A recovered
expression gets `Unknown` (`sema/expressions.rs:1014-1021`). A recovered
initializer sets the binding's type to `Unknown`
(`sema/initializers.rs:103-105`). So a recovered function definition has no
`Definition` entry.

**Silent unknowns.** These remain even when the error count is zero. Valid
code using an unmodeled extension produces `TypeKind::Unknown` types, or
tags with `tainted` set, without any diagnostic. The causes are listed under
G11, G15 and G17 (`sema.rs:690-700`, `sema/traversal.rs:195-199`,
`sema/expressions.rs:1006-1007`). Lowering must check every type it consumes
by walking arrays and atomic wrappers to the terminal node. It must reject
`Unknown`, a tainted tag, or a `layout()` of `None` on an object that needs
storage, and report "construct not supported by code generation" at the
node's provenance. Once G7 is fixed, that check is just
`types.unanalyzed(ty)`.

**Recommended policy for the prototype:**

1. Lower nothing if the error count is nonzero.
2. Otherwise lower one function definition or static object at a time, and
   abort that item, with a diagnostic, on the first unanalyzed type, the
   first `Unknown` conversion target, or a call to an opaque builtin.
3. Treat `ExternalDeclaration::Asm` and asm statements as unsupported until
   modeled.
