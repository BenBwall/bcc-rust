//! Opt-in parser timing and parity runs accepted by the Windows test
//! dispatcher.
#![expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "The runner prepares owned inputs outside the parser measurement."
)]

use std::{
    fs,
    path::Path,
};

use bcc_rust::{
    ParserStudyMode,
    with_study_source,
};

use super::{
    reserved_stack,
    run,
};

#[test]
#[ignore = "explicit native runner benchmark with nine rotating samples"]
fn runner_parser_measurements() {
    let expressions: String = (0..4000)
        .map(|index| {
            format!(
                "int expr{index}(int a,int b,int *p){{return (a+=b*3), (a<<2)+p[a&3] > b ? \
                 (int)(a||b&&p[0]) : (a^b)|(~b&7);}}\n"
            )
        })
        .collect();
    let statements: String = (0..4000)
        .map(|index| {
            format!(
                "int stmt{index}(int a){{int n=0;for(int \
                 i=0;i<a;++i){{if(i&1)continue;n+=i;}}while(n>100)n-=3;do{{n++;}}while(n<0);\
                 switch(a){{case 0:return n;default:break;}}return n;}}\n"
            )
        })
        .collect();
    for (name, source) in [
        ("expression-mix.c", expressions),
        ("statement-mix.c", statements),
    ] {
        timed(name, &source);
    }
    let mut paths: Vec<_> = fs::read_dir("benches/c")
        .expect("benchmark directory")
        .map(|entry| entry.expect("benchmark input").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "c"))
        .collect();
    paths.sort();
    for path in paths {
        let source = fs::read_to_string(&path).expect("benchmark C source");
        parity(&source);
        timed(path.to_str().expect("UTF-8 benchmark path"), &source);
    }
    // Optional exact preprocessed corpus, if a future runner stages it.
    let lua = Path::new(".scratch/parser-study/inputs/lua-supported.i");
    if lua.is_file() {
        let source = fs::read_to_string(lua).expect("staged preprocessed Lua");
        timed("lua-supported.i", &source);
    } else {
        println!("runner_parser input=lua-supported.i staged=0");
    }
    for depth in [4096, 16_384] {
        let parentheses = format!(
            "int f(void){{return {}1{};}}\n",
            "(".repeat(depth),
            ")".repeat(depth)
        );
        let blocks = format!(
            "int f(void){{{}int x;{}return 0;}}\n",
            "{".repeat(depth),
            "}".repeat(depth)
        );
        for (kind, source) in [("parentheses", parentheses), ("blocks", blocks)] {
            println!("runner_parser kind=stack input={kind} depth={depth}");
            reserved_stack::run(1024_usize.pow(3), |bounds| {
                run(
                    &source,
                    1,
                    ParserStudyMode::Recursive,
                    true,
                    None,
                    Some(bounds),
                    true,
                );
            })
            .expect("reserved parser stack");
        }
    }
}

fn timed(name: &str, source: &str) {
    let modes = [
        ("machine", ParserStudyMode::Machine, false),
        ("recursive", ParserStudyMode::Recursive, false),
        ("reserved:machine", ParserStudyMode::Machine, true),
        ("reserved:recursive", ParserStudyMode::Recursive, true),
    ];
    for repeat in 0..9 {
        for offset in 0..modes.len() {
            let (label, mode, reserved) = modes[(repeat + offset) % modes.len()];
            println!("runner_parser kind=timing input={name} repeat={repeat} mode={label}");
            if reserved {
                reserved_stack::run(1024_usize.pow(3), |bounds| {
                    run(source, 1, mode, false, None, Some(bounds), false);
                })
                .expect("reserved parser stack");
            } else {
                run(source, 1, mode, false, None, None, false);
            }
        }
    }
}

fn parity(source: &str) {
    let inspect = |mode| {
        with_study_source(source, |prepared| {
            prepared
                .measure_with_mode(mode, false, true, || {}, || {})
                .inspection
        })
    };
    let baseline = inspect(ParserStudyMode::Machine);
    for mode in [
        ParserStudyMode::Recursive,
        ParserStudyMode::Growing,
        ParserStudyMode::Hybrid { depth: 16 },
    ] {
        assert_eq!(baseline, inspect(mode), "AST/provenance/diagnostics parity");
    }
    let reserved = reserved_stack::run(1024_usize.pow(3), |_| inspect(ParserStudyMode::Recursive))
        .expect("reserved parity worker");
    assert_eq!(
        baseline, reserved,
        "reserved AST/provenance/diagnostics parity"
    );
    println!("runner_parser kind=parity identical=1");
}
