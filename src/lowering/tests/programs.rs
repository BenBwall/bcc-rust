//! Small whole programs whose lowering must succeed and verify. Each
//! records the status its `main` returns; these tests check that every
//! program lowers to a module the verifier accepts, and the end-to-end tests
//! in `pipeline/tests/middle_end.rs` run them in the interpreter and through
//! LLVM.

use super::*;

/// A C program and the value its `main` returns.
pub(crate) struct Program {
    pub(crate) name:     &'static str,
    pub(crate) source:   &'static str,
    pub(crate) expected: i32,
}

pub(crate) const PROGRAMS: &[Program] = &[
    Program {
        name:     "vla_extents_and_typedefs",
        source:   "static int f(int n, int (*p)[n]) { return p[1][2] + (int)sizeof *p; }
int main(void) { int n=3, m=2; typedef int Row[n++]; Row a[m++]; a[1][2]=7;
 if(n!=4 || m!=3 || sizeof a!=24 || sizeof a[0]!=12) return 1;
 if(f(3,a)!=19 || &a[1]-&a[0]!=1) return 2;
 int (*p)[3]=a; p++; if((*p)[2]!=7) return 3;
 int size=sizeof(int[n++]); return size!=16 || n!=5; }",
        expected: 0,
    },
    Program {
        name:     "vla_block_lifetime",
        source:   "int main(void) { int n=8192, total=0;
 for(int i=0;i<1000;i++) { int a[n]; a[0]=i; a[n-1]=i+1; total+=a[n-1]-a[0]; if(i%2) continue; { \
                   int b[n]; b[0]=1; if(i==998) break; } }
 { int a[n]; a[0]=2; total+=a[0]; goto done; }
 done: return total!=1001; }",
        expected: 0,
    },
    Program {
        name:     "vla_parameter_bounds",
        source:   "static int f(int n, int a[n++]) { return n; }
static int g(int n, int (*a)[n++]) { return n + (int)sizeof *a; }
static int old(n,a) int n; int a[n++]; { return n; }
int main(void) { int a[3][3]; return f(3,a[0])!=4 || g(3,a)!=16 || old(3,a[0])!=4; }",
        expected: 0,
    },
    Program {
        name:     "sum_to_ten",
        source:   "int main(void) { int s = 0; for (int i = 1; i <= 10; i++) s += i; return s; }",
        expected: 55,
    },
    Program {
        name:     "recursive_fibonacci",
        source:   "int fib(int n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }
int main(void) { return fib(20) - 6765 + 1; }",
        expected: 1,
    },
    Program {
        name:     "euclid_gcd",
        source:   "unsigned gcd(unsigned a, unsigned b) { while (b) { unsigned t = a % b; a = b; \
                   b = t; } return a; }
int main(void) { return gcd(1071, 462); }",
        expected: 21,
    },
    Program {
        name:     "bubble_sort_local_array",
        source:   "int main(void) {
  int a[5] = { 5, 3, 9, 1, 7 };
  for (int i = 0; i < 5; i++)
    for (int j = 0; j + 1 < 5 - i; j++)
      if (a[j] > a[j + 1]) { int t = a[j]; a[j] = a[j + 1]; a[j + 1] = t; }
  return a[0] + a[4] * 10;
}",
        expected: 91,
    },
    Program {
        name:     "struct_field_updates",
        source:   "struct point { int x, y; };
static void scale(struct point *p, int k) { p->x *= k; p->y *= k; }
int main(void) {
  struct point p;
  p.x = 3;
  p.y = 4;
  p.x += p.y;
  scale(&p, 2);
  return p.x + p.y;
}",
        expected: 22,
    },
    Program {
        name:     "pointer_walks_a_string",
        source:   "static int length(const char *s) { const char *p = s; while (*p) p++; return p \
                   - s; }
int main(void) { return length(\"hello, world\"); }",
        expected: 12,
    },
    Program {
        name:     "nested_loops_break_continue",
        source:   "int main(void) {
  int count = 0;
  for (int i = 0; i < 5; i++) {
    for (int j = 0; j < 5; j++) {
      if (j == i) continue;
      if (j > 3) break;
      count += j;
    }
  }
  return count;
}",
        expected: 24,
    },
    Program {
        name:     "short_circuit_side_effects",
        source:   "static int calls;
static int bump(int v) { calls++; return v; }
int main(void) {
  int r = 0;
  if (bump(0) && bump(1)) r += 1;
  if (bump(1) || bump(1)) r += 2;
  if (bump(1) && bump(0)) r += 4;
  if (bump(0) || bump(0)) r += 8;
  int v = bump(0) || (bump(1) && bump(2));
  return r * 10 + calls + v * 100;
}",
        expected: 129,
    },
    Program {
        name:     "switch_fallthrough",
        source:   "static int classify(int x) {
  int r = 0;
  switch (x) {
  case 1: r += 1;
  case 2: r += 2; break;
  case 3: r += 3;
  default: r += 10;
  }
  return r;
}
int main(void) { return classify(1) + classify(2) + classify(3) + classify(4); }",
        expected: 28,
    },
    Program {
        name:     "goto_loop",
        source:   "int main(void) {
  int i = 1, s = 0;
top:
  s += i;
  if (++i <= 5) goto top;
  return s;
}",
        expected: 15,
    },
    Program {
        name:     "recursive_factorial",
        source:   "static long factorial(long n) { if (n <= 1) return 1; return n * factorial(n - \
                   1); }
int main(void) { return factorial(5); }",
        expected: 120,
    },
    Program {
        name:     "globals_and_static_locals",
        source:   "int counter;
int next(void) { static int n = 10; return n++ + counter++; }
int main(void) { int a = next(); int b = next(); return a + b + next(); }",
        expected: 36,
    },
    Program {
        name:     "pointer_difference",
        source:   "int main(void) {
  int a[10];
  int *p = a + 7;
  int *q = &a[2];
  *(p - 1) = 4;
  return (p - q) * 10 + a[6];
}",
        expected: 54,
    },
    Program {
        name:     "unsigned_wraparound",
        source:   "int main(void) { unsigned u = 0; u--; unsigned char c = 255; c++; return (u > \
                   100u ? 40 : 7) + c + (u == 4294967295u); }",
        expected: 41,
    },
    Program {
        name:     "character_array_update",
        source:   "int main(void) { char s[] = \"abc\"; s[1] += 1; return s[1] - 'a' + sizeof s; }",
        expected: 6,
    },
    Program {
        name:     "collatz_do_while",
        source:   "int main(void) {
  int n = 6, steps = 0;
  do {
    n = n % 2 ? 3 * n + 1 : n / 2;
    steps++;
  } while (n != 1);
  return steps;
}",
        expected: 8,
    },
    Program {
        name:     "array_of_structs",
        source:   "struct item { int w, v; };
int main(void) {
  struct item items[3] = { { 1, 2 }, { 3, 4 }, 5, 6 };
  int total = 0;
  for (struct item *it = items; it < items + 3; ++it) total += it->w * it->v;
  return total;
}",
        expected: 44,
    },
    Program {
        name:     "function_pointers",
        source:   "static int add(int a, int b) { return a + b; }
static int mul(int a, int b) { return a * b; }
int main(void) {
  int (*ops[2])(int, int) = { add, mul };
  int (*pick)(int, int) = &mul;
  return ops[0](2, 3) + ops[1](4, 5) + (*pick)(1, 1);
}",
        expected: 26,
    },
    Program {
        name:     "printf_hello",
        source:   "int printf(const char *, ...);
int main(void) { printf(\"%s %d\\n\", \"answer\", 42); return 0; }",
        expected: 0,
    },
    Program {
        name:     "printf_long",
        source:   "int printf(const char *, ...);
int main(void) { long l = -5; printf(\"%ld %lu %d\\n\", l, 3000000000ul, (int)sizeof l); return 0; \
                   }",
        expected: 0,
    },
    Program {
        name:     "two_dimensional_array",
        source:   "int main(void) {
  int m[3][3];
  for (int i = 0; i < 3; i++)
    for (int j = 0; j < 3; j++)
      m[i][j] = i * 3 + j;
  return m[2][1] + m[1][2];
}",
        expected: 12,
    },
    Program {
        name:     "bit_operations",
        source:   "int main(void) { unsigned x = 0xF0; int y = -16; return ((x >> 4) | (x & 3)) ^ \
                   5 ^ (y >> 2 == -4) ^ (1 << 4); }",
        expected: 27,
    },
    Program {
        name:     "linked_list_on_the_stack",
        source:   "struct node { int value; struct node *next; };
int main(void) {
  struct node c = { 3, 0 }, b = { 2, &c }, a = { 1, &b };
  int sum = 0;
  for (struct node *n = &a; n; n = n->next) sum = sum * 10 + n->value;
  return sum % 256;
}",
        expected: 123,
    },
    Program {
        name:     "irreducible_goto_cycle",
        source:   "static int f(int c) {
  int n = 0;
  if (c) goto l2;
l1:
  n += 1;
  if (n > 10) return n;
l2:
  n += 2;
  goto l1;
}
int main(void) { return f(0) + f(1); }",
        expected: 25,
    },
    Program {
        name:     "duffs_device",
        source:   "int main(void) {
  int n = 7, count = 0;
  int k = (n + 3) / 4;
  switch (n % 4) {
  case 0: do { count++;
  case 3:      count++;
  case 2:      count++;
  case 1:      count++;
          } while (--k > 0);
  }
  return count;
}",
        expected: 7,
    },
    Program {
        name:     "designators_and_compound_literals",
        source:   "struct p { int x, y; };
int main(void) {
  int a[5] = { [2] = 5, [4] = 1 };
  struct p q = { .y = 3 };
  int *r = (int[]){ 4, 6 };
  return a[2] + a[4] + a[0] + q.y + q.x + r[1];
}",
        expected: 15,
    },
    Program {
        // GNU casts of a structure or union to its own type give its value.
        name:     "same_type_aggregate_casts",
        source:   "struct S { int a, b; } s = {1, 2};
union U { int i; float f; } u = {5};
typedef struct S T;
static struct S get(void) { return (struct S)s; }
int main(void) {
  struct S t = (struct S)s;
  const struct S cs = {3, 4};
  T v = (T)cs;
  union U w = (union U)u;
  return (t.b == 2) + (v.a == 3) * 2 + (((struct S)get()).a == 1) * 4 + (w.i == 5) * 8
       + (sizeof((struct S)s) == 8) * 16;
}",
        expected: 31,
    },
    Program {
        // GNU and MSVC `#pragma push_macro` / `pop_macro`.
        name:     "push_and_pop_macro",
        source:   "#define VALUE 1
#pragma push_macro(\"VALUE\")
#undef VALUE
#define VALUE 20
static int inner(void) { return VALUE; }
#pragma pop_macro(\"VALUE\")
int main(void) { return inner() + VALUE; }",
        expected: 21,
    },
    Program {
        // GNU `\\e` (ESC) and a conditional with one void alternative.
        name:     "gnu_escape_and_one_void_conditional",
        source:   "static int calls;
static void note(void) { calls++; }
int main(void) {
  int i = 1;
  i ? note() : (void)0;
  i ? (note(), 5) : (void)0;
  !i ? 7 : note();
  i ?: note();
  0 ?: note();
  return calls * 10 + \"\\e\"[0] + '\\E' - 28;
}",
        // Not 67: WSL on the Windows runner reports a Linux exit status of
        // 67 as its own failure, WSL_E_USER_NOT_FOUND.
        expected: 66,
    },
    Program {
        // GNU constant compound literals initializing static structures,
        // members, array elements and a block-scope static.
        name:     "constant_compound_literal_statics",
        source:   "struct s { int a, b; const char *n; };
static int x = 4;
struct s whole = (struct s){1, 2, \"x\"};
struct w { struct s inner; int *p; } member = { ((struct s){.b = 3}), &x };
struct s array[2] = { (struct s){5}, [1] = (struct s){.a = 6, .n = \"yz\"} };
static int local(void) { static struct s kept = (struct s){7, 8}; return kept.a + kept.b; }
int main(void) {
  return whole.a + whole.b + whole.n[0] - 'x' + member.inner.b + *member.p + array[0].a
       + array[1].a + array[1].n[1] - 'z' + local();
}",
        expected: 36,
    },
    Program {
        // GNU scalar compound literals of a static object's own type.
        name:     "scalar_compound_literal_statics",
        source:   "static int g = 9;
static int i = (int){3};
static double d = ((double){2.5});
static char ch = (char){300};
static int braces = (int){{6}};
static int *p = (int *){&g};
static _Bool b = (_Bool){2};
static int local(void) { static long w = (long){7}; return (int)w; }
int main(void) { return i + (int)(d * 2) + ch + braces + *p + b + local(); }",
        expected: 75,
    },
    Program {
        // A scalar compound literal of a static bit-field's type converts to
        // that type and then truncates to the field's width.
        name:     "scalar_compound_literal_bit_fields",
        source:   "static struct S { unsigned x:4; int y:3; _Bool b:1; unsigned z:5; } s =
  {(unsigned){3}, (int){-2}, (_Bool){5}, ((unsigned){0x2b})};
static struct S t = {.z = (unsigned){{7}}};
int main(void) {
  return s.x + (s.y == -2) * 4 + s.b * 8 + (s.z == 11) * 16 + (t.z == 7) * 32;
}",
        expected: 63,
    },
    Program {
        name:     "old_style_definition",
        source:   "int scale(x, k) char x; int k; { return x * k; }
int main(void) { return scale(7, 6); }",
        expected: 42,
    },
];

/// Prints each program's IR, for inspection with `--nocapture`.
#[test]
#[ignore = "prints the IR of every program"]
fn print_programs() {
    for program in PROGRAMS {
        println!(
            "; {}
{}",
            program.name,
            functions(&lowered(program.source))
        );
    }
}

#[test]
fn every_program_lowers_to_verified_ir() {
    for program in PROGRAMS {
        let ir = lower(program.source).unwrap_or_else(|errors| {
            panic!(
                "{} failed to lower: {errors:?}\n{}",
                program.name, program.source
            )
        });
        assert!(
            ir.contains("function @main() -> i32 external {"),
            "{} defines main:\n{ir}",
            program.name
        );
    }
}

#[test]
fn programs_have_distinct_names_and_portable_statuses() {
    let mut names: Vec<_> = PROGRAMS.iter().map(|program| program.name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), PROGRAMS.len());
    assert!(PROGRAMS.len() >= 15);
    assert!(
        PROGRAMS
            .iter()
            .all(|program| (0..256).contains(&program.expected))
    );
}

#[test]
fn programs_lower_for_every_target() {
    for target in [
        Target::LinuxGnu,
        Target::LinuxMusl,
        Target::WindowsGnu,
        Target::WindowsMsvc,
    ] {
        for program in PROGRAMS {
            if let Err(errors) = lower_for(program.source, target) {
                panic!("{} failed for {target:?}: {errors:?}", program.name);
            }
        }
    }
}
