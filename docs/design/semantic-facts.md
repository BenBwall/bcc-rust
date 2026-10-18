# One source of truth for semantic facts

Extend the existing retained `SemanticTranslationUnit<'tu>` interface into the single producer of language facts needed by lowering: control and lifetime regions, resolved calls and builtins, expression evaluation effects, parameter correspondence, runtime-bound evaluation sites, initializer placement, and symbolic constants. Keep lowering's explicit execution work stack, SSA construction, storage choices and IR instruction selection. Put builtin contracts in one declarative registry used by semantic analysis and preprocessing, with retained checked instances consumed by lowering. This is an incremental completion of the existing stage-0 design, not a parser rewrite or a new typed syntax tree. Start with control/lifetime facts: the separate VLA statement walk is the highest-risk disagreement.

## Evidence and scope

This is a proposal, with no implementation changes. The inspected revision is
`061c74f6560fff82b4481d28547a113795f5381d` (PR #42's supplied head), on
`claude/semantic-facts-design`. References below describe that revision, not
fixes being made in other worktrees. The working tree was clean before writing.
No compiler or test executable was built or run for this document. Neither
`target/llvm` nor `target/llvm-libs` was modified.

The audit covered all 20 production files under `src/lowering/` (9,743 lines),
the lowering entry file, semantic producers, `src/abi/`, `src/optimizer/`,
pipeline and CLI handoff, and builtin consumers in preprocessing, parsing and
the back ends. The inventory has **36 consumer entries and 32 builtin-knowledge
locations**. These are grouped responsibilities, not 68 independent bugs;
the tables intentionally overlap where a builtin consumer also reconstructs a
language fact. Tests contain additional fixtures and expected names, rather
than production decision makers.

History was checked in `git log`, [semantic-analysis.md](../../semantic-analysis.md),
[middle-end.md](../../middle-end.md), and
[the GCC execute report](../research/gcc-torture-execute-results.md).
The report's surveys are historical measurements, not a run of this design
branch. A history association identifies an exposed contract or related repair;
it does not establish that every repair had this structural cause. “None found”
means no specific corresponding finding was located. Risk labels are design
judgments: **high** means control, effects, binding or object contents can be
wrong; **medium** means missing constructs, repeated work or representation drift;
**low** means an appropriate downstream projection with a small shared contract.

Useful history anchors:

| Key | Evidence and what it actually establishes |
| --- | --- |
| H1 | Task-supplied PR #42 round-1 P1: goto to a label inside a statement expression restores an enclosing VLA. Source inspection confirms `index_vla_scopes` never visits expression children; this document does not independently execute the reproducer or inspect another worktree's repair. |
| H2 | PR #41 round 1: `dcde939d`, `6485511c`; builtin shadowing, hosted/freestanding library types, GNU inline emission and statement-expression entry constraints. Lowering effects were cached to avoid repeated walks. |
| H3 | PR #41 round 2: `6f7e2ee6`; statement-expression ancestry became constant-time. `5bdfccd6` adds typed overflow and mixed-switch coverage; it is not a demonstrated switch miscompile. |
| H4 | PR #41 round 3: `3fd94049`; converted block values and loop-body jump contexts. The docs record Clang header-jump limitations and independent GCC controls. |
| H5 | PR #41 round 4/5 evidence: `05e354cd`, `a984fd48`; probability must be checked after double rounding, object-size mode after int conversion, alignment after size_t conversion. The round-5 audit explicitly distinguishes prefetch's unconverted variadic ICEs. |
| H6 | PR #41 round 6 and shared-helper follow-up (task describes round 7): `9225bf83`, `f84d8428`, `e9333e4e`, `a5670e79`; selected builtin designators/callees, labeled value tails, selected varargs and labeled overflow effects. `33591815` tests the existing dynamic-alloca inlining exclusion. |
| H7 | Earlier middle-end repairs: `c8973422`, `4c9b5c6e`, `38359700`, `86d1597e`, `00153aef`; replacement initializers, union selections, remaining string-array bytes, braced strings and preservation of explicit bytes. `0a547f82`, `0f7f1023` index reset coverage. |
| H8 | Earlier middle-end repairs: `f940b3b7`, `c5c2ece8`, `a07d7654`; assembler names/conventions, user symbol preservation and recovery, completed global layouts/volatile accesses. |
| H9 | `bb65a81a`, `854889dc`, `671de1c3`, `ee567c09`: open C calls, compatible host calls, runtime signature checks and implicit library prototypes. GCC report “Correctness follow-up” and “Implicit library prototypes” explain the distinct contracts. |
| H10 | `e36d11d4`, `033f9808`, `a472e002`: retained VLA bounds/original parameter types and their execution. GCC report names `970217-1.c`, `pr77767.c` and seven VLA sources; correspondence is a current implementation issue, not a separately located past finding. |
| H11 | `4a6e3d26`, `e7a55353`, `c68cab5a`, `dada4617`: representation attributes, union/range initialization, static compound literals/pointer constants and floating builtins. GCC report “torture type representations and static initializers” records support gains and remaining refusals. |
| H12 | `40b79b00`, `a79a429f`, `4e0dff9d`, `caa17107`: RHS-first compound updates (`pr58943.c`), aggregate-argument snapshots, host floating carrier decoding, bounded overlapping-union ABI flattening. These include legitimate execution/representation repairs, not just duplicated semantic facts. |

## Inventory: consumers after semantic analysis

In this table, `S/` means
[`src/translation_phases/semantic_analysis/`](../../src/translation_phases/semantic_analysis.rs)
and `L/` means [`src/lowering/`](../../src/lowering.rs). Function names are the
search anchors; grouping several functions avoids treating one walk as many
independent problems.

| ID / fact | Semantic producer or existing knowledge | Consumer computation / duplication | History; risk; destination |
| --- | --- | --- | --- |
| C01 Lifetime topology and declaration start points | `S/scopes.rs::enter/leave`; `S/statements.rs::record_vm`; traversal visits statement expressions | `L/vla.rs::index_vla_scopes/index_declaration/enter_scope/enter_vla` constructs a second tree from statement addresses and binding+1, including only automatic VLA storage | H1, H10; **high**. Retain lexical scopes plus declaration regions, with VM constraints separate from storage requirements. |
| C02 Scopes left by exits/jumps | `S/statements.rs::find_entered_vm/entered_vm/finish_labels/switch_label` compares persistent declaration paths for diagnostics | `L/vla.rs::cleanup_to/is_scope_ancestor/remember_target`; `L/statements.rs::jump_away/return_value` compare another topology to release storage | H1; **high**. One retained edge transition drives diagnostics and cleanup; emit restores only for storage-owning regions. |
| C03 Label identity and goto destination | `S/statements.rs::label/label_index/local_labels/finish_labels` knows function ownership, local-label visibility, VM and expression ancestry | `L/statements.rs::label_block/statement` keys labels by spelling; goto reconstructs the target scope from `label_scopes` | H1, H2; **high**. Consume `LabelId` and `JumpFacts`. Local labels/nonlocal jumps remain explicit unsupported capabilities until implemented. |
| C04 Switch/case/default ownership | `S/statements.rs::SwitchReady/case_label/switch_label/finish_switch`; parser owns duplicate-default diagnostic | `L/statements.rs::switch` walks the entire body again, excluding nested switches, to discover cases/default | H3 coverage, H4 contexts; **high**. Retain switch-owned label identities. Semantic retention must include the default without duplicating its parser diagnostic. |
| C05 Converted case values/ranges | `S/statements.rs::case_done` converts endpoints to promoted control type; `finish_switch` sorts and checks overlaps | `L/statements.rs::switch` fetches original expression integers and casts endpoints again; ranges and default get fresh maps | H3, H11; **high**. Retain converted `Integer` endpoints, source ordinal and sorted dispatch index. |
| C06 Break/continue selection, especially headers | `S/traversal.rs::statement`; `S/statements.rs::LoopBody/LeaveLoop` and switch continuations decide validity/context | `L/statements.rs::statement` plus `Task::PushTargets/PopTargets` independently chooses nearest break or loop continue target | H4; **high**. Retain logical control target per jump. Lowering still assigns those targets IR blocks. |
| C07 Purity / whether a value read is observable | `S/builtins/effects.rs::dependencies/overflow_type_operand_is_pure`; semantic overflow ICE classification invokes it | `FunctionLowerer::effect_summaries` invokes the same `EffectSummaries::pure` engine again in function scratch | H2 scaling; **high**. Shared implementation exists, but results are recomputed and discarded. Retain both read and designation/effects summaries. |
| C08 Evaluation needed when a value is discarded | `S/builtins/effects.rs::dependencies` has syntax-specific effect rules; selected operands and ICEs are known | `L/builtin_effects.rs::builtin_effects` independently matches operators, volatile reads, calls, short circuit and block expressions to schedule effects | H2, H6; **high**. Retain effect recipes; scheduler becomes a recipe interpreter. This is not two wholly independent purity engines at this head. |
| C09 Direct callee and designation-preserving wrappers | `S/expressions/results.rs::selected_designator`; call typing and binding lookup know selected operands | `L/expressions.rs::direct_callee` rechecks function decay, conversions along the selected chain and function binding; called during scheduling and completion | H6; **high**. Retain call-site directness and point-of-use function type, preserving conservative indirect cases. |
| C10 Named builtin identity and argument evaluation mode | `S/builtins/common.rs::common_builtin_call`; family call checkers; `Binding::builtin` | `L/builtins.rs::schedule_builtin/finish_builtin`, `L/expressions.rs::direct_callee` classify names again, choose evaluated/unevaluated/type-effect operands and unsupported families | H2, H5, H6; **high**. Retain descriptor ID and checked operation instance. Current object-size folding already consumes sema's value. |
| C11 Keyword/ordinary varargs operand shape and storage | `S/builtins/intrinsics.rs::type_builtin/builtin_va_list`; `common.rs::keyword_call` adapts ordinary calls | `L/builtins.rs::schedule_keyword/finish_keyword/va_list_address` and named-call path reconstruct shape/count and manufacture another `Builtin` syntax adapter in `'tu` | H6, H10; **high**. Retain normalized operands (storage address, adjusted list pointer, type operand); ABI expansion remains downstream. |
| C12 Statement-expression value tail | `S/expressions/results.rs::statement_expression_tail`; `S/expressions.rs::expression_done` computes converted result | `L/gnu.rs::statement_expression` reruns the shared tail walk, then derives the prefix length | H4, H6; **high**. Retain outer labeled tail statement, leaf expression and prefix length; preserve labels during execution. |
| C13 Prototype-to-body parameter correspondence | `S/functions.rs::function_body/old_signature` creates body bindings with parameter order; prototype bindings remain in bound expressions | `L/statements.rs::FunctionLowerer::new` builds a source-vector map then scans **all TU bindings per function** to alias parameter storage (~873) | H10; **high** correctness and scaling; contradicts `middle-end.md` stage-0 item 4. Retain explicit binding aliases per definition. |
| C14 Bound discovery, evaluation sites/order and reuse | `S/types.rs::variable_bound`, runtime-bound mode in `S/traversal.rs`/analyzer; `Parameter::original`, `FunctionRecord::parameter_types` | `L/vla.rs::prepare_type_tasks` walks array/pointer types at declarations, parameters and casts, inferring which bounds are not yet registered; `runtime_size` rebuilds factor chains | H10; **high**. Retain site-specific evaluation actions and shared size/stride formulas. Dynamic values/SSA variables remain lowering state. |
| C15 Runtime sizeof's operand/type and evaluation | `S/expressions/typing.rs::type_sizeof`; `S/functions.rs::enter_sizeof`; retained type names and runtime bounds | `L/expressions.rs::evaluate_raw` searches every type-name record by source vector for `SizeofType`; chooses operand evaluation/bound tasks for runtime sizeof | H10; **high** semantics, **medium** repeated lookup. Retain `SizeQueryFacts`, including exact required evaluation, and identity-indexed type names. |
| C16 Declarator-to-binding correspondence | `S/declarations.rs::bind_declaration`, `S/functions.rs::function_body` knows declaration identities | `L/module_items.rs::Unit::new/declarator_binding` builds `(name, source_vectors) -> first binding` index; declaration/extension paths depend on it | H8, H10; **high**. Retain occurrence IDs from the producer, including rejected/recovered occurrences without fabricating accepted bindings. |
| C17 Definition-to-initializer association | `S/initializers.rs::initialize_declaration`, finalized `Definition` records | `L/module_items.rs::declare_items` walks parsed external declarations to rebuild initializer map; local declaration lowering independently pairs syntax and binding | H7, H8; **medium**. Retain initializer/plan ID on declaration occurrence and finalized object definition. |
| C18 Current object, brace elision, subobject advancement | `S/initializers.rs::check_initializer/subobject/advance_current/whole_object_expression` | `L/initializers.rs::leaves/Walker::subobject/consume/open_next/braced_scalar` implements a second initializer current-object machine | H7; **high**. One semantic placement plan serves automatic and static objects. |
| C19 Designated field/path and range endpoints | `S/initializers.rs::check_initializer` resolves designators, validates endpoints and completes extent | `L/initializers.rs::Walker::designate` searches `Tag::fields` by name and reconstructs Cartesian range cursors | H7, H11; **high**. Retain resolved paths/offsets and compact repeat dimensions; no second field-name search. |
| C20 Whole-object copy vs scalar vs string initialization | `S/initializers.rs::whole_object_expression/initialize_string`, retained `string_initializers` | `L/initializers.rs::Walker::leaf/braced_string`, `braced_scalar`, `initializer_expression` reclassify copies/wrappers. String whole-array classification itself is already shared | H7; **high**. Retain placement operation kind and literal identity; retain the existing string-classification invariants. |
| C21 Union selection, replacement, implicit bytes | `S/initializers.rs::check_initializer` determines current object and ordering, but does **not** retain a complete replacement/zero plan | `L/initializers.rs::select_union/forget_subobjects/initialize_aggregate`; `initializers/coverage.rs::CoverageIndex` invents reset/coverage operations. Static bytes replay `leaves` too | H7; **high**. Complete the semantic initializer contract, retaining reset scopes and covered bytes/bits. The efficient coverage index may survive as an implementation detail of plan emission. |
| C22 Range expression sharing and once-only evaluation | `S/initializers.rs::check_initializer` checks GNU ranges; same expression may initialize many subobjects | `L/initializers.rs::initialize_aggregate` counts repeated expression addresses to choose remembered values and reuse tasks | H11; **high**. Explicit `InitValueId` and repeat operations preserve one evaluation per range occurrence, including aggregate snapshots. |
| C23 Address base and scaled addend | `S/expressions/address.rs::address_parts/address_value/integer_address` computes eligibility and address decomposition for constants | `L/module_items.rs::address_constant` walks selections, casts, members, subscripts and pointer arithmetic again to choose symbol/addend | H11; **high**. Retain `SymbolicAddress` before mapping C entities to IR symbols. |
| C24 Static scalar conversion / floating representation | `S/constants.rs::floating_value/round_floating`, `S/integer.rs::Integer::cast`, recorded initializer conversion | `L/module_items.rs::static_scalar` converts integer/floating values again; `L/floats.rs::constant_bits/from_integer/from_unsigned/to_integer` implements target encoding/rounding beside semantic conversion | H5, H12; **medium/high**. Retain converted semantic constants at initializer destinations; share target scalar conversion helpers. Byte serialization and runtime IR conversions stay downstream. |
| C25 Calling-convention/attribute interpretation | `S/attributes.rs`, semantic declarator/type traversal retains raw modifier chains | `L/extensions.rs::prepare_extensions/check_chain/check_keyword/check_attribute/default_convention` parses attribute tokens and interprets ABI spellings again | H8; **high**. Semantic analysis resolves a convention and attribute result once; lowering's capability check accepts/refuses that enum. |
| C26 Assembler label / entity symbol selection | `S/declarations.rs` and `S/functions.rs` know linkage and entity relationships but retained modifiers are raw | `L/extensions.rs::record_label/symbol_name` decodes labels and inherits linked names; `L/module_items.rs::declare_items/function/object` resolves collisions and aliases | H8; **high**. Retain decoded language symbol requests and entity IDs; target symbol representability/collision checks remain IR construction work. |
| C27 Known definition vs open external, effective call shape | `S/functions.rs` finalizes linked definitions; `S/expressions/typing.rs::type_call` knows prototype/default promotions at point of use | `L/module_items.rs::Unit::new/unprototyped_external/function` rebuilds known-definition name set; `L/expressions.rs::call` derives fixed prefix/ellipsis again | H9; **high**. Retain entity definition state and call facts. Never replace point-of-use call typing with a later composite prototype. |
| C28 Increment promotion / pointer operation semantics | `S/expressions/typing.rs::type_unary/type_binary/promote/common_arithmetic` checks promoted arithmetic and pointer operands | `L/expressions.rs::step_value` infers narrow-int overflow from width; `finish/binary/offset_pointer` chooses pointer operand/stride again from types | H12 is related scheduling evidence, not proof these helpers are wrong; **medium/high**. Retain operation class, promoted operation type, narrowing-back rule and stride. Instruction selection stays downstream. |
| C29 Const/volatile object eligibility | `S/functions.rs::inline_object_modifiable`; type graph qualifiers/array tails; `S/expressions/typing.rs::contains_const` serves a different aggregate constraint | `L/module_items.rs::constant_object` traverses array layers for readonly global emission; `L/places.rs::mem_flags` and initializer flags inspect qualifiers | H8; **medium**. Share retained type/object access predicates, keeping “modifiable aggregate” distinct from “whole object readonly”. Load/store flags are projections of those facts. |
| C30 Main/fallthrough policy | `S/functions.rs::check_main` validates hosted main; function result and configuration are available | `L/statements.rs::finish_body` synthesizes zero for scalar functions named `main`, with **no standard/hosted test**; interpreter/CLI launch `main` separately | None found for the name-only end policy; **medium**. Retain `FunctionEnd::{ReturnVoid, ReturnIndeterminate, MainZero}`; interpreter launch remains a runtime contract. |
| C31 Address requirements (positive example with remaining builtin coupling) | Typed records contain resolved bindings/categories and conversions | `L/address_taken.rs::find` does one linear TU-record scan, but still follows selected-designator chains and classifies varargs names/operands | H6; **medium**. Retain address-use facts/varargs storage operands. Lowering still chooses SSA vs slots; preserve this linear scan pattern instead of adding another tree walk. |
| C32 Selected member query through wrappers | `ExpressionInfo::field_index` is retained | `S/results.rs::SemanticTranslationUnit::selected_field`, called by lowering, walks `selected_designator`, inspects base syntax and recovers the owning record type | H6; **medium**. Producer retains `(TagId, field index)` or `FieldId`, not only the field's relative index. Existing member offsets are not recomputed. |
| C33 Aggregate projection for ABI | Semantic `Types::layout`, natural member types/offsets, bit fields and `contains_flexible` are retained | `L/types.rs::aggregate_layout/scalar_leaves` flattens retained **types**, deduplicates `(type, offset)` and derives unaligned-leaf flag; no syntax access by ABI lowering | H12; **medium** repeated work, **low** ownership risk. Keep ABI representation projection downstream and cache per complete type; share language layout predicates. This is an allowed type-graph walk, not another language analyzer. |
| C34 Returns-twice external names / frame effects | `L/expressions.rs::direct_callee` refuses setjmp-family/vfork names; semantic calls lack retained returns-twice effect | `src/optimizer/inline.rs::returns_twice/CallGraph::compute` repeats that spelling list and propagates unsafe-frame state; also examines dynamic stack IR operations | H6; **high**. One external-symbol contract catalog, projected to IR facts; preserve textual-IR protection and dynamic-alloca exclusions. |
| C35 Runtime library signature contract | `S/builtins/library.rs::library_function/library_prototype` supplies reserved aliases/implicit library declarations | `src/backend/interpreter/host.rs::check_signature` has an independent name/signature table before executing host operations | H9; **high**. Share neutral external-symbol signatures; runtime checks remain necessary for arbitrary textual IR. Host implementation and fault handling are separate producers of runtime behavior. |
| C36 Parameter metadata correspondence in semantic inspection | Semantic parameter records already retain `array_minimum` | `S/inspection.rs::SemanticTranslationUnit::inspect/write_binding` builds a source-vector map to attach constant static minima to displayed bindings | No separately verified finding; **low**, diagnostic presentation only. Use retained parameter/binding correspondence, preserving inspection output. This is a source-identity join, not another type checker. |

### Audited downstream work that stays downstream

[`src/abi.rs::lower_abi`](../../src/abi.rs), `abi/plan.rs::plan_signature` and
the System V/Win64 classifiers consume IR signature/layout descriptors. They
do not receive syntax or semantic analyzer state. Register allocation,
eightbyte classification, aggregate packing, va-list cursor movement and ABI
preflight are not semantic duplicates. Neither are optimizer CFG, alias,
escape, poison and memory analyses: their input is IR, including user-written
textual IR. `lowering/ssa.rs`, `emit.rs` and `bit_fields.rs` choose IR storage,
blocks and access pieces; the latter consumes retained byte/bit placement.

[`pipeline/middle_end.rs::lower_translation_unit`](../../src/pipeline/middle_end.rs)
passes syntax and sema to lowering and schedules optimization/ABI once. It
does not reanalyze either. `cli/code_generation.rs::generate_code` and
`cli/output.rs::print_parser_output` select views and use the existing error gate;
syntax-only output legitimately stops before semantic analysis.
`backend/interpreter/main.rs::Machine::enter_main` checks executable IR
signatures and supplies writable runtime arguments; it cannot trust C sema
for textual IR. LLVM intrinsic emission and interpreter instruction execution
implement IR operations, rather than recognize C builtins again.

These distinctions matter: compound-update scheduling, argument snapshots,
poison handling and ABI packing can still have bugs after semantic duplication
is removed. The proposal reduces one recurrent cause, not every compiler bug.

## Inventory: where builtin knowledge lives

The registry replaces independent contracts, not independent implementations
of semantic rules and executable operations. `S/` and `L/` retain the meanings
above. Configuration, resource headers and preprocessor macros are included
to make the distinction between syntax recognition, advertised semantics and
executable support explicit.

| ID / location and functions | Knowledge / current overlap | History; risk; disposition |
| --- | --- | --- |
| B01 `S/builtins.rs::modeled` (`implemented_builtin`) | Keyword intrinsic support list, separate from named recognition and lowering | H6; medium. Registry semantic-support query keyed by keyword identity. |
| B02 `S/builtins/common.rs::common_builtin/common_builtin_type/common_builtin_call/common_argument/common_integer_constant/constant_overflow/keyword_call` | Scalar names/suffixes, signatures, arity, conversions, immediates, folds, varargs adaptation | H2, H5, H6; high. Declarative descriptors plus scalar checking/folding handlers. |
| B03 `S/builtins/library.rs::library_function/library_builtin/library_prototype/implicit_library_type/library_builtin_name` | Library prototypes, reserved aliases/runtime names, hosted-only implicit ordinary names | H2, H9, H11; high. One signature/alias data set; existing warning policy remains semantic. |
| B04 `S/builtins/atomics.rs::intrinsic/modeled/atomic_call/atomic_order` | C11/GNU/legacy atomic name families, object/buffer rules, orders and lock-free queries | Earlier `57e21fc2`; high. Family descriptors and dedicated type/order checker; preserve warning-only order rules. |
| B05 `S/builtins/x86_builtins.rs::known/x86_builtin_type/x86_immediates/vector_overload` | Table decoding, pointer types, converted immediates, overload exceptions | Earlier `67232f36`; high. X86 handler consumes generated descriptors. |
| B06 `S/builtins/x86_builtin_table.rs::BUILTINS/Immediate` | 386 generated named entries with signatures/immediate limits | B05 history; medium. Generated registry shard; preserve generator provenance and four-target verification. |
| B07 `S/builtins/floating.rs::floating_builtin` | Classification/sign-copy conversions, fold eligibility and signed-zero/NaN behavior | H11; high. Dedicated fold handler selected by descriptor. |
| B08 `S/builtins/intrinsics.rs::type_builtin/type_offsetof/builtin_va_list/operand_type` | Keyword dispatch, varargs constraints/layout, compatible-type/choose/offsetof operands | H6, H10; high. Special-form descriptors, normalized retained operands; keep type/member algorithms. |
| B09 `S/builtins/designators.rs::check_builtin_designators` | Intrinsic-only vs addressable library alias; scans transparent uses | H6; high. Descriptor `DesignatorPolicy` and retained normalized designation. |
| B10 `S/builtins/names.rs::named_builtin` | Joins six recognition predicates for `__has_builtin` | H2; medium. Registry lookup; no union of independent recognizers. |
| B11 `S/builtins/effects.rs::dependencies/EffectSummaries::pure` | Overflow type-operand purity; conservative calls/block forms; recomputed by both phases | H2, H6; high. Expression facts, with descriptor operand/effect policy. |
| B12 `S/type_generic.rs::named_builtin/type_generic_builtin/math128_builtin/classify_type_callee/type_generic_constant` | Classification, constant selection, binary128 math spellings/types | Earlier `57e21fc2`; high. Dedicated type rule/fold handlers behind descriptors. Do not invent `__builtin_tgmath` support. |
| B13 `S/vectors.rs::named_builtin/convert_vector_builtin/shuffle_vector_builtin/bit_cast_builtin` | Vector operation names, lane constraints, size/type relations; also unrelated vector attributes/operators | Earlier `67232f36`; high. Registry identifies calls/forms; vector semantics remains a subsystem. |
| B14 `S/expressions/typing.rs::implicit_function/builtin_name/type_call`; `S/expressions.rs::expression_done` | Implicit binding creation and precedence among common, atomic, vector, generic and ordinary calls | H2, H6, H9; high. Resolve descriptor once, after visible user binding lookup; one call-check entry. |
| B15 `S/traversal.rs::expression` | Prebinds reserved designators before reduction, decides modeled keyword recovery taint | H6; high. Binding origin and descriptor ID available at discovery; no second spelling policy. |
| B16 `L/builtins.rs::schedule_builtin/finish_builtin/schedule_keyword/finish_keyword/overflow` | Separate named/keyword dispatch, evaluation policy and executable implementation | H2, H5, H6; high. Checked operation enum dispatch; keep integer overflow algorithm and IR emission. |
| B17 `L/builtins/floating.rs::floating_builtin/floating_sign_byte` | Runtime classification/sign-copy implementation and format details | H11; medium. Keep implementation; operation identity/type arrive from checked facts. |
| B18 `L/expressions.rs::direct_callee` | Intrinsic vs library call rejection; returns-twice spelling list | H6; high. Call facts plus shared external-symbol contract. |
| B19 `L/module_items.rs::declare_items/function` | Alias runtime names and collisions, implementation binding check | H2, H8; high. Retained entity/symbol request; no builtin name query. |
| B20 `L/address_taken.rs::find` | Recognizes varargs to force list storage for named and keyword forms | H6; medium. Retained storage-address uses. |
| B21 `preprocessing/token.rs::KeywordTokenType::classify/spelling` and aliases | Reserved special-form token identities and gates; expression-only va names deliberately remain ordinary identifiers | H2; high if changed carelessly. Keep the only keyword classifier; descriptor references keyword IDs rather than introducing another lexer classifier. |
| B22 `preprocessing/language_features.rs::has_builtin` | Combines keyword modeled and named predicates; separate language/predefined macro machinery | H2; medium. Registry semantic-capability query, independent of local bindings and backend support. `__LINE__`, `__FILE__`, feature macros etc. remain preprocessing facts. |
| B23 `parsing/extensions/gnu.rs::GnuFrame::step`; `parsing/frames/expression.rs` operand dispatch; `parsing/construction.rs::Parser::with_upstream` | Special operand grammar/arity, expression introducer dispatch and builtin va-list/type-alias visibility | H2, H6; medium. Parser owns grammar; registry supplies special-form shape where needed. Semantic arity/type diagnostics remain distinct. |
| B24 `configuration/features.rs::Feature` (origins/availability) | GnuBuiltins, VectorBuiltins, BuiltinVaArg and extension policy vocabulary | H2; medium. Keep canonical configuration; descriptors refer to `Feature`, never duplicate dialect tests. |
| B25 `src/headers.rs`, 34 top-level `src/headers/*.h` plus `headers/bits/floatn.h`, `target/predefined.rs`, per-target macro headers | Resource intrinsic/macro spellings, target capability promises and compiler identity | Earlier `521873d6`, H10; medium. Header interfaces and target macros remain separately owned; registry coverage tests check every referenced builtin and advertised capability. Do not equate macro promises with completed backend support. |
| B26 `scripts/generate_x86_builtins.py` | Discovers header calls through Clang, derives signatures/immediates and verifies targets | B05/B06 history; medium. Emit descriptor shard instead of a parallel table; do not regenerate shared LLVM. |
| B27 `optimizer/inline.rs::returns_twice/CallGraph::compute` | External-name contract duplicated with lowering; unsafe frame propagation derives from IR | H6; high. Shared catalog projection and retained IR flags, with textual-IR checks. |
| B28 `backend/interpreter/host.rs::DefaultHost::call/check_signature` | Runtime library operations/signature table; overlaps library prototypes and contains host-only names | H9, H11; high. Share neutral signatures on the intersection; host-only descriptors need not create C builtin aliases. Keep runtime effects and validation. |
| B29 `src/ir/` opcode contracts; `backend/llvm/function.rs`; interpreter arithmetic/execute/varargs | Executable meaning of bit/overflow, stack, varargs, trap and memory intrinsics | H6, H10; medium. Keep IR semantics independent of C registry; lowering operation IDs select these implementations. Registry effects must not replace IR poison/UB/memory contracts. |
| B30 `S/declarations.rs::resolve_spec` | Recognizes builtin va-list and int128 typedef spellings independently of ordinary typedef lookup | Earlier int128/header work; medium. Registry has separate builtin-type descriptors, referenced by parser visibility and this type-resolution producer; preserve the current reserved-type contract. |
| B31 `S/integer.rs::evaluate/valid_ice_operands` | Modeled builtin/selected constant eligibility, type-generic constant exception and recovery exclusions | Earlier `34578b67`, H5; high. Consume retained constant/evaluation facts and registry support; do not equate an intrinsic's folded integer with an ICE. |
| B32 `S/errors.rs::SemanticErrorKind` and diagnostic conversion | Builtin-specific constraint identities, severities, notes and rendered messages | H5/H6 diagnostic goldens; medium. Descriptor diagnostic adapters refer to this one catalog, rather than copying messages or severity into a second table. |

## Ownership rule and retained interface

**Semantic analysis is the single source of truth for every checked
language-level fact consumed after it. Each fact has exactly one producer.
Consumers query retained facts; they do not walk syntax or parse names/tokens
to rediscover scope, binding, type, conversion, builtin, effect, lifetime or
initializer decisions.** A pure shared descriptor is the source for static
builtin contracts; semantic analysis is the producer of their checked instances.

Lowering may inspect the current syntax node's operator/statement form and its
immediate children to schedule execution in the existing explicit task stack.
It may read source vectors, literal IDs and node identity for diagnostics and
lookup. It may choose a permitted operand evaluation order, take aggregate
snapshots, create SSA variables/blocks, select scalar IR instructions, serialize
constants and walk **retained types** to project ABI layouts. It may reject a
retained, valid operation because a backend capability is absent. It must not
perform auxiliary syntax walks to find labels, tails, fields, bounds, declarations
or selected callees, or decide what an argument means from its spelling.
Reading “this node is addition” is execution scheduling; deciding anew which
operand is a pointer, which expression is unevaluated, or which region a goto
leaves is semantic reconstruction.

The seam stays at `SemanticTranslationUnit`, described in
[results.rs](../../src/translation_phases/semantic_analysis/results.rs).
Stage-0 items in [middle-end.md](../../middle-end.md#front-end-prerequisites-stage-0)
already require expression indexing, parameter records and selected fields.
Complete that contract instead of adding a second analyzer behind a lowerer
helper. The existing `address_taken::find` demonstrates the useful shape:
linear iteration over retained records followed by a downstream storage choice.

All retained arrays, canonical nodes, final lookup maps and expression references
live in `Context`'s translation-unit arena (`'tu`). Mutable continuations,
identity maps being built, open scopes, sorting buffers and interning work live
in semantic scratch (`'s`) and are copied/finalized once. No retained value has
`Drop` or a scratch reference. Lowering uses its own resettable function scratch
and IR arena; no semantic record contains an IR block, SSA value, stack token or
target register. Existing completion cells for nominal tags finish before the
facts become consumable. IDs are deterministic typed indices; address keys are
private lookup identity, never printed values.

Proposed public vocabulary lives under
`src/translation_phases/semantic_analysis/facts/`, re-exported by the semantic
entry file; `results.rs` holds the slices and query methods. Add `ExprId`,
`BindingId`, `EntityId`, `FunctionId`, `ScopeId`, `RegionId`, `LabelId`, `JumpId`,
`SwitchId`, `BoundId`, `CompoundLiteralId` and `InitPlanId` as typed indices over
existing/new vectors. Start
with checked `usize` wrappers to avoid introducing a new u32 TU limit merely
for storage. Narrow sparse indices only with explicit overflow handling later.
Continue to expose old inspection ordinals during migration.

Queries return `Option` or an explicit `FactState::{Valid, Recovered, Unsupported}`
where a missing record is possible. `sema.lowerable(context)` remains the error
gate; accepted opaque extensions must remain distinguishable from checked facts.
Never substitute root scope, empty effect, zero bound or ordinary call on a
missing lookup. A lowerable modeled node missing its required fact is a
`MissingFact` diagnostic with provenance, not a fallback syntax analysis.

Rejected alternatives:

| Alternative | Reason to reject |
| --- | --- |
| More shared syntax visitors | Removes some implementation copies but repeats work, leaves independent consumer policies and still requires teaching every new construct to auxiliary walks. Current shared tail/callee helpers are useful migration tools, not the finished interface. |
| A full typed HIR now | Would replace expression/statement scheduling, initializer representation and tests simultaneously. The existing immutable syntax plus retained types/conversions already provides the seam; a compact facts overlay gets the ownership benefit with smaller landings. |
| A semantic analyzer kept alive during lowering | Couples phase lifetimes, exposes mutable lookup/work state and prevents scratch reclamation. Retained output already solves this. |
| A registry containing arbitrary checker/lowerer callbacks | Hides constraints and coverage behind code pointers, invites backend dependencies in sema and makes descriptors hard to audit. Use closed rule/operation enums and dedicated family handlers. |
| One giant builtin checker | Atomics, vectors and x86 have real type-rule depth. Unify recognition and contracts, not those algorithms. |

## Target facts and producers

The types below are proposed shapes, not claimed existing Rust declarations.
Keep records split by family: avoid adding several large optional fields to
every `ExpressionInfo`. Reuse the final expression ordinal map and use dense
small summary arrays plus sparse fact tables indexed by `ExprId` where useful.

### 1. One control and lifetime model

Keep the existing lexical `Scope` records and add an execution-region tree
within each function. The distinction is deliberate: block scope begins at
the block, while a VM identifier's protected region begins at its declaration.
A backward goto can leave a declaration region while staying in the same
lexical block. `ScopeId` and `RegionId` are different IDs into one semantic
control model, not two independently discovered scope systems.

```rust
struct Region {
    parent: Option<RegionId>,
    lexical_scope: ScopeId,
    kind: RegionKind, // Lexical, VmDeclaration(BindingId), StatementExpression(ExprId)
    preorder: usize,
    subtree_end: usize,
    nearest_vm: Option<RegionId>,
    nearest_statement_expression: Option<RegionId>,
    cleanup: CleanupKind, // None or AutomaticVla(BindingId)
}
struct ControlPoint { region: RegionId, function: FunctionId }
struct RegionRun { first: RegionId, stop_before: RegionId }
struct Transition { left: RegionRun, entered: RegionRun }
struct JumpFacts {
    source: ControlPoint,
    target: ControlTarget, // Label, LoopBreak, LoopContinue, SwitchBreak, FunctionExit
    destination: ControlPoint,
    transition: Transition,
}
```

`RegionRun` is a parent-chain slice. `left` enumerates inner to outer;
`entered` denotes the destination-to-common-ancestor chain and can be iterated
in reverse with a scratch stack. Empty runs have `first == stop_before`.
The function root is a real region; function-exit transitions end at it and
include all storage-owning children. Store source vectors beside jump/label
records rather than numeric pointer-derived synthetic scope IDs.

`FunctionControl` holds root region, labels, statement-to-point/exit records,
jumps and switches. `LabelFacts` includes definition statement, resolved
function/local-label identity and point. `SwitchFacts` includes control
expression/type/dispatch point, default `LabelId`, source-ordered cases and
their sorted index. Each case has a target point, converted `Integer` lower
and upper endpoint, ordinal and dispatch `Transition`. A case range stays an
interval, never an expanded table. `StatementFacts` provides lexical entry/exit
transitions for normal fallthrough too; cleanup must not be restricted to
explicit jump syntax. Loop facts provide header/body/step/exit points, so
break/continue targets are resolved with the same body-only contexts that
semantic diagnostics use today.

Produce these facts at `scopes::enter/leave`, `record_vm`, statement discovery,
`LoopBody`, `SwitchReady`, `case_done`, `label` and function finalization on the
existing `Work` stack. Extend continuations to carry statement/expr identities;
current case records retain provenance but not their target syntax identity.
Every VM declaration (including typedefs and pointers to VLA) opens a protected
region. Only an automatic actual VLA object has cleanup storage. Restore the
region cursor on lexical exit. Statement-expression traversal is already in
the analyzer; it enters the same region graph even in conditions, arguments,
initializers, bound expressions and sizeof operands that actually execute.
Nested functions start independent roots and label/control contexts.

At function/TU finalization resolve forward label destinations, then compute
transition common ancestors with **iterative heavy-light decomposition** of the
region tree in scratch. Construction costs O(R); each LCA/path query costs
O(log R), with O(R + J) scratch for R regions and J edges. The same index
finds the first entered VM declaration so the diagnostic can preserve today's
outermost-entered-declaration label without a deep scan per rejected jump.
Do not materialize every ancestor list: nested inputs
with many gotos otherwise require quadratic retained storage. Preorder
intervals give O(1) ancestor queries. A destination's nearest VM region not
containing the source identifies forbidden VM entry; likewise its nearest
statement-expression region identifies forbidden expression entry. Both
diagnostics and lowering read these same points and transitions. Preserve
existing diagnostic order/secondary labels and parser-owned duplicate-default
handling. Switch sorting remains O(C log C) per switch.

Lowering maps `ControlTarget`/`LabelId` to IR blocks, saves dynamic stack state
at retained VLA-start actions, and emits restores for cleanup-marked regions
in `transition.left`. It does not discover scopes from syntax. Tokens are SSA
values in lowering scratch keyed by `RegionId`, so a repeated declaration in a
loop produces a fresh runtime token without changing the semantic region ID.
Return value/tail aggregate snapshots are taken before regions are released.
Leaving a block after evaluating a value must not make that value depend on
freed VLA storage.

For H1, source and destination retain their actual statement-expression
ancestry and common enclosing VLA region; the edge cannot accidentally restore
that enclosing allocation because an auxiliary walk failed to index the label.
Keep entry-diagnostic and storage-cleanup tests paired: a VM pointer/typedef
must forbid entry even when there is no stack allocation to restore.

`__builtin_alloca` is a function-lifetime allocation, distinct from an automatic
VLA. The current stack-restore IR releases all later dynamic objects. Therefore
mixed VLA/alloca lifetime behavior needs an explicit equivalence test before
reusing a save token as a blanket cleanup operation; a function-lifetime object
cannot acquire block lifetime just because it was allocated after a VLA save.
This is a migration risk inferred from the IR contract, not a newly verified
PR finding. Resolving any required allocation strategy belongs to lowering/IR,
with semantic allocation-kind facts as input.

### 2. One declarative builtin registry

Introduce `src/builtins.rs` with static shards under `src/builtins/`. It is
phase-neutral: configuration/target IDs and abstract rules are allowed; syntax,
`Analyzer`, semantic `TypeId`, IR opcodes and backend state are not. Static
descriptor storage costs no TU arena allocation. A generated sorted name index
maps exact names to stable descriptor IDs; parameterized families expand to
explicit entries or generated IDs with one canonical decoder. Merge overlapping
scalar names in the generated x86 shard into the canonical scalar descriptor
after checking their contracts agree: a spelling has one ID, not one per
family. Do not retain
overlapping “try every family” classifiers.

```rust
struct BuiltinDescriptor {
    id: BuiltinId,
    spelling: &'static str,
    availability: Availability,
    syntax: SyntaxForm,              // OrdinaryCall or keyword special form
    designator: DesignatorPolicy,    // DirectCallOnly or AddressableAlias
    type_rule: TypeRule,             // FixedSignature or named family rule
    arguments: &'static [ArgumentRule],
    effects: BuiltinEffectClass,
    operation: LoweringOperation,    // Intrinsic, Library, Fold, Special, Unsupported
}
struct ArgumentRule {
    conversion: ConversionRule,
    evaluation: ArgumentEvaluation, // Value, StorageAddress, EffectsOnly, Unevaluated, Type
    constraint: ConstantConstraint,
    diagnostic: ConstraintDiagnostic,
}
```

`Availability` expresses `Feature` origin/gate, target applicability and
environment/naming policy: reserved alias vs ordinary implicit library name,
hosted/freestanding, and plain GNU `alloca`. Keep semantic availability separate
from backend capability. Preserve current `__has_builtin` behavior initially:
it advertises modeled semantics, including families whose lowering is unsupported,
and does not consult a local binding. Future target/CPU restrictions require
evidence and tests, not guessed feature promises imported from a header name.
The four current targets are x86-64; this document makes no claim of other-target
support. A separate `BuiltinTypeDescriptor` shard records reserved va-list/int128
type names, their parser visibility and semantic type rule. It shares the
registry index but is not manufactured into an ordinary call descriptor.

`TypeRule` uses closed variants such as `Fixed`, `GenericOverflow`, `Atomic`,
`X86Signature`, `VectorShuffle`, `ConvertVector`, `TypeClassify`, `Math128`,
`VaArg`, `Offsetof` and `ChooseExpr`. Family handlers remain in semantic
analysis. The descriptor is their only recognition/contract entry. For complex
relations it names a family constraint, whose implementation and diagnostic
identity have one owner; declarative does not mean encoding an entire type
checker into a mini-language.

Constant constraints explicitly specify **which value is checked**:

| Argument | Retained contract |
| --- | --- |
| Object-size mode | Require ICE, convert to target int, check [0,3]; query result is sema's retained conservative constant and neither argument executes. |
| Expect probability | Require arithmetic constant, convert/round to double, check finite [0,1]; preserve endpoint-rounding regression. |
| Assume-aligned alignment | Require ICE converted to size_t, check allowed nonzero power of two; optional offset is a runtime value. |
| Prefetch rw/locality | ICE in original operand integer representation after default-promotion policy, no invented fixed int narrowing; check [0,1]/[0,3]. |
| X86 immediate | Prototype conversion, then ICE/range/bit-mask requirement from generated descriptor. |
| Atomic lock-free size/order | Size conversion/fold rule and operation-specific order validation; invalid constant orders preserve current warning severity. |
| Choose-expr condition / overflow-p destination | ICE truth with no invented formal int; type/effects-only destination with retained bit-field precision and no value promotion. |

Run assignment/type compatibility before constant constraints; suppress dependent
value errors when conversion failed. `ConstraintDiagnostic` names existing
`SemanticErrorKind` through a semantic adapter, severity/policy and offending
operand. Preserve written spelling and source vectors, including macros.

Binding lookup still comes first. `BindingOrigin::{User, Intrinsic(BuiltinId),
ImplicitOrdinary}` replaces the coarse builtin bool; a local object/function
pointer, user prototype, definition, enumerator or assembler label never becomes
an intrinsic by spelling. An unknown reserved builtin remains opaque/unsupported,
not an invented int function. Semantics creates `BuiltinCallFacts` with descriptor
ID, normalized operands, resolved type rule output, converted constants, chosen
operation and effect/evaluation policy. Lowering dispatches that operation enum
and does not run `common_builtin` or create a synthetic `Builtin` AST node.

Library aliases share signature data with `ExternalSymbolDescriptor` records.
Those also cover host-only operations and the returns-twice name contract without
automatically adding C reserved aliases. Interpreter host checks project these
neutral signatures to target IR types and still reject incompatible textual IR
before effects. Optimizer checks use the same external catalog or IR-projected
attributes, preserving checks for textual functions, wrappers and address uses.
Dynamic-alloca/frame exclusions are derived from IR opcodes as today.
Status (2026-10-16, #50): the returns-twice and environment part of this
catalog exists as `builtins/external.rs`. Semantic analysis retains it in
`EntityAttributes::returns_twice`, lowering projects that onto the IR
function flag `returns_twice`, the textual parser projects the catalog onto
unmarked external declarations, and the inliner, LLVM emission, the native
support check and the interpreter read the flag or the catalog; the lowering
and inliner spelling lists of C34/B18/B27 are gone.

Atomics retain their operation/object/order checker; x86 retains its generated
signature/immediate data (386 names); vector and type-generic math keep their
algorithms. Each exports descriptor shards into one registry, not a new public
recognition predicate. `LoweringOperation::Unsupported(reason)` is explicit for
semantic-only support. Scalar folds and special forms have checked payloads;
IR instruction implementations remain in lowering and the back ends.

### 3. One expression effect/evaluation summary

Retain a dense `ExpressionEffects` array parallel to expression ordinals, with
two context summaries: evaluation when a value is read and evaluation when only
designation/unused effects are required. Distinguish observable writes/calls,
volatile accesses, control transfer and dynamic lifetime operations; use an
explicit `Unknown` state conservatively. This is **not** an optimizer promise
that “pure” arithmetic cannot trap or produce poison.

Produce both contexts after children complete in `expression_done`; combine
statement-expression bodies at their closing continuation. Consult builtin
argument evaluation rules and retained selections, never inspect unselected
arms as though they execute. `sizeof` uses its size-query evaluation facts;
VLA bounds are not automatically pure. A volatile lvalue's address is different
from reading it. ICE eligibility remains its own C constraint and does not
become interchangeable with effect freedom.

Where effects-only lowering needs control, retain a sparse recipe:
`None`, `EvaluateValue(ExprId)`, `Sequence(uses)`,
`Conditional(condition, true_recipe, false_recipe)`, or
`StatementBlock(statement_id)`. A use carries read/designation mode. Omitted
conditional operands use the saved condition once. Recipes refer to already
retained expressions/control facts rather than copy syntax subtrees.
`builtin_effects.rs` then interprets recipes on the lowering task stack; it
does not reclassify operators/volatility to rediscover what must execute.
Ordinary execution scheduling can still read the current node's structure.

Cost is O(E + operand edges + statement items), two summaries per expression
and recipes only where needed. Delete the lowering cache and the semantic
per-query `EffectSummaries::new` path once producers/consumers use retained
facts. Keep the deep visit-counter and volatile/address/short-circuit tests.
If a child is not yet complete, schedule a continuation; do not recurse or
perform a fresh suffix walk.

### 4. Calls, selections, tails and members

`CallFacts` stores `Callee::{Direct(EntityId, BindingId), Indirect(ExprId),
Builtin(BuiltinCallId)}`, the **point-of-use** function type, fixed argument
count, prototype/ellipsis status, and argument type/conversion references.
Only a designation-preserving, effect-free callee with the correct decay and
no conversion-changing child receives `Direct`. Preserve the current indirect
fallback cases from `direct_callee`; this migration is not an optimization that
turns arbitrary pointer expressions into direct calls.

Compute a normalized designation result bottom-up alongside expression records:
selected leaf, binding and whether a conversion intervenes. `_Generic`,
`choose_expr`, parentheses and `__extension__` reuse children's results in O(1)
per edge; no repeated chain walks. Keep selected operand identity for ordinary
evaluation and static-address facts. Call finalization may consult finalized
entity definition state, but must not overwrite conversions/types determined
at the source occurrence.

`StatementExpressionFacts` stores block/outer-tail statement identity,
`Option<ExprId>` value tail, prefix length, converted result type and body-effect
summary. Produce tail identity at block/label completion using continuation
results, rather than scanning a deep label chain at every enclosing consumer.
A nested compound final statement still yields void; labels preserve tail
identity and must execute before it. `MemberFacts` stores owning tag and field
index, access type/alignment and bit-field facts; `selected_field` becomes an
index query instead of peeling wrappers to recover the record. Union casts
keep the already retained selected member type (`operation_type` today), with
an explicit operation variant instead of overloading that field further.

### 5. Parameters, occurrences and runtime sizes

At parameter-list construction retain the prototype binding ID with each
`Parameter`, then record the exact prototype-to-body alias when
`function_body` creates its body binding. For K&R definitions, `old_signature`
records declared body type, promoted incoming type and original bound-bearing
type after the declaration list is checked. `FunctionRecord` holds ordered
`ParameterFacts` and its alias list; missing/duplicate/unnamed parameters remain
explicit invalid/recovered entries. This costs O(parameters), not
O(functions * TU bindings). Lowering aliases storage by these IDs after incoming
parameters are initialized, then evaluates entry-bound actions.

Retain `DeclarationFacts` keyed by declaration/declarator occurrence identity,
with binding/entity, initializer plan and ordered bound actions. A declarator
is a copied value in some interfaces: use its stable suffix/list identity plus
occurrence ordinal established by traversal, not a pointer to a temporary copy.
Retain type-name identity -> resolved type/size-query record directly. Source
vectors are provenance, never joins between semantic occurrences.

Keep existing unique `ArrayBound::Variable(BoundId)` identities. Extend each
`RuntimeBound` with typed expression, required conversion, origin/evaluation
site and dependencies. Retain actions on the **site**, not merely the type:
declaration, function entry, cast/type operand or runtime sizeof. Typedef use
reuses the saved extent; each runtime declaration execution evaluates the
scheduled bound once. Array adjustment must not erase definition-parameter
actions, including evaluated outer bounds.

`SizeFormula` is a shared arena DAG of constant byte size and products with
`BoundId`s, built/cached during type construction in O(new derivations).
Pointer arithmetic/subscript/increment and sizeof reference formula IDs.
`SizeQueryFacts` says precisely which bound/operand effects execute; it does
not infer them from “layout is missing”. Lowering assigns runtime SSA extent
variables and emits multiplications/loads for these facts. Do not fold or
deduplicate distinct source evaluations because their bound expressions happen
to be identical.

### 6. Initializer plans and symbolic constants

Extend `check_initializer`'s existing `InitWork`/`Current` machine to emit the
accepted current-object plan as it checks it. Retain plans for object definitions,
automatic declarations and compound literals, keyed by initializer occurrence
and destination type. This must become **one** current-object traversal, not a
third pass that copies lowering's walker under the semantic directory while the
original checker remains.

```rust
enum InitOperation {
    Evaluate { value: InitValueId, expression: ExprId, conversion: ConversionRef },
    Store { value: InitValueId, destination: SubobjectId },
    Copy { value: InitValueId, destination: SubobjectId },
    String { literal: LiteralId, destination: SubobjectId, copied_units: u64 },
    Reset { destination: SubobjectId, replacement_scope: InitScopeId },
    Repeat { dimensions: RepeatId, operations: OperationRun },
}
```

`SubobjectFacts` includes completed destination type, resolved member path/byte
offset, guaranteed alignment, qualifiers and bit range. `InitValueId` belongs
to an initializer occurrence, not an expression-address deduplication heuristic.
`Repeat` retains compact inclusive range dimensions and one evaluation action
outside the repeat. Nested ranges use explicit cursors when emitting, avoiding
retained output proportional to the Cartesian product. Overlapping writes,
replacement/union reset scopes and later-explicit coverage are part of the plan;
implicit zeroing must not erase explicitly initialized bytes or neighboring
bit fields. Preserve current source-order evaluation choices and already tested
overridden-initializer behavior initially. Sparse arrays retain zero ranges,
not one record per omitted element.

Incomplete arrays finish their type/extent before offsets and destination facts
are finalized. Offsets can be accumulated symbolically in scratch until this
completion; do not query an unfinished outer layout. Keep diagnostics, recovery
taint and extension reporting in the producer. Automatic emission chooses
stores/copies/zero chunks and snapshots aggregate values. Static emission maps
the same plan to bytes and relocations. Efficient overlap indices can operate
on plan operations downstream, but they must not choose the language's current
object or union member again.

Retain `SymbolicAddress { base, byte_addend }`, with base
`Entity(EntityId)`, `String(LiteralId/occurrence)`,
`StaticCompoundLiteral(CompoundLiteralId)` or `Absolute`. Reuse/extend
`expressions/address.rs::AddressBase` and compute decomposition bottom-up
using resolved fields, selections and pointer stride facts; static initializer
checking consumes it too. Use a checked wide addend (i128) until the IR
relocation adapter checks its i64 limit. Label addresses/differences remain
explicit unsupported facts until code-label relocation exists.

Retain destination-converted `ConstantValue` for static stores, separate from
original expression category/ICE class. Semantic integer/floating conversion
uses one target scalar conversion module shared with IR constant encoding where
necessary. Lowering serializes the resulting bits or maps symbolic bases to
IR globals/functions; it does not repeat C cast/address eligibility rules.
Binary128 non-folded arithmetic stays explicit and unsupported where it cannot
be materialized; do not approximate it through x87. Runtime conversions still
emit IR and preserve its poison/UB rules.

### 7. Entities, symbol requests and operation facts

Retain finalized `EntityFacts`: linked occurrence IDs, definition ID/body
emission decision, effective symbol request, initializer plan, and whether an
external unprototyped declaration has a known definition. Keep point-of-use
binding types separate from final entity types. Semantic analysis already owns
inline/tentative-definition policy; expose that state instead of reconstructing
name sets in `Unit::new`. Retain `FunctionEnd` during function finalization;
initially `MainZero` reproduces the current name-only rule for equivalence.
Applying additional standard/hosted distinctions requires a separately evidenced
behavior change, not a silent correction hidden in this ownership migration.

Decode assembler labels and convention/attribute syntax while semantic type
and declaration traversal already has it. Retain `SymbolRequest` with decoded
name/provenance and linkage propagation, plus `CallingConvention` on callable
types/occurrences. Lowering refuses unsupported conventions and unrepresentable
symbol requests through its current capability diagnostics. Keep collision
checks for two IR definitions sharing one name, generated-symbol naming and
backend restrictions downstream. Attribute acceptance is not a promise that
the selected backend can implement it.

Add small `OperationFacts` only for language choices not already captured by
types/conversions: arithmetic vs pointer/index/difference, increment promoted
type and conversion back, semantic overflow behavior, stride formula, and
compound-update single-evaluation requirement. Lowering owns RHS-first scheduling
and aggregate snapshots. Reading signedness from a retained scalar to select
`sdiv` is permitted; recomputing integer promotion from width is not.

Expose canonical type/object predicates for readonly storage, volatile access,
VM status and natural member layout. `contains_const` (modifiable-lvalue
constraint) and whole-object readonly storage must stay distinct. Cache ABI
aggregate projection in lowering by completed type; it is not sema's job to
choose eightbyte classes or IR scalar access pieces. Centralize target data
model/format knowledge under existing target/scalar utilities, with semantic
and ABI consumers sharing it rather than copying target tables.

## Migration: eleven independently landable steps

Sizes below are estimates of hand-written implementation/test lines touched,
including deletion/moves, not promised net growth or measured schedules.
Generated descriptor data and documentation are counted separately. Each step
is its own commit or PR. Transitional comparison implementations belong in
tests/debug instrumentation and are deleted in the same step that switches
the last production consumer. Never leave a permanent fallback analyzer.

| Step / estimated size | What moves and what is deleted | Required equivalence and acceptance evidence |
| --- | --- | --- |
| M0 Baseline and fact test seam; 150–300 lines | Add retained-fact inspection assertions/work counters and freeze exact starting revision, source hashes, compiler digest and survey options. No facts change yet. | Existing semantic/diagnostic and lowering IR goldens; capture all four torture arm outcomes, not only totals. Preserve original raw failures and reference limitations. Baseline after any landed PR #42 repairs, with a separate pre-repair record if available. |
| M1 Shared control/lifetime facts; 700–1,200 | Retain regions, labels, jump/loop targets, cases/default and transitions during semantic traversal. Switch diagnostics and lowering to them. Delete `index_vla_scopes`, `index_declaration`, pointer/binding synthetic region IDs, `scope_parents`/`label_scopes` reconstruction, body case-discovery walk and endpoint recasts. | Pair semantic VM/expression-entry goldens with VLA execution/memory-bound tests; add H1 internal expression-label case, forward/backward/sibling goto, two declarations in one block, VM typedef/pointer, header jumps, nested switch, ranges and normal exits. Check mixed VLA/alloca and tail-result lifetime. Existing fixed-array goldens unchanged; any VLA IR difference must show the corrected transition. Deep counts bound construction/LCA cost. |
| M2 Occurrence/parameter/bound correspondence; 350–650 | Retain declaration/type-name identities and parameter binding aliases; emit bound-site/size-query/stride facts. Delete source-vector binding/type-name joins, including inspection's static-minimum map, whole-TU parameter scans and type-based bound scheduling rediscovery. Retain runtime SSA size emission. | Prototype/K&R/unnamed/register/shadow parameters; bound side effects on entry/declaration, typedef reuse, casts, sizeof and multidimensional strides. H10 fixtures and four-target semantic/inspection assertions. Generated many-function test counts work proportional to parameters plus bindings, not their product. |
| M3 Resolved expressions/calls/tails; 300–550 | Retain normalized designations, call type/shape, tails and owning-field identity. Delete consumer uses of `selected_designator`/`statement_expression_tail`, including `selected_field` syntax recovery and direct-callee repeated queries. | H4/H6 selected builtins, varargs, function pointers, conversion-changing wrappers, void/nested-block/labeled tails, member qualifiers/bit fields. Golden IR unchanged for valid supported cases; include all existing conservative indirect-call cases. |
| M4 Registry and scalar checked instances; 600–1,000 | Introduce neutral descriptor index and scalar/library contracts. Retain `BindingOrigin`/checked scalar and keyword calls. Delete scalar name redispatch in lowering and synthetic varargs AST adapters; old family recognition bridges are temporary and explicit. | Compare descriptor IDs/signatures/support against old scalar/library predicates in tests, then remove them. H2/H5/H6 goldens, all builtin spellings/suffixes, shadowing, strict/GNU/pedantic, hosted/freestanding, all targets; conversion boundary/128-bit immediate tests; same IR operation/fold/effects. |
| M5 Retained effects/recipes; 400–750 | Produce dual-context summaries and sparse recipes once; delete lowering `effect_summaries`, fresh per-semantic-query purity caches and effect-only syntax classification. | Deep effect visit counters; calls/volatile reads/address-only uses, selected/unevaluated args, overflow-p bit fields, short circuit and omitted conditionals, labeled statement blocks, VLA sizeof effects. Interpreter memory/effects and IR goldens; preserve constant eligibility separately. |
| M6 Register remaining builtin families; 500–900 plus generated data | Move atomic/x86/vector/type-generic descriptor shards and generated table output; keep dedicated semantic handlers. Delete `names.rs` union recognizer and family public name predicates; `__has_builtin` queries registry. | Exhaustive name support comparison, generated x86 signature/immediate four-target checks, header-call coverage, atomic order warnings/lock-free folds, vector lane/type goldens, binary128/type-generic tests. Unsupported lowering remains the same refusal, not an external symbol. |
| M7 Entity/symbol/function policy; 400–700 | Retain symbol requests, resolved conventions, finalized entities, function-end policy and object predicates. Delete lowering attribute-token parsing, label decoding/inheritance, known-definition name reconstruction and name-based main-end policy reconstruction. Share external signatures/returns-twice catalog. | H8/H9 user/asm alias collisions, linked redeclarations, implicit/open calls, inline modes, all target convention refusals and main end behavior. Host malformed textual-IR tests and returns-twice wrapper/frame exclusions must still pass. Preserve refusal stage/text initially where capability belongs downstream. |
| M8 One initializer plan producer; 800–1,400 | Extend semantic current-object checker to emit plan; automatic/static lowering consumes it. Delete `Walker`, `leaves`, field-name designator walks, braced-scalar/whole-copy rediscovery and repeated-expression counting heuristic. Keep efficient plan coverage/emission helpers where needed. | H7/H11 replacement/string/union/bit-field/range tests, exact static bytes/relocations and automatic effects, incomplete arrays, packing/alignment, recovery goldens. Deep/sparse/overlapping-range work counters. Old/new plan comparison on accepted fixtures before deleting old walker; no blind golden blessing. |
| M9 Symbolic/converted constants and remaining operation facts; 400–750 | Retain address decompositions and destination constants; consolidate scalar constant conversion helpers and explicit increment/pointer operation facts. Delete `address_constant` syntax walk, static C conversion rechecks and width-based promotion inference. Cache retained-type ABI projections. | Static compound/array-member/selected/&* addresses, absolute/null pointers, 128-bit integers, all floating formats/signed zero/NaN, GNU address differences, narrow/enum/bit-field increments and pointer strides. Golden bytes/relocations and IR; two-direction Clang ABI conformance unchanged. |
| M10 Enforce seam and rewrite design docs; 200–400 code/test lines plus docs | Remove raw `ParsedTranslationUnit` argument from lowering/pipeline if retained function/definition/declaration records now supply all roots. Make traversal helpers semantic-private; add architecture check and rule text below. Rewrite roadmap/history into current contracts, link archived evidence. | Zero consumer syntax-traversal helpers or builtin classifiers; no sema/syntax imports in ABI/optimizer. Canonical suite, allocation tests, differential tests and fresh matched torture survey unchanged or improved for every previously passing arm. Inspect final documentation links and `git diff --check`. |

Status (2026-10-18): M6 is implemented; every builtin spelling and keyword
form has one registry descriptor and `__has_builtin` queries the registry. See
[the M6 receipt](../research/semantic-facts-m6.md) and the "Builtin families in
the registry" section of `semantic-analysis.md`.

For each landing, run focused tests while developing, then repository canonical
checks: `cargo test --all-targets`, feature-enabled allocation counting, nightly
format check and both required Clippy configurations with warnings denied.
Do not overlap feature-enabled benchmark builds with CLI execution tests.
After them rebuild the ordinary CLI where the test workflow needs it. Run
`BCC_DIFFTEST_CASES=1000 cargo test --lib difftest` for behavior-bearing landings;
extend generated programs to cover a migrated fact family when the existing
generator lacks it. These are implementation acceptance requirements, not
checks claimed executed by this documentation task.

Run full matched GCC execute surveys for M1, M6, M8 and final M10, plus targeted
affected-source subsets for intermediate steps. Retain per-source arm statuses,
source hashes, flags, compiler digest, host and deadlines in `docs/research/`
receipts. A historical report or an old cached binary is not a new baseline.
Use bundled Clang O0/O2 and existing pre/post-ABI interpreter/native arms for
defined supported cases; use GCC for documented Clang limitations (header
jumps and nonconstant range behavior). Record skipped native arms as untested.
Do not weaken expected semantic constraints or classify an old raw failure as
an improvement merely by changing its label.

No build may modify/delete/rebuild shared `target/llvm` or `target/llvm-libs`.
Run compiled programs only through `python scripts/cargo_runner.py <program>`;
existing cargo/test/survey runners already apply that protection.

### Relationship to in-flight PR #42 repairs

Land correctness regressions and narrow repairs before this migration; do not
hold a P1 fix for a redesign. The supplied H1 VLA-walk repair becomes obsolete
as an implementation in M1, while its reproducer remains a permanent control
fact/cleanup regression. If that repair retains semantic regions already,
adopt it as the beginning of M1 instead of implementing another tree.

The already merged VLA/parameter-bound work moves into M1/M2, aggregate-varargs
IR/ABI implementation remains in place, and representation/range/static work
moves into M7/M8/M9 without discarding execution coverage. Existing shared
callee/tail repairs move into semantic producers in M3; constant-argument
repairs become descriptor-driven constraints in M4/M6. Any in-flight finding
about these families should patch the existing owner now and supply a regression
that follows that fact into its new owner. Findings about actual ABI packing,
interpreter execution, instruction semantics or optimizer transforms remain
in those modules. No other PR #42 finding or repair status is asserted here:
this worktree does not contain their future commits.

## Guardrails

Add the following text to `.agents/AGENTS.md` during M10 (not in this
documentation-only change):

> **Semantic fact ownership:** After semantic analysis, language decisions come
> from `SemanticTranslationUnit` retained facts. Before changing lowering,
> identify the semantic producer and its query. Fix that producer and add a
> retained-interface regression before changing a consumer. Do not add syntax
> prepasses, provenance-based binding joins, builtin spelling classifiers, or
> consumer fallbacks for missing facts. Lowering may inspect immediate syntax
> to schedule execution and provenance, and may project retained types to IR;
> it must not rediscover bindings, scopes/lifetimes, selections, conversions,
> argument evaluation modes or initializer placement. Builtin contracts have
> one descriptor; checked instances are retained. Document backend capability
> refusals separately from semantic acceptance. See `docs/design/semantic-facts.md`.

Add this text to `CONTRIBUTING.md`:

> **Retained language facts:** Each language fact consumed by the middle end
> has one producer in semantic analysis (static builtin contracts live in the
> shared registry). A new construct extends that producer and its retained
> interface rather than teaching independent consumer walks. A fix includes
> tests through the same retained query used by consumers and execution or
> diagnostic evidence appropriate to the fact. Syntax identity indexes facts;
> source vectors explain diagnostics and do not join semantic occurrences.
> Keep traversal iterative and allocation arena-only. Downstream execution,
> IR optimization and target ABI planning retain their own responsibilities.

Future review brief:

> For each changed language decision, name its unique producer, retained record
> and consumers. Check that lowering consumes it without reclassification.
> Flag a consumer-only patch when the producer is missing or wrong, and show
> the construct for which the independent consumer disagrees. Check bindings,
> discarded/unevaluated effects, embedded statement expressions, forward and
> backward edges, recovery and asymptotic work. Do not report ordinary IR/ABI
> implementation as semantic duplication. Preserve independent runtime and
> textual-IR verification even when C semantic analysis checked the source.

A mechanical check is feasible but incomplete. Make `selected_designator`,
tail discovery and builtin recognizers private to semantic producers as soon
as their consumers migrate; Rust visibility provides the strongest simple
guard. Add a repository architecture test/script that scans production imports
under lowering for semantic **traversal** helpers and raw builtin classifiers,
and disallows syntax/sema dependencies in ABI/optimizer. Maintain a small
documented migration allowlist with owning step; require it to shrink to zero.
Also flag source-vector equality joins and fresh auxiliary syntax worklists
for review, allowing legitimate execution `Task` stacks.

Do not ban all `ExpressionType` or `StatementType` imports: the allowed execution
walk still needs them. `.clippy.toml` `disallowed-methods` can block named
functions globally with producer exceptions, but it cannot naturally express
“only forbidden in lowering” or detect an inline match that reimplements a
walk. Prefer visibility plus the scoped architecture test; use Clippy only for
specific remaining escape hatches. No mechanical grep proves semantic
ownership. Descriptor completeness tests must cover unique names, every keyword
form, every executable/unsupported operation variant, header references,
availability policies and signature/effect agreement on shared host symbols.

## Documentation policy

`semantic-analysis.md` should describe the phase as it exists: retained records,
producer/query contracts, ordering, diagnostics/recovery, complexity and known
unsupported features. `middle-end.md` should describe consumption, storage/SSA,
IR/ABI contracts and capability limits. Replace the stale stage-0 “all done”
claim with the actual completed interfaces, including explicit correspondence;
remove obsolete “no lowering exists yet” explanations when touching related
records. Stage history is not an excuse to leave contradictory current claims.

During each migration landing, replace its family's PR-round sections with one
design description. Preserve useful constraints and counterexamples in that
description (converted immediates, labeled tails, header contexts, VLA lifetime,
dynamic-alloca inlining). Move chronology, exact commands/counts, oracle failures,
before/after measurements and source hashes into a commit-qualified report under
`docs/research/`, linked from the design. Existing GCC reports remain historical;
append a new receipt instead of overwriting their numbers. Commit messages record
the reason for each change. `language-standards.md` keeps availability/feature
status and links the registry contract; it does not acquire another builtin list.

Archive the PR #41 round-1/2/3/5/6 sections together when their owners migrate;
the shared-helper follow-up can be linked by commit. Deduplicate repeated
validation prose without deleting the only evidence of a limitation. M10 checks
that both design docs agree and contain no round-numbered current architecture.
This proposal file remains the decision record; after implementation, mark
completed steps and link the current interfaces rather than accumulating a new
review-round diary here.

## Costs, measurements and migration risks

Current facts already occupy retained memory: expression records/index,
conversion runs, binding/scope/type graphs, function records and runtime-bound
expression references. `ExpressionInfo` already has selected expression,
operation type, binding, bit-field and field-index storage; the source notes
that the four-byte field index used existing padding. Current function records
retain two parameter slices; runtime bounds retain one expression pointer each.
`tests/allocation_count.rs` tests **global allocator calls**, not retained bytes
or peak arena residency. No current `size_of::<ExpressionInfo>()` measurement
or semantic retained-byte result was found in the inspected tests. Inferring
exact Rust struct sizes from source would be unreliable; none is claimed here.

A quick read-only count on
`../bcc-rust/target/compiler-corpus/gcc-15.2.0/gcc/testsuite/gcc.c-torture/execute`
found 1,677 top-level `.c` sources. After removing comments with a simple regex,
489 contain `__builtin_` spellings, 62 contain `goto`, 64 contain `switch`, 10
contain a `({` spelling and two contain a bracketed range-designator spelling.
These are **lexical exposure counts**, not analyzed expression/region counts:
the scan does not preprocess includes/macros or distinguish strings/disabled
branches. The research report's six range-dependent sources, for example,
include evidence reached through other text. This count only supports testing
priority; it cannot predict retained memory. The generated x86 table contains
386 builtin-name entries and resources contain 34 top-level header files plus
the `bits/floatn.h` shim (35 headers in total) at this head.

Provisional storage budgets, to be measured rather than treated as Rust layouts:

| Family | Target representation / scaling | Illustration on a large artificial TU |
| --- | --- | --- |
| Effects/designation lookup | Aim for 2–4 bytes/expression effects plus sparse/dense normalized IDs chosen by measured prevalence; avoid several pointer-sized optional structs in every expression | At 100,000 expressions, 2–4 bytes is 0.2–0.4 MB; an additional 8-byte ID array is 0.8 MB. |
| Regions/edges | O(R + J + cases), region records roughly 48–80 bytes and compact jump/transition records roughly 48–80 bytes with current usize IDs | 10,000 regions and 10,000 jumps are roughly 1.0–1.6 MB combined, before lookup maps; explicit full ancestor runs would be unboundedly worse. |
| Calls/builtins/tails | Sparse records per relevant expression, approximately 32–96 bytes plus operands already needed; registry itself is static | 10,000 calls are approximately 0.3–1.0 MB before operand records/maps. |
| Parameter aliases/bounds | O(parameters + bound sites + distinct formula nodes), exact slices copied once | Two usize binding IDs require 16 payload bytes per alias on the supported 64-bit hosts; no TU-sized per-function alias scan/map. |
| Initializers/constants | O(written initializer structure + resolved paths + repeat dimensions), not omitted array extent/range Cartesian expansion | Exact plan operation sizes and path sharing must be measured in M8; downstream bytes necessarily scale with the materialized object. |

These budgets are design estimates, exclude alignment, hash-map capacity and
arena growth waste, and are not a total-memory forecast. Retaining plans longer
increases TU residency; deleting lowerer reconstructions and caches reduces
temporary memory but may not reduce peak memory because lifetimes overlap.
Prefer final-size maps/slices, shared formula/path DAGs and lazy empty families.
Do not allocate a large hash table for a TU with no relevant constructs.

M0/M1/M8 instrumentation should record actual `size_of` values for retained
types, fact counts, TU-arena used/committed bytes, scratch peak and total process
working set at phase boundaries, using existing arena facilities or narrow
test-only counters. Run representative benchmark inputs, the matched torture
corpus, system-header-heavy TUs and generated deep/many-function/sparse-range
inputs. Retain counts and compiler revision with results. Keep allocation tests
to enforce zero global allocation; do not present them as memory benchmarks.

Time should fall where whole-TU parameter scans, repeated selected chains,
effect caches, switch/VLA prepasses and initializer/address walks disappear.
That is an inference, not a measured speedup. Semantic analysis may become
slower and semantic-only CLI output retains more facts; measure semantic-only,
lowering-only and full pipeline separately. Construction targets linear work
plus existing type compatibility/layout exceptions, switch sorting,
O(J log R) transition/diagnostic finalization and actual plan/IR emission. Repeat emission
can legitimately scale with bytes/stores produced; fact construction must not
expand a huge range simply to retain its semantic meaning.
For performance claims, use the repository's controlled comparison guidance
and seven interleaved runs; investigate a reproducible regression above 1%
instead of describing uncontrolled torture elapsed time as a speedup.

The largest migration risks are losing point-of-use types in favor of final
entity types; confusing visibility with allocation lifetime; evaluating
unevaluated/overridden operands; releasing tail storage before copying a value;
changing diagnostic order/severity; and retaining incomplete layouts too early.
Typed IDs, explicit recovered/unsupported state and producer/query tests make
these observable. Temporary old/new comparisons must remain independent enough
to expose disagreement, then disappear when the new interface is accepted.
Do not change every representation at once, bless broad golden differences,
or expand feature support during an ownership migration.

## Review decision

Approve the retained-facts overlay and neutral declarative registry as the
direction, with M1 control/lifetime unification first after baseline capture.
Each later landing must delete its obsolete consumer reconstruction and retain
its regression coverage. A construct should require one language-rule change
and the relevant executable implementation, rather than synchronized edits to
several independent semantic copies.
