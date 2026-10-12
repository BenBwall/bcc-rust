//! Execute wide floating varargs at both independently compiled boundaries.
use super::*;

#[test]
fn long_double_varargs_cross_both_directions() {
    floating_varargs(
        Target::WindowsMsvc,
        "long double",
        "f64",
        0x3FF1_8685_8483_8281,
        8,
    );
    floating_varargs(
        Target::LinuxGnu,
        "long double",
        "f80",
        0x3FFF_8887_8685_8483_8281,
        10,
    );
}

/// Scalar binary128 C lowering is refused; f128 remains reachable in IR.
#[test]
fn quad_varargs_cross_both_directions() {
    floating_varargs(
        Target::LinuxGnu,
        "__float128",
        "f128",
        0x3FFF_1234_5678_9ABC_DEF0_1234_5678_9ABC,
        16,
    );
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "The low word intentionally keeps the lower 64 bits."
)]
fn floating_varargs(target: Target, c_type: &str, ty: &str, bits: u128, bytes: usize) {
    let low = bits as u64;
    let high = bits >> 64;
    let mut ir = format!("target triple = \"{}\"\n", target.triple());
    let mut clang = format!(
        r"
#include <stdarg.h>
static {c_type} make(void) {{
  unsigned long long words[2] = {{0x{low:016x}ULL, 0x{high:016x}ULL}};
  {c_type} x; __builtin_memcpy(&x,words,sizeof(x)); return x;
}}
"
    );
    let mut main = "int b_call(void); int main(void){\n".to_owned();
    let mut caller = "function @b_call() -> i32 external {\nblock0:\n".to_owned();
    let mut next = 0;
    let mut results = Vec::new();
    // Two fixed GP arguments leave four registers: the fifth integer spills
    // eight bytes, forcing realignment before the System V wide stack value.
    for (case, (doubles, integers)) in [(7, 0), (8, 0), (7, 5), (8, 5)].into_iter().enumerate() {
        writeln!(
            ir,
            "function @c{case}(i32, i32, ...) -> i32 external\nfunction @b{case}(i32, i32, ...) \
             -> i32 external {{\n slot0 = stack_slot 24, align 8\n slot1 = stack_slot 16, align \
             16\nblock0(v0: i32, v1: i32):\n v2 = stack_addr slot0\n va_start v2"
        )
        .unwrap();
        let mut v = 3;
        for skipped in
            std::iter::repeat_n("f64", doubles).chain(std::iter::repeat_n("i64", integers))
        {
            writeln!(ir, " v{v} = va_arg.{skipped} v2").unwrap();
            v += 1;
        }
        writeln!(
            ir,
            " v{v} = va_arg.{ty} v2\n v{} = stack_addr slot1\n store.{ty} v{v}, v{}, align 16\n \
             v{} = load.i64 v{}, align 16\n v{} = iconst.i64 {}\n v{} = icmp.i64 ne v{}, v{}",
            v + 1,
            v + 1,
            v + 2,
            v + 1,
            v + 3,
            low as i64,
            v + 4,
            v + 2,
            v + 3
        )
        .unwrap();
        let address = v + 1;
        let mut bad = v + 4;
        v += 5;
        if bytes > 8 {
            let upper = if bytes == 10 { "i16" } else { "i64" };
            writeln!(
                ir,
                " v{v} = iconst.i64 8\n v{} = ptr_add v{address}, v{v}\n v{} = load.{upper} v{}, \
                 align 8\n v{} = iconst.{upper} {high}\n v{} = icmp.{upper} ne v{}, v{}\n v{} = \
                 or.i1 v{bad}, v{}",
                v + 1,
                v + 2,
                v + 1,
                v + 3,
                v + 4,
                v + 2,
                v + 3,
                v + 5,
                v + 4
            )
            .unwrap();
            bad = v + 5;
            v += 6;
        }
        writeln!(
            ir,
            " v{v} = va_arg.i32 v2\n v{} = iconst.i32 12345\n v{} = icmp.i32 ne v{v}, v{}\n v{} = \
             or.i1 v{bad}, v{}\n v{} = zext.i32 v{}\n va_end v2\n return v{}\n}}",
            v + 1,
            v + 2,
            v + 1,
            v + 3,
            v + 2,
            v + 4,
            v + 3,
            v + 4
        )
        .unwrap();
        let args = arguments(doubles, integers);
        writeln!(
            clang,
            "int b{case}(int,int,...); int c{case}(int n,int m,...) {{ va_list ap; \
             va_start(ap,m); for(int i=0;i<n;i++) (void)va_arg(ap,double); for(int i=0;i<m;i++) \
             (void)va_arg(ap,long long); {c_type} x=va_arg(ap,{c_type}); int \
             sentinel=va_arg(ap,int); va_end(ap); {c_type} expected=make(); unsigned char \
             *p=(unsigned char *)&x,*q=(unsigned char *)&expected; for(int i=0;i<{bytes};i++) \
             if(p[i]!=q[i]) return 1; return sentinel != 12345; }}"
        )
        .unwrap();
        writeln!(main, "if(b{case}({doubles},{integers},{args})) return 1;").unwrap();
        let mut args = Vec::new();
        for value in [doubles, integers] {
            writeln!(caller, " v{next} = iconst.i32 {value}").unwrap();
            args.push(format!("v{next}"));
            next += 1;
        }
        for i in 1..=doubles {
            let value = f64::from(u32::try_from(i).unwrap()).to_bits();
            writeln!(caller, " v{next} = fconst.f64 0x{value:016X}").unwrap();
            args.push(format!("v{next}"));
            next += 1;
        }
        for i in 1..=integers {
            writeln!(caller, " v{next} = iconst.i64 {i}").unwrap();
            args.push(format!("v{next}"));
            next += 1;
        }
        writeln!(caller, " v{next} = fconst.{ty} 0x{bits:X}").unwrap();
        args.push(format!("v{next}"));
        next += 1;
        writeln!(caller, " v{next} = iconst.i32 12345").unwrap();
        args.push(format!("v{next}"));
        next += 1;
        writeln!(caller, " v{next} = call @c{case}({})", args.join(", ")).unwrap();
        results.push(next);
        next += 1;
    }
    let mut result = results[0];
    for &value in &results[1..] {
        writeln!(caller, " v{next} = or.i32 v{result}, v{value}").unwrap();
        result = next;
        next += 1;
    }
    writeln!(caller, " return v{result}\n}}").unwrap();
    ir.push_str(&caller);
    main.push_str("return b_call();}\n");
    clang.push_str(&main);
    for bcc_opt in [false, true] {
        let arena = Bump::new();
        let mut module = parse_module(&arena, &ir).unwrap();
        assert_verified(&module, Profile::PreAbi, &ir);
        if bcc_opt {
            _ = optimize(&mut module, &OptimizerOptions::default(), &Bump::new());
        }
        lower_abi(&mut module, target, &Bump::new()).unwrap();
        assert_verified(&module, Profile::PostAbi, &module.to_string());
        assert_conforms(&format!("{ty}-varargs-{bcc_opt}"), target, &module, &clang);
    }
}

fn arguments(doubles: usize, integers: usize) -> String {
    let mut args: Vec<String> = (1..=doubles).map(|i| format!("{i}.0")).collect();
    args.extend((1..=integers).map(|i| format!("{i}LL")));
    args.extend(["make()".to_owned(), "12345".to_owned()]);
    args.join(", ")
}
