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
