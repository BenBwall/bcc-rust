//! Clang fortify contracts must survive semantic checking, lowering and an
//! actual call frame, including hidden parameters interleaved with written
//! arguments. These regressions use interpreter results, not query parity.
use super::*;
use crate::backend::interpreter::{
    RuntimeValue,
    Termination,
    run_text,
};

fn returns(source: &str, expected: u128) -> String {
    let text = lowered(source);
    assert_eq!(
        run_text(&text, "main").result.unwrap().termination,
        Termination::Returned(Some(RuntimeValue::Int(expected))),
        "{source}\n{text}"
    );
    text
}

#[test]
fn size_parameters_interleave_and_forward_without_changing_source_arity() {
    let source = r"
unsigned long sizes(char *const a __attribute__((pass_object_size(1))), int n,
                    char *const b __attribute__((pass_object_size(1)))) {
    return __builtin_object_size(a,1) + n + __builtin_object_size(b,1);
}
unsigned long forward(char *const p __attribute__((pass_object_size(1)))) {
    char other[7]; return sizes(p,3,other);
}
int main(void) { char data[11]; return forward(data+2); }
";
    let text = returns(source, 19);
    assert!(
        text.contains(
            "function @sizes(ptr noundef, i64 noundef, i32 noundef, ptr noundef, i64 noundef)"
        ),
        "{text}"
    );
}

#[test]
fn enclosing_and_subobject_sizes_have_distinct_boundaries() {
    let _text = returns(
        r"
struct S { char before[4]; char member[8]; char after[4]; };
int main(void) { struct S s; return
    __builtin_object_size(s.member+2,0) * 100 +
    __builtin_object_size(s.member+2,1) * 10 +
    __builtin_object_size(s.member+8,3); }
",
        1060,
    );
}

/// Constant pointer steps accumulate signed, so an outer subtraction does
/// not underflow a position an inner addition advances, and a
/// final position before the object leaves no bytes, as in Clang. A
/// position before a member falls back to the complete object for mode 1
/// and leaves none for mode 3.
/// C99: §6.5.6p8, p. 83; PDF p. 95. GNU extension: GCC manual,
/// "Object Size Checking".
#[test]
fn constant_offsets_accumulate_signed_steps() {
    let _text = returns(
        r"
struct S { int a; char b[4]; } s;
int main(void) { char buf[10]; return
    (__builtin_object_size(buf + 4 - 2, 0) != 8) |
    (__builtin_object_size(buf + 4 - 2, 2) != 8) << 1 |
    (__builtin_object_size(&buf[4] - 2, 0) != 8) << 2 |
    (__builtin_object_size(&buf[4] - 2, 1) != 8) << 3 |
    (__builtin_object_size(buf + 12 - 4, 0) != 2) << 4 |
    (__builtin_object_size(buf - 2, 0) != 0) << 5 |
    (__builtin_object_size(&buf[4] - 6, 2) != 0) << 6 |
    (__builtin_object_size(s.b + 2 - 1, 1) != 3) << 7 |
    (__builtin_object_size(s.b - 1, 1) != 5) << 8 |
    (__builtin_object_size(s.b - 1, 3) != 0) << 9; }
",
        0,
    );
}

/// `&a[i]` designates an element of `a`, so mode 1 measures the rest of
/// `a`; the decayed `a[i]` designates the inner array's first element, so
/// mode 1 measures that inner array. Values match Clang.
/// C99: §6.5.2.1p3-4, p. 70; PDF p. 82 (successive subscripts);
/// §6.3.2.1p3, p. 46; PDF p. 58 (array decay). GNU extension: GCC manual,
/// "Object Size Checking".
#[test]
fn subobject_sizes_distinguish_element_addresses_from_decay() {
    let _text = returns(
        r"
int main(void) { char a[2][10]; char b[3][4][5]; return
    (__builtin_object_size(&a[0], 1) != 20) |
    (__builtin_object_size(&a[1], 1) != 10) << 1 |
    (__builtin_object_size(a[0], 1) != 10) << 2 |
    (__builtin_object_size(&a[0][0], 1) != 10) << 3 |
    (__builtin_object_size(&a[1][3], 1) != 7) << 4 |
    (__builtin_object_size(&a[0] + 1, 1) != 10) << 5 |
    (__builtin_object_size(&b[1][2], 1) != 10) << 6 |
    (__builtin_object_size(b[1], 1) != 20) << 7 |
    (__builtin_object_size(&b[1], 1) != 40) << 8 |
    (__builtin_object_size(&a[0], 3) != 20) << 9; }
",
        0,
    );
}

#[test]
fn dynamic_size_uses_saved_vla_bound_and_static_size_is_unknown() {
    let _text = returns(
        r"
unsigned long dynamic(char *const p __attribute__((pass_dynamic_object_size(1)))) {
    return __builtin_dynamic_object_size(p,1);
}
unsigned long fixed(char *const p __attribute__((pass_object_size(1)))) {
    return __builtin_object_size(p,1);
}
int test(int n) { char data[n]; return dynamic(data+2) == n-2 && fixed(data+2) == (unsigned long)-1; }
int main(void) { return test(13); }
",
        1,
    );
}

#[test]
fn overloads_select_distinct_mangled_entities_and_prefer_size_contract() {
    let text = returns(
        r"
int __attribute__((overloadable)) f(int a) { return 3; }
int __attribute__((overloadable)) f(double a) { return 7; }
int g(const char *p) { return 100; }
int __attribute__((overloadable)) g(const char *const p __attribute__((pass_object_size(1)))) { return __builtin_object_size(p,1); }
int main(void) { char data[8]; int (*ordinary)(const char *) = g; return f(2) + f(3.0) + g(data) + ordinary(data); }
",
        118,
    );
    assert!(
        text.contains("@_Z1fi")
            && text.contains("@_Z1fd")
            && text.contains("@_Z1gPKcU17pass_object_size1"),
        "{text}"
    );
}

#[test]
fn enable_if_prefers_a_true_predicate_and_rejects_unknown_values() {
    let _text = returns(
        r#"
int __attribute__((overloadable)) f(int a) { return 3; }
int __attribute__((overloadable)) f(int a) __attribute__((enable_if(a>0,"positive"))) { return 7; }
int indirect(int a) { return f(a); }
int main(void) { return f(2) + f(-1) + indirect(2); }
"#,
        13,
    );
}

#[test]
fn predicate_abi_collisions_preserve_distinct_bodies() {
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        for marked in ["", "overloadable,"] {
            let source = format!(
                "int f(int n) __attribute__(({marked}enable_if(n>0,\"positive\"))) {{return 7;}} \
                 int f(int n) __attribute__(({marked}enable_if(n<=0,\"nonpositive\"))) {{return \
                 3;}} int main(void) {{return f(1)+f(0);}}"
            );
            let text = lower_for(&source, target).unwrap();
            assert_eq!(
                run_text(&text, "main").result.unwrap().termination,
                Termination::Returned(Some(RuntimeValue::Int(10))),
                "{target:?}: {source}\n{text}"
            );
        }
    }
}

#[test]
fn overload_abi_ordinals_follow_first_emitted_use() {
    for (source, target, expected) in [
        (
            "extern int f(struct {int x;}*) __attribute__((overloadable)); extern int g(struct \
             {double x;}*) __attribute__((overloadable)); int main(void){return g(0);}",
            Target::LinuxGnu,
            "_Z1gP3$_0",
        ),
        (
            "static int unused(void) { extern int f(struct {int x;}*) \
             __attribute__((overloadable)); return f(0); } extern int g(struct {double x;}*) \
             __attribute__((overloadable)); int main(void){return g(0);}",
            Target::LinuxGnu,
            "_Z1gP3$_0",
        ),
        (
            "int test(void) { {struct S{int x;};extern int f(struct S*) \
             __attribute__((overloadable));} {struct S{double x;};extern int f(struct S*) \
             __attribute__((overloadable)); return f(0);}}",
            Target::LinuxGnu,
            "_Z1fPZ4testE1S",
        ),
        (
            "int test(void) { {struct S{int x;};extern int f(struct S*) \
             __attribute__((overloadable));} {struct S{double x;};extern int f(struct S*) \
             __attribute__((overloadable)); return f(0);}}",
            Target::WindowsMsvc,
            "?f@@$$J0YAHPEAUS@?1??test@@9@@Z",
        ),
    ] {
        let text = lower_for(source, target).unwrap();
        assert!(text.contains(expected), "{source}\n{text}");
    }
}

#[test]
fn overload_substitutions_ignore_nested_storage_alignment() {
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        let text = lower_for(
            "typedef int I __attribute__((aligned(32))); int f(I*a,int*b) \
             __attribute__((overloadable)){return *a+*b;}",
            target,
        )
        .unwrap();
        let expected = if target == Target::WindowsMsvc {
            "?f@@$$J0YAHPEAH0@Z"
        } else {
            "_Z1fPiS_"
        };
        assert!(text.contains(expected), "{target:?}\n{text}");
    }
}

#[test]
fn hidden_sizes_preserve_aggregate_results_and_variadic_argument_positions() {
    let _text = returns(
        r"
struct R { long first, second, third; };
struct R gather(char *const a __attribute__((pass_object_size(1))),
                struct R input, char *const b __attribute__((pass_object_size(1))), int last, ...) {
    __builtin_va_list ap;
    __builtin_va_start(ap,last);
    int extra=__builtin_va_arg(ap,int);
    __builtin_va_end(ap);
    struct R result={input.first+__builtin_object_size(a,1),
                     input.second+__builtin_object_size(b,1),
                     input.third+last+extra};
    return result;
}

int main(void) {
    char a[5], b[9]; struct R input={1,2,3};
    struct R result=gather(a,input,b,4,7);
    return result.first+result.second+result.third;
}
",
        31,
    );
}

#[test]
fn dynamic_interior_sizes_use_the_evaluated_pointer_once() {
    let _text = returns(
        r"
unsigned long take(char *const p __attribute__((pass_dynamic_object_size(0)))) { return __builtin_dynamic_object_size(p,0); }
int test(int n,int i) {
  char data[n],fixed[13]; int j=2,k=2;
  return __builtin_dynamic_object_size(data+i,1)==11 &&
         __builtin_dynamic_object_size(fixed+i,1)==11 &&
         __builtin_dynamic_object_size(data+k++,1)==(unsigned long)-1 && k==2 &&
         take(data+j++)==11 && j==3 &&
         __builtin_dynamic_object_size(data+n,1)==0;
}
int main(void) { return test(13,2); }
",
        1,
    );
}

#[test]
fn dynamic_subobject_queries_never_exceed_the_nearest_boundary() {
    let _text = returns(
        r"
unsigned long maximum(char *const p __attribute__((pass_dynamic_object_size(1)))) {
    return __builtin_dynamic_object_size(p,1);
}
unsigned long minimum(char *const p __attribute__((pass_dynamic_object_size(3)))) {
    return __builtin_dynamic_object_size(p,3);
}
int bounded(unsigned long size, unsigned long extent) {
    return size == (unsigned long)-1 || size <= extent;
}
int check(int i) {
    struct S { char before[4], member[8], after[4]; } s;
    char nested[2][8];
    return __builtin_dynamic_object_size(s.member+i,0) == 12-i &&
           __builtin_dynamic_object_size(nested[0]+i,0) == 16-i &&
           bounded(__builtin_dynamic_object_size(s.member+i,1),8-i) &&
           __builtin_dynamic_object_size(s.member+i,3) <= 8-i &&
           bounded(maximum(s.member+i),8-i) && minimum(s.member+i) <= 8-i &&
           bounded(__builtin_dynamic_object_size(nested[0]+i,1),8-i) &&
           __builtin_dynamic_object_size(nested[0]+i,3) <= 8-i &&
           bounded(maximum(nested[0]+i),8-i) && minimum(nested[0]+i) <= 8-i;
}
int main(void) { return check(2); }
",
        1,
    );
}

#[test]
fn nested_array_subobjects_and_incoming_mode_compatibility_are_retained() {
    let _text = returns(
        r"
int whole(char *const p __attribute__((pass_object_size(0)))) { return __builtin_object_size(p,0)==8 && __builtin_object_size(p,1)==8 && __builtin_object_size(p+2,0)==(unsigned long)-1; }
int least(char *const p __attribute__((pass_object_size(3)))) { return __builtin_object_size(p,2)==8 && __builtin_object_size(p,3)==8; }
int dynamic(char *const p __attribute__((pass_dynamic_object_size(1)))) { return __builtin_object_size(p,1)==8; }
int main(void) { char a[2][8],b[8]; return whole(b) && least(b) && dynamic(b) && __builtin_object_size(a[0]+2,0)==14 && __builtin_object_size(a[0]+2,1)==6 && __builtin_object_size(a[0]+2,2)==14 && __builtin_object_size(a[0]+2,3)==6; }
",
        1,
    );
}

#[test]
fn overload_promotions_qualification_scope_and_contextual_addresses_match_clang() {
    let _text = returns(
        r"
int f(int n) __attribute__((overloadable)) { return 1; }
int f(long n) __attribute__((overloadable)) { return 2; }
int p(const char *p) __attribute__((overloadable)) { return 3; }
int p(const void *p) __attribute__((overloadable)) { return 4; }
int q(char *p) __attribute__((overloadable)) { return 1; }
int q(const char *p) __attribute__((overloadable)) { return 2; }
int scoped(int n) __attribute__((overloadable)) { return 5; }
int scoped(double n) __attribute__((overloadable)) { return 6; }
int main(void) { char c=1,data[4]; int (*chosen)(int)=&f; int scoped(double) __attribute__((overloadable)); return f(c)+p(data)+q(data)+chosen(2)+scoped(3); }
",
        12,
    );
}

#[test]
fn enable_mangling_retains_calls_conditionals_casts_and_substitutions() {
    let text = lowered(
        r#"
int folded(int n) __attribute__((overloadable,enable_if(__builtin_constant_p(n),"constant"))) { return n; }
int conditional(int n) __attribute__((overloadable,enable_if(n?1:0,"condition"))) { return n; }
int casted(int n) __attribute__((overloadable,enable_if((unsigned)n>0,"condition"))) { return n; }
int repeated(char *a,char *b) __attribute__((overloadable)) { return 0; }
int main(void) { return folded(2)+conditional(2)+casted(2); }
"#,
    );
    for name in [
        "_Z6foldedUa9enable_ifIXclL_Z20__builtin_constant_pEfL0p_EEEi",
        "_Z11conditionalUa9enable_ifIXqufL0p_Li1ELi0EEEi",
        "_Z6castedUa9enable_ifIXgtcvjfL0p_Li0EEEi",
        "_Z8repeatedPcS_",
    ] {
        assert!(text.contains(name), "{name}\n{text}");
    }
}

#[test]
fn non_call_function_designators_retain_definition_references() {
    let _text = returns(
        r"static int target(void) __attribute__((overloadable)) { return 7; } int main(void) { return !target; }",
        0,
    );
}

#[test]
fn foreign_attribute_namespaces_leave_ordinary_calls_and_symbols() {
    let text = returns(
        r#"
[[gnu::overloadable]] int plain(int n) { return n; }
int fixed(char *const p [[gnu::pass_object_size(missing_name)]]) { return 2; }
int dynamic(char *const p [[gnu::pass_dynamic_object_size(1)]]) { return 3; }
[[gnu::unavailable("blocked"), gnu::diagnose_if(missing_name,"blocked","error"),
  gnu::enable_if(missing_name,"blocked")]] int checked(int n) { return n; }
int main(void) { char data[9]; return plain(1)+fixed(data)+dynamic(data)+checked(4); }
"#,
        10,
    );
    for signature in [
        "function @plain(i32 noundef)",
        "function @fixed(ptr noundef)",
        "function @dynamic(ptr noundef)",
        "function @checked(i32 noundef)",
    ] {
        assert!(text.contains(signature), "{signature}\n{text}");
    }
}

#[test]
fn recognized_clang_namespace_keeps_mangling_and_hidden_sizes() {
    let text = returns(
        r"
[[clang::overloadable]] int scoped(char *const p [[clang::pass_object_size(1)]]) {
    return __builtin_object_size(p,1);
}
int main(void) { char data[9]; return scoped(data); }
",
        9,
    );
    assert!(
        text.contains("function @_Z6scopedPcU17pass_object_size1(ptr noundef, i64 noundef)"),
        "{text}"
    );
}

#[test]
fn cleanup_calls_transport_fixed_object_sizes() {
    let _text = returns(
        r"
unsigned long observed;
void scalar(int *const p __attribute__((pass_object_size(0)))) {
    observed += __builtin_object_size(p,0);
}
void array(void *const p __attribute__((pass_object_size(3)))) {
    observed += __builtin_object_size(p,3);
}
int main(void) {
    { int n __attribute__((cleanup(scalar))) = 1;
      char data[7] __attribute__((cleanup(array))); }
    return observed;
}
",
        11,
    );
}

#[test]
fn cleanup_calls_transport_saved_dynamic_and_unknown_static_vla_sizes() {
    let _text = returns(
        r"
unsigned long observed;
void dynamic(void *const p __attribute__((pass_dynamic_object_size(1)))) {
    observed += __builtin_dynamic_object_size(p,1);
}
void upper(void *const p __attribute__((pass_object_size(1)))) {
    observed += __builtin_object_size(p,1) == (unsigned long)-1;
}
void lower(void *const p __attribute__((pass_object_size(3)))) {
    observed += __builtin_object_size(p,3) == 0;
}
void test(int n) {
    char data[n][3] __attribute__((cleanup(dynamic)));
    char other[n] __attribute__((cleanup(upper)));
    char third[n] __attribute__((cleanup(lower)));
    n = 77;
}
int main(void) { test(5); return observed; }
",
        17,
    );
}

#[test]
fn private_inline_cleanup_calls_keep_the_hidden_size_argument() {
    let text = returns(
        r"
unsigned long observed;
extern inline __attribute__((gnu_inline,always_inline))
void clean(void *const p __attribute__((pass_object_size(1)))) {
    observed = __builtin_object_size(p,1);
}
int main(void) { { char data[13] __attribute__((cleanup(clean))); } return observed; }
",
        13,
    );
    assert!(text.contains("@clean.always_inline"), "{text}");
}

#[test]
fn cleanup_enable_if_is_exempt_while_diagnose_if_warnings_allow_execution() {
    let _text = returns(
        r#"
int observed;
void clean(int *p) __attribute__((enable_if(0,"disabled"),diagnose_if(p!=0,"cleanup","warning"))) {
    observed += *p;
}
int main(void) { { int n __attribute__((cleanup(clean))) = 7; } return observed; }
"#,
        7,
    );
}

#[test]
fn cleanup_hidden_sizes_follow_aggregate_result_storage() {
    let _text = returns(
        r"
struct R { int value[4]; };
unsigned long observed;
struct R clean(void *const p __attribute__((pass_object_size(1)))) {
    observed = __builtin_object_size(p,1);
    struct R result = {0}; return result;
}
int main(void) { { char data[9] __attribute__((cleanup(clean))); } return observed; }
",
        9,
    );
}

#[test]
fn overload_floating_promotions_and_contextual_function_arguments_execute() {
    let text = returns(
        r"
int widen(double value) __attribute__((overloadable)) { return value == 1.5; }
int widen(int value) __attribute__((overloadable)) { return 20; }
int transform(int value) __attribute__((overloadable)) { return value + 3; }
int transform(double value) __attribute__((overloadable)) { return 30; }
int apply(int (*function)(int)) __attribute__((overloadable)) { return function(4); }
int apply(double value) __attribute__((overloadable)) { return 40; }
int plain(int (*function)(int)) { return function(5); }
int main(void) { return widen(1.5f) + apply(transform) + plain(&transform); }
",
        16,
    );
    assert!(
        text.contains("@_Z5widend") && text.contains("@_Z5applyPFiiE"),
        "{text}"
    );
}

#[test]
fn overload_mangling_preserves_complex_substitutions_and_c_function_types() {
    let text = lowered(
        r"
int complex(float _Complex a, float _Complex b) __attribute__((overloadable)) { return 0; }
int functions(int (*a)(), int (*b)(void)) __attribute__((overloadable)) { return 0; }
typedef struct { int value; } Named;
int record(Named *p) __attribute__((overloadable)) { return p->value; }
int anonymous(struct { int value; } *p) __attribute__((overloadable)) { return p->value; }
int main(void) { return 0; }
",
    );
    for name in [
        "_Z7complexCfS_",
        "_Z9functionsPFiEPFivE",
        "_Z6recordP5Named",
        "_Z9anonymousP3$_0",
    ] {
        assert!(text.contains(name), "{name}\n{text}");
    }
}

#[test]
fn overload_local_tags_distinguish_repeated_names_and_internal_owners() {
    let text = lowered(
        r"
void outer(void) {
 { struct S { int x; }; extern int f(struct S *) __attribute__((overloadable)); f(0); }
 { struct S { double x; }; extern int f(struct S *) __attribute__((overloadable)); f(0); }
}
static void inner(void) {
 struct S { int x; }; extern int f(struct S *) __attribute__((overloadable)); f(0);
}
void *keep = (void *)inner;
",
    );
    for name in ["_Z1fPZ5outerE1S", "_Z1fPZ5outerE1S_0", "_Z1fPZL5innerE1S"] {
        assert!(text.contains(name), "{name}\n{text}");
    }
}

#[test]
fn overload_anonymous_tags_keep_identity_through_later_typeof_typedefs() {
    for source in [
        "struct {int x;} obj; typedef __typeof__(obj) Named; int f(Named*p) \
         __attribute__((overloadable)){return p->x;}",
        "typedef struct{int x;} *P; typedef __typeof__(*((P)0)) Named; int f(Named*p) \
         __attribute__((overloadable)){return p->x;}",
    ] {
        let text = lowered(source);
        assert!(text.contains("_Z1fP3$_0"), "{source}\n{text}");
        assert!(!text.contains("_Z1fP5Named"), "{source}\n{text}");
    }
}
