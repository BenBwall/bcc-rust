//! The characteristics macros of the interchange and extended floating
//! types, `__FLT16_MANT_DIG__` through `__FLT128_IS_IEC_60559__`, as GCC 7
//! and later predefine them for its `<float.h>` and the C library. The
//! values are those GCC 15 predefines for x86-64; the pinned Clang defines
//! only the `_Float16` ones, spelled otherwise and without
//! `__FLT16_IS_IEC_60559__`, which the frozen target tables keep. GCC also
//! describes `__bf16` in `__BFLT16_MANT_DIG__` and its relatives; Clang
//! does not.
//!
//! C23: the `<float.h>` characteristics of these types, §H.3 paragraph 7,
//! pp. 556-558; PDF pp. 569-571, named after those of the standard types,
//! which C99 gives in §5.2.4.2.2, pp. 23-27; PDF pp. 35-39. GCC
//! preprocessor manual, "Common Predefined Macros",
//! <https://gcc.gnu.org/onlinedocs/cpp/Common-Predefined-Macros.html>.

use core::fmt::Write;

use super::Target;

impl Target {
    /// Appends the characteristics macros of every interchange and extended
    /// floating type the target provides: `_Float32`, `_Float64` and
    /// `_Float32x` always, `_Float64x` where `long double` is wider than
    /// binary64, and `_Float128` where binary128 is a type. Then
    /// `__FLT_EVAL_METHOD_TS_18661_3__`, the evaluation method counting
    /// these types, which glibc's `<math.h>` prefers to `__FLT_EVAL_METHOD__`:
    /// 0, since every bcc target evaluates each type in its own format.
    pub(crate) fn write_interchange_float_macros(self, out: &mut impl Write) {
        for characteristics in CHARACTERISTICS {
            let provided = match characteristics.name {
                | "FLT64X" => !self.layout().long_double_is_double(),
                | "FLT128" => self.float128_type(),
                | _ => true,
            };
            if provided {
                characteristics.write(out);
            }
        }
        _ = out.write_str("#define __FLT_EVAL_METHOD_TS_18661_3__ 0\n");
    }

    /// Appends GCC's characteristics macros of `_Float16` and `__bf16`,
    /// which it defines whenever the types exist, apart from the other
    /// interchange types.
    pub(crate) fn write_half_float_macros(out: &mut impl Write) {
        HALF.write(out);
        BFLOAT.write(out);
    }
}

/// One type's `<float.h>` characteristics. Every type has subnormals,
/// infinities and quiet NaNs, and all but bfloat16 are IEC 60559 formats.
struct InterchangeCharacteristics {
    /// The macro prefix after `__`, as in `FLT32`.
    name:        &'static str,
    /// The constant suffix of the type's values, as in `F32`.
    suffix:      &'static str,
    mant_dig:    u32,
    dig:         u32,
    min_exp:     i32,
    min_10_exp:  i32,
    max_exp:     i32,
    max_10_exp:  i32,
    decimal_dig: u32,
    max:         &'static str,
    min:         &'static str,
    epsilon:     &'static str,
    denorm_min:  &'static str,
    iec_60559:   bool,
}

impl InterchangeCharacteristics {
    fn write(&self, out: &mut impl Write) {
        let Self {
            name,
            suffix,
            mant_dig,
            dig,
            min_exp,
            min_10_exp,
            max_exp,
            max_10_exp,
            decimal_dig,
            max,
            min,
            epsilon,
            denorm_min,
            iec_60559,
        } = self;
        let iec_60559 = u8::from(*iec_60559);
        _ = write!(
            out,
            "#define __{name}_MANT_DIG__ {mant_dig}\n#define __{name}_DIG__ {dig}\n#define \
             __{name}_MIN_EXP__ ({min_exp})\n#define __{name}_MIN_10_EXP__ \
             ({min_10_exp})\n#define __{name}_MAX_EXP__ {max_exp}\n#define __{name}_MAX_10_EXP__ \
             {max_10_exp}\n#define __{name}_DECIMAL_DIG__ {decimal_dig}\n#define __{name}_MAX__ \
             {max}{suffix}\n#define __{name}_NORM_MAX__ {max}{suffix}\n#define __{name}_MIN__ \
             {min}{suffix}\n#define __{name}_EPSILON__ {epsilon}{suffix}\n#define \
             __{name}_DENORM_MIN__ {denorm_min}{suffix}\n#define __{name}_HAS_DENORM__ 1\n#define \
             __{name}_HAS_INFINITY__ 1\n#define __{name}_HAS_QUIET_NAN__ 1\n#define \
             __{name}_IS_IEC_60559__ {iec_60559}\n",
        );
    }
}

/// GCC 15's x86-64 values of `_Float16`, binary16.
const HALF: InterchangeCharacteristics = InterchangeCharacteristics {
    name:        "FLT16",
    suffix:      "F16",
    mant_dig:    11,
    dig:         3,
    min_exp:     -13,
    min_10_exp:  -4,
    max_exp:     16,
    max_10_exp:  4,
    decimal_dig: 5,
    max:         "6.55040000000000000000000000000000000e+4",
    min:         "6.10351562500000000000000000000000000e-5",
    epsilon:     "9.76562500000000000000000000000000000e-4",
    denorm_min:  "5.96046447753906250000000000000000000e-8",
    iec_60559:   true,
};

/// GCC 15's x86-64 values of `__bf16`, bfloat16, which IEC 60559 does not
/// define.
const BFLOAT: InterchangeCharacteristics = InterchangeCharacteristics {
    name:        "BFLT16",
    suffix:      "BF16",
    mant_dig:    8,
    dig:         2,
    min_exp:     -125,
    min_10_exp:  -37,
    max_exp:     128,
    max_10_exp:  38,
    decimal_dig: 4,
    max:         "3.38953138925153547590470800371487867e+38",
    min:         "1.17549435082228750796873653722224568e-38",
    epsilon:     "7.81250000000000000000000000000000000e-3",
    denorm_min:  "9.18354961579912115600575419704879436e-41",
    iec_60559:   false,
};

/// GCC 15's x86-64 values: binary32, binary64, binary64, the x87 extended
/// format and binary128.
const CHARACTERISTICS: [InterchangeCharacteristics; 5] = [
    InterchangeCharacteristics {
        name:        "FLT32",
        suffix:      "F32",
        mant_dig:    24,
        dig:         6,
        min_exp:     -125,
        min_10_exp:  -37,
        max_exp:     128,
        max_10_exp:  38,
        decimal_dig: 9,
        max:         "3.40282346638528859811704183484516925e+38",
        min:         "1.17549435082228750796873653722224568e-38",
        epsilon:     "1.19209289550781250000000000000000000e-7",
        denorm_min:  "1.40129846432481707092372958328991613e-45",
        iec_60559:   true,
    },
    InterchangeCharacteristics {
        name:        "FLT64",
        suffix:      "F64",
        mant_dig:    53,
        dig:         15,
        min_exp:     -1021,
        min_10_exp:  -307,
        max_exp:     1024,
        max_10_exp:  308,
        decimal_dig: 17,
        max:         "1.79769313486231570814527423731704357e+308",
        min:         "2.22507385850720138309023271733240406e-308",
        epsilon:     "2.22044604925031308084726333618164062e-16",
        denorm_min:  "4.94065645841246544176568792868221372e-324",
        iec_60559:   true,
    },
    InterchangeCharacteristics {
        name:        "FLT32X",
        suffix:      "F32x",
        mant_dig:    53,
        dig:         15,
        min_exp:     -1021,
        min_10_exp:  -307,
        max_exp:     1024,
        max_10_exp:  308,
        decimal_dig: 17,
        max:         "1.79769313486231570814527423731704357e+308",
        min:         "2.22507385850720138309023271733240406e-308",
        epsilon:     "2.22044604925031308084726333618164062e-16",
        denorm_min:  "4.94065645841246544176568792868221372e-324",
        iec_60559:   true,
    },
    InterchangeCharacteristics {
        name:        "FLT64X",
        suffix:      "F64x",
        mant_dig:    64,
        dig:         18,
        min_exp:     -16381,
        min_10_exp:  -4931,
        max_exp:     16384,
        max_10_exp:  4932,
        decimal_dig: 21,
        max:         "1.18973149535723176502126385303097021e+4932",
        min:         "3.36210314311209350626267781732175260e-4932",
        epsilon:     "1.08420217248550443400745280086994171e-19",
        denorm_min:  "3.64519953188247460252840593361941982e-4951",
        iec_60559:   true,
    },
    InterchangeCharacteristics {
        name:        "FLT128",
        suffix:      "F128",
        mant_dig:    113,
        dig:         33,
        min_exp:     -16381,
        min_10_exp:  -4931,
        max_exp:     16384,
        max_10_exp:  4932,
        decimal_dig: 36,
        max:         "1.18973149535723176508575932662800702e+4932",
        min:         "3.36210314311209350626267781732175260e-4932",
        epsilon:     "1.92592994438723585305597794258492732e-34",
        denorm_min:  "6.47517511943802511092443895822764655e-4966",
        iec_60559:   true,
    },
];
