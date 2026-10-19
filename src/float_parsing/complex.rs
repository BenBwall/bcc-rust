//! Native complex arithmetic uses real scalar and output-pointer interfaces.
//! No guest memory crosses the bridge. C99: §6.5.5-§6.5.6, pp. 82-84;
//! PDF pp. 94-96; Annex G.5.1, pp. 469-472; PDF pp. 481-484.

use super::{
    LongDouble,
    ffi,
};

pub(crate) fn complex_f32(a: [f32; 2], b: [f32; 2], operation: i32) -> [f32; 2] {
    let (mut real, mut imag) = (0.0, 0.0);
    // SAFETY: Inputs have their declared real types and outputs are host
    // temporaries.
    unsafe {
        ffi::bcc_complex_float(
            a[0],
            a[1],
            b[0],
            b[1],
            operation,
            &raw mut real,
            &raw mut imag,
        );
    }
    [real, imag]
}

pub(crate) fn complex_f64(a: [f64; 2], b: [f64; 2], operation: i32) -> [f64; 2] {
    let (mut real, mut imag) = (0.0, 0.0);
    // SAFETY: Inputs have their declared real types and outputs are host
    // temporaries.
    unsafe {
        ffi::bcc_complex_double(
            a[0],
            a[1],
            b[0],
            b[1],
            operation,
            &raw mut real,
            &raw mut imag,
        );
    }
    [real, imag]
}

impl LongDouble {
    /// Fold complex constants in their target component precision.
    /// C99: §6.3.1.8, pp. 44-45; PDF pp. 56-57; §6.6, pp. 95-96; PDF pp.
    /// 107-108.
    pub(crate) fn complex_arithmetic(
        a: [Self; 2],
        b: [Self; 2],
        operation: i32,
        precision: i32,
    ) -> [Self; 2] {
        let (mut real, mut imag) = (Self::ZERO.to_ffi(), Self::ZERO.to_ffi());
        // SAFETY: Initialized carriers and writable host output objects.
        unsafe {
            ffi::long_double_complex_arithmetic(
                a[0].to_ffi(),
                a[1].to_ffi(),
                b[0].to_ffi(),
                b[1].to_ffi(),
                operation,
                precision,
                &raw mut real,
                &raw mut imag,
            );
        }
        // SAFETY: The bridge initialized the entire real output carrier.
        let real = unsafe { real.bytes };
        // SAFETY: The bridge initialized the entire imaginary output carrier.
        let imag = unsafe { imag.bytes };
        [Self { value: real }, Self { value: imag }]
    }
}
