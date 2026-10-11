//! Golden IR for each construct family. Each snapshot under
//! `tests/fixtures/lowering/` holds the C source as `;` comments and the IR
//! lowering produces for it; `BLESS=1` rewrites them.

use super::*;
use crate::test_support::assert_snapshot;

/// Lowers `source` and compares the source and its IR with the snapshot
/// `name`.
#[track_caller]
fn golden(name: &str, source: &str) {
    let ir = lowered(source);
    let mut text = String::new();
    for line in source.trim().lines() {
        text.push_str(if line.is_empty() { ";" } else { "; " });
        text.push_str(line);
        text.push('\n');
    }
    text.push('\n');
    text.push_str(&ir);
    assert_snapshot(&format!("tests/fixtures/lowering/{name}.ir"), &text);
}

#[test]
fn integer_arithmetic_marks_signed_overflow() {
    golden(
        "arithmetic",
        "int add(int a, int b) { return a + b * 3 - (a << 2); }
unsigned wrap(unsigned a, unsigned b) { return a * b + (a >> 1) - b / a % 7u; }
int divide(int a, int b) { return a / b + a % b + (a >> b) + (a & b | a ^ ~b) + -a; }
long widen(int i, unsigned u, char c, short s) { return i + u + c + s + (long)u; }
char narrow(long l) { return (char)l + (unsigned char)l; }",
    );
}

#[test]
fn comparisons_choose_signedness() {
    golden(
        "comparisons",
        "int sless(int a, int b) { return a < b; }
int uless(unsigned a, unsigned b) { return a <= b; }
int mixed(int a, unsigned b) { return a > b; }
int pointers(int *p, int *q) { return (p < q) + (p == q) + (p != 0) + !p; }
int chars(char c, unsigned char u) { return c >= u; }",
    );
}

#[test]
fn bool_conversions_compare_with_zero() {
    golden(
        "bool",
        "_Bool from_int(int x) { return x; }
_Bool from_pointer(int *p) { return p; }
int to_int(_Bool b) { return b + 1; }
int step(_Bool b) { b++; return b; }",
    );
}

#[test]
fn pointer_arithmetic_scales_and_differences_divide() {
    golden(
        "pointers",
        "long difference(int *p, int *q) { return q - p; }
int *advance(int *p, int i) { return p + i; }
int *back(int *p, unsigned i) { return p - i; }
char *bytes(char *p, long i) { return 2 + p + i; }
int deref(int **pp) { return **pp + (*pp)[3]; }",
    );
}

#[test]
fn logical_operators_and_conditionals_are_control_flow() {
    golden(
        "logical",
        "int f(int);
int both(int a, int b) { return a && f(b); }
int either(int a, int b) { return a || f(b); }
int pick(int c, int a, long b) { return c ? a : b; }
int not(int a) { return !a; }
int branch(int a, int b) { if (a > 0 && (b < 0 || !a)) return 1; return 0; }",
    );
}

#[test]
fn assignment_and_compound_assignment_convert_back() {
    golden(
        "assignment",
        "int f(int a, char c, unsigned u, int *p) {
  int x = a;
  x += u;
  c *= 3;
  c <<= a;
  p += 2;
  p -= a;
  *p = x = c;
  return x;
}",
    );
}

#[test]
fn increments_follow_the_operand_type() {
    golden(
        "increments",
        "int f(int i, char c, int *p, unsigned u) {
  int old = i++;
  ++c;
  p--;
  --u;
  return old + i + c + *p++ + (int)u;
}",
    );
}

#[test]
fn address_taken_locals_and_aggregates_live_in_slots() {
    golden(
        "locals",
        "void g(int *);
int f(int a) {
  int x = a;
  int y = a + 1;
  int arr[4];
  g(&x);
  arr[0] = y;
  arr[a] = x;
  return arr[0] + x + y;
}
int v(void) { volatile int n = 3; n = n + 1; return n; }",
    );
}

#[test]
fn members_use_field_offsets() {
    golden(
        "members",
        "struct inner { char c; long l; };
struct outer { int a; struct inner in; int arr[3]; };
long f(struct outer *o, int i) {
  struct outer local;
  local.in.l = o->in.c;
  local.arr[i] = o->arr[1];
  local = *o;
  return local.in.l + o->a;
}",
    );
}

#[test]
fn calls_convert_arguments() {
    golden(
        "calls",
        "int printf(const char *, ...);
int old();
long proto(long, char);
int (*pick(void))(int);
int f(char c, short s, int (*fp)(int)) {
  printf(\"%d %d\\n\", c, s);
  old(c, s);
  proto(c, 300);
  pick()(c);
  return fp(s) + (*fp)(1);
}",
    );
}

#[test]
fn loops_use_headers_with_block_parameters() {
    golden(
        "loops",
        "int sum(int n) {
  int s = 0;
  for (int i = 0; i < n; i++) {
    if (i == 3) continue;
    if (i > 100) break;
    s += i;
  }
  int j = 0;
  while (j < n) j += 2;
  do { s--; } while (s > 50);
  for (;;) { if (s) break; s = 1; }
  return s + j;
}",
    );
}

#[test]
fn switch_dispatches_on_the_promoted_value() {
    golden(
        "switch",
        "int f(char c, int x) {
  int r = 0;
  switch (c) {
  case 'a': r = 1;
  case 'b': r += 2; break;
  case -1: return 7;
  default: r = x;
  }
  switch (x) { case 1: r++; }
  return r;
}",
    );
}

#[test]
fn goto_and_labels_seal_at_the_end() {
    golden(
        "goto",
        "int f(int n) {
  int i = 0;
  int s = 0;
again:
  if (i >= n) goto done;
  s += i;
  i++;
  goto again;
done:
  return s;
}",
    );
}

#[test]
fn file_scope_objects_and_string_literals_are_globals() {
    golden(
        "globals",
        "int counter;
static int limit = 10;
const char message[] = \"hi\";
char buffer[4] = \"abc\";
const char *greeting = \"hello\";
int table[4] = { 1, 2, [3] = 4 - 0 };
int *pointer = &table[2];
struct pair { char a; int b; } pair = { 'x', -1 };
int f(void) { static int calls = 5; calls++; counter += limit; return calls + message[0] + \
         greeting[1]; }",
    );
}

#[test]
fn compound_literals_are_unnamed_locals() {
    golden(
        "compound_literals",
        "struct point { int x, y; };
int f(int a) {
  int *p = (int[]){ 1, a, 3 };
  struct point q = (struct point){ .y = a };
  return p[1] + q.y + ((struct point){ a, 2 }).y + (int){ 4 };
}",
    );
}

#[test]
fn function_names_are_static_arrays() {
    golden(
        "function_name",
        "const char *name(void) { return __func__; }
int first(void) { return __func__[0]; }",
    );
}

#[test]
fn implicit_declarations_call_unprototyped_functions() {
    let configuration = crate::configuration::CompilerConfiguration::new(
        crate::configuration::CStandard::C89,
        crate::configuration::ExtensionPolicy::Allow,
    );
    let ir = lower_with(
        "int main() { putchar(72); return exit2(1, 2L); }",
        configuration,
    )
    .expect("C89 declares called names implicitly");
    assert!(
        ir.contains(
            "function @putchar() -> i32 external
"
        ),
        "{ir}"
    );
    assert!(ir.contains("call_indirect"), "{ir}");
}

#[test]
fn main_returns_zero_at_its_closing_brace() {
    golden(
        "main",
        "int main(void) { int x = 1; x++; }\nvoid nothing(void) {}",
    );
}

#[test]
fn local_initializers_zero_fill_and_store() {
    golden(
        "initializers",
        "struct point { int x, y; };
int f(int a) {
  int arr[5] = { 1, a, 3 };
  struct point p = { a };
  struct point ps[2] = { 1, 2, { a, 4 } };
  char s[8] = \"text\";
  int scalar = { a };
  int d[5] = { [3] = a, 9, [0] = 1 };
  struct point q = { .y = a };
  struct { struct point in; int z; } n = { .in.y = 2, 3, .z = a };
  return arr[1] + p.x + ps[1].x + s[0] + scalar + d[4] + q.y + n.z;
}",
    );
}

#[test]
fn the_target_sets_type_widths_and_the_data_layout() {
    let source = "long f(long a) { return a + 1; }";
    let linux = lower_for(source, Target::LinuxGnu).unwrap();
    let windows = lower_for(source, Target::WindowsMsvc).unwrap();
    assert!(
        linux.starts_with("target triple = \"x86_64-unknown-linux-gnu\""),
        "{linux}"
    );
    assert!(linux.contains("function @f(i64) -> i64"), "{linux}");
    assert!(
        windows.starts_with("target triple = \"x86_64-pc-windows-msvc\""),
        "{windows}"
    );
    assert!(windows.contains("e-m:w-"), "{windows}");
    assert!(windows.contains("function @f(i32) -> i32"), "{windows}");
}
