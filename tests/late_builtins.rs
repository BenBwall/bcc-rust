//! Linker-plugin LTO turns some operations into `compiler_builtins` calls
//! only while generating code, after LTO has dropped `rust_eh_personality`.
//! The objects behind those calls reference it from their unwind tables, so
//! `.cargo/config.toml` makes the linker fetch them before LTO. If that list
//! stops covering an operation, this test binary fails to link.

#![expect(
    unused_crate_dependencies,
    reason = "Integration tests inherit package dependencies they do not use."
)]

#[cfg(test)]
mod tests {
    use std::hint::black_box;

    #[test]
    fn signed_128_bit_division_links() {
        let (dividend, divisor) = (black_box(-7_i128), black_box(2_i128));
        assert_eq!(dividend / divisor, -3);
        assert_eq!(dividend % divisor, -1);
        assert_eq!(dividend.div_euclid(divisor), -4);
        assert_eq!(dividend.rem_euclid(divisor), 1);
    }
}
