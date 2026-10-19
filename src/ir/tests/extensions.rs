//! Extended narrow integers in signatures: their textual form on parameters,
//! results and indirect calls, and the verifier's type, order and profile
//! rules.

use super::*;

/// A pre-ABI module whose signatures record the C signedness of `char`,
/// `short` and `_Bool` parameters and results.
const PRE_ABI: &str = "\
function @narrow(i8 signed, i16 unsigned, i32, i8 bool) -> i8 signed external

function @call(i8 unsigned) -> i16 unsigned external {
block0(v0: i8):
    v1 = iconst.i16 7
    v2 = iconst.i32 3
    v3 = iconst.i8 1
    v4 = call @narrow(v0, v1, v2, v3)
    v5 = func_addr @narrow
    v6 = call_indirect v5(v0, v1, v2, v3) : (i8 signed, i16 unsigned, i32, i8 bool) -> i8 signed
    v7 = sext.i16 v6
    return v7
}
";

/// The post-ABI forms, as ABI lowering leaves them for System V.
const POST_ABI: &str = "\
abi lowered

function @narrow(i8 signext, i16 zeroext, i32, i8 zeroext) -> i8 signext external
";

#[test]
fn extensions_round_trip() {
    assert_round_trip(PRE_ABI);
    assert_round_trip(POST_ABI);
}

#[test]
fn each_profile_accepts_only_its_own_extensions() {
    let arena = Bump::new();
    let pre = parse(&arena, PRE_ABI);
    assert_eq!(verify(&pre, Profile::PreAbi), Vec::<String>::new());
    assert_eq!(
        verify(&pre, Profile::PostAbi),
        [
            "@narrow: parameter 0 has an extension of the other profile",
            "@narrow: parameter 1 has an extension of the other profile",
            "@narrow: parameter 3 has an extension of the other profile",
            "@narrow: the result has an extension of the other profile",
            "@call: parameter 0 has an extension of the other profile",
            "@call: the result has an extension of the other profile",
            "@call, inst5: parameter 0 has an extension of the other profile",
            "@call, inst5: parameter 1 has an extension of the other profile",
            "@call, inst5: parameter 3 has an extension of the other profile",
            "@call, inst5: the result has an extension of the other profile",
        ]
    );
    let post = parse(&arena, POST_ABI);
    assert_eq!(verify(&post, Profile::PostAbi), Vec::<String>::new());
    assert_eq!(verify(&post, Profile::PreAbi).len(), 4);
}

#[test]
fn only_narrow_integers_are_extended() {
    let cases: [(&str, &[&str]); 3] = [
        (
            "function @f(i32 signed, ptr unsigned) external\n",
            &[
                "@f: parameter 0 has an extension but is not an i8 or i16",
                "@f: parameter 1 has an extension but is not an i8 or i16",
            ],
        ),
        (
            "function @f(i8) -> f32 bool external\n",
            &["@f: the result has an extension but is not an i8 or i16"],
        ),
        (
            "abi lowered\n\nfunction @f(i1 zeroext) -> i64 signext external\n",
            &[
                "@f: parameter 0 has an extension but is not an i8 or i16",
                "@f: the result has an extension but is not an i8 or i16",
            ],
        ),
    ];
    for (text, expected) in cases {
        let arena = Bump::new();
        let module = parse(&arena, text);
        let profile = module.profile();
        assert_eq!(verify(&module, profile), expected, "for\n{text}");
    }
}
