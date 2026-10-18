//! `long double`, `__float128` and conversions between `__int128` and
//! floats cross between the native back end and Clang in both directions:
//! arguments in memory, by reference and in SSE registers, results in
//! `%st0`, through hidden pointers and in `%xmm0`, and the runtime routines
//! both sides call.
use super::*;

const BCC: &str = r"
typedef long double ld;
typedef __int128 i128;
typedef unsigned __int128 u128;
ld b_ld_mix(ld a, int i, ld b, double d, ld c, float f, ld e, long long k, ld g) {
  return a * i + b - d * c + f / e + k - g;
}
ld b_ld_neg(ld x) { return -x; }
long long b_ld_to_int(ld x) { return (long long)x; }
unsigned long long b_ld_to_uint(ld x) { return (unsigned long long)x; }
ld b_ld_from_uint(unsigned long long x) { return x; }
i128 b_ld_to_i128(ld x) { return (i128)x; }
ld b_ld_from_i128(i128 x) { return x; }
double b_ld_to_double(ld x) { return (double)x; }
ld b_ld_from_float(float x) { return x; }
int b_ld_less(ld a, ld b) { return a < b; }
int b_ld_unordered(ld a, ld b) { return !(a <= b) && !(a > b); }
double b_i128_to_double(i128 x) { return x; }
float b_u128_to_float(u128 x) { return x; }
i128 b_double_to_i128(double x) { return x; }
u128 b_float_to_u128(float x) { return x; }

ld c_ld_mix(ld a, int i, ld b, double d, ld c, float f, ld e, long long k, ld g);
ld c_ld_scale(ld x, int k);
i128 c_to_i128(double x);
double c_from_i128(i128 x);
#ifndef _WIN32
ld b_ld_sum(int n, ...) {
  __builtin_va_list ap;
  __builtin_va_start(ap, n);
  ld sum = 0;
  for (int i = 0; i < n; i++) sum += __builtin_va_arg(ap, ld);
  __builtin_va_end(ap);
  return sum;
}
ld c_ld_sum(int n, ...);
#endif
int bcc_wide(void) {
  ld expected = 1.5L * 2 + 3.25L - 0.5 * 4.0L + 0.25f / 8.0L + 7 - 1.25L;
  if (c_ld_mix(1.5L, 2, 3.25L, 0.5, 4.0L, 0.25f, 8.0L, 7, 1.25L) != expected) return 20;
  ld (*scale)(ld, int) = c_ld_scale;
  if (scale(1.25L, 3) != 3.75L) return 21;
  if (c_to_i128(-1e30) != (i128)-1e30) return 22;
  if (c_from_i128((i128)1 << 100) != (double)((i128)1 << 100)) return 23;
#ifndef _WIN32
  if (c_ld_sum(3, 1.5L, 2.25L, 1e300L) != 1.5L + 2.25L + 1e300L) return 24;
#endif
  return 0;
}
";

const CLANG: &str = r"
typedef long double ld;
typedef __int128 i128;
typedef unsigned __int128 u128;
ld b_ld_mix(ld a, int i, ld b, double d, ld c, float f, ld e, long long k, ld g);
ld b_ld_neg(ld x);
long long b_ld_to_int(ld x);
unsigned long long b_ld_to_uint(ld x);
ld b_ld_from_uint(unsigned long long x);
i128 b_ld_to_i128(ld x);
ld b_ld_from_i128(i128 x);
double b_ld_to_double(ld x);
ld b_ld_from_float(float x);
int b_ld_less(ld a, ld b);
int b_ld_unordered(ld a, ld b);
double b_i128_to_double(i128 x);
float b_u128_to_float(u128 x);
i128 b_double_to_i128(double x);
u128 b_float_to_u128(float x);
int bcc_wide(void);
ld c_ld_mix(ld a, int i, ld b, double d, ld c, float f, ld e, long long k, ld g) {
  return a * i + b - d * c + f / e + k - g;
}
ld c_ld_scale(ld x, int k) { return x * k; }
i128 c_to_i128(double x) { return x; }
double c_from_i128(i128 x) { return x; }
#ifndef _WIN32
#include <stdarg.h>
ld b_ld_sum(int n, ...);
ld c_ld_sum(int n, ...) {
  va_list ap;
  va_start(ap, n);
  ld sum = 0;
  for (int i = 0; i < n; i++) sum += va_arg(ap, ld);
  va_end(ap);
  return sum;
}
#endif
int main(void) {
  volatile ld third = 1.0L / 3;
  ld expected = 1.5L * 2 + 3.25L - 0.5 * 4.0L + 0.25f / 8.0L + 7 - 1.25L;
  if (b_ld_mix(1.5L, 2, 3.25L, 0.5, 4.0L, 0.25f, 8.0L, 7, 1.25L) != expected) return 1;
  if (b_ld_neg(third) != -third) return 2;
  if (b_ld_to_int(-third * 3e18L) != (long long)(-third * 3e18L)) return 3;
  if (b_ld_to_uint(third * 5e19L) != (unsigned long long)(third * 5e19L)) return 4;
  if (b_ld_from_uint(18446744073709551615ULL) != (ld)18446744073709551615ULL) return 5;
  if (b_ld_to_i128(third * 1e35L) != (i128)(third * 1e35L)) return 6;
  if (b_ld_from_i128(-((i128)1 << 90) - 12345) != (ld)(-((i128)1 << 90) - 12345)) return 7;
  if (b_ld_to_double(third) != (double)third) return 8;
  if (b_ld_from_float(0.1f) != (ld)0.1f) return 9;
  if (!b_ld_less(third, 0.5L) || b_ld_less(0.5L, third)) return 10;
  if (!b_ld_unordered(third, third / 0 * 0) || b_ld_unordered(third, 1)) return 11;
  i128 wide = ((i128)0x123456789abcdefLL << 50) + 0x1234567;
  if (b_i128_to_double(-wide) != (double)-wide) return 12;
  if (b_u128_to_float((u128)wide << 10) != (float)((u128)wide << 10)) return 13;
  if (b_double_to_i128(-1.5e35) != (i128)-1.5e35) return 14;
  if (b_float_to_u128(3e38f) != (u128)3e38f) return 15;
#ifndef _WIN32
  if (b_ld_sum(4, 1.5L, third, 1e300L, -2.0L) != 1.5L + third + 1e300L - 2.0L) return 16;
#endif
  int status = bcc_wide();
  return status ? 100 + status : 0;
}
";

/// The C exchange, with the bcc side through the native back end and,
/// for comparison, through the LLVM back end. Win64's assembly is built
/// and assembled where it cannot run.
#[test]
fn long_double_and_int128_conversions_cross_natively() {
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        for optimized in [false, true] {
            let arena = Bump::new();
            let tu = Bump::new();
            let scratch = Bump::new();
            let mut module = lower_c(&tu, &arena, BCC, target);
            if optimized {
                _ = optimize(&mut module, &OptimizerOptions::default(), &scratch);
            }
            lower_abi(&mut module, target, &scratch).unwrap();
            assert_verified(&module, Profile::PostAbi, &module.to_string());
            let name = format!("wide-floats-{optimized}");
            assert_conforms(&name, target, &module, CLANG);
            assert_native_conforms(&format!("{name}-native"), target, &module, CLANG);
        }
    }
}

/// `__float128` functions written in IR, since C lowering refuses the
/// type's values: Clang calls them with more arguments than SSE registers,
/// and they call Clang's.
const QUAD_IR: &str = "\
function @c_qmul(f128, f128) -> f128 external
function @b_qadd(f128, f128) -> f128 external {
block0(v0: f128, v1: f128):
    v2 = fadd.f128 v0, v1
    return v2
}
function @b_qmany(f128, f128, f128, f128, f128, f128, f128, f128, f128, i32, f128) -> f128 \
                       external {
block0(v0: f128, v1: f128, v2: f128, v3: f128, v4: f128, v5: f128, v6: f128, v7: f128, v8: f128, \
                       v9: i32, v10: f128):
    v11 = fadd.f128 v0, v1
    v12 = fmul.f128 v11, v2
    v13 = fsub.f128 v12, v3
    v14 = fdiv.f128 v13, v4
    v15 = fadd.f128 v14, v5
    v16 = fadd.f128 v15, v6
    v17 = fsub.f128 v16, v7
    v18 = fmul.f128 v17, v8
    v19 = sitofp.f128 v9
    v20 = fadd.f128 v18, v19
    v21 = fsub.f128 v20, v10
    return v21
}
function @b_qless(f128, f128) -> i32 external {
block0(v0: f128, v1: f128):
    v2 = fcmp.f128 olt v0, v1
    v3 = zext.i32 v2
    return v3
}
function @b_qconv(f128) -> i64 external {
block0(v0: f128):
    v1 = fptosi.i64 v0
    return v1
}
function @b_qneg(f128) -> f128 external {
block0(v0: f128):
    v1 = fneg.f128 v0
    return v1
}
function @b_qcall() -> i32 external {
block0:
    v0 = fconst.f128 0x40008000000000000000000000000000
    v1 = fconst.f128 0x40004000000000000000000000000000
    v2 = call @c_qmul(v0, v1)
    v3 = fconst.f128 0x4001E000000000000000000000000000
    v4 = fcmp.f128 une v2, v3
    v5 = zext.i32 v4
    return v5
}
";

const QUAD_CLANG: &str = r"
typedef __float128 q;
q b_qadd(q, q);
q b_qmany(q, q, q, q, q, q, q, q, q, int, q);
int b_qless(q, q);
long long b_qconv(q);
q b_qneg(q);
int b_qcall(void);
q c_qmul(q a, q b) { return a * b; }
int main(void) {
  volatile q third = (q)1 / 3;
  if (b_qadd(third, (q)2) != third + 2) return 1;
  if (b_qmany(third, (q)1, (q)2, (q)3, (q)4, (q)5, (q)6, (q)7, (q)8, 9, third)
      != ((((third + 1) * 2 - 3) / 4 + 5 + 6 - 7) * 8 + 9 - third)) return 2;
  if (!b_qless(third, (q)0.5) || b_qless((q)0.5, third)) return 3;
  if (b_qconv(third * (q)3e18) != (long long)(third * (q)3e18)) return 4;
  if (b_qneg(third) != -third) return 5;
  return b_qcall() ? 6 : 0;
}
";

/// The `__float128` exchange on System V, the convention under which Clang
/// passes the type in SSE registers.
#[test]
fn quad_values_cross_natively() {
    for optimized in [false, true] {
        let arena = Bump::new();
        let mut module = parse_module(&arena, QUAD_IR).unwrap();
        if optimized {
            _ = optimize(&mut module, &OptimizerOptions::default(), &Bump::new());
        }
        lower_abi(&mut module, Target::LinuxGnu, &Bump::new()).unwrap();
        assert_verified(&module, Profile::PostAbi, &module.to_string());
        let name = format!("quad-{optimized}");
        assert_conforms(&name, Target::LinuxGnu, &module, QUAD_CLANG);
        assert_native_conforms(
            &format!("{name}-native"),
            Target::LinuxGnu,
            &module,
            QUAD_CLANG,
        );
    }
}
