#![expect(
    clippy::disallowed_macros,
    reason = "Generated study inputs use formatting outside measurements."
)]
//! Native runner tests use the same library seam and demand-paged worker.

use super::*;

#[test]
fn recursive_lowering_matches_tasks() {
    for source in [
        "int f(int a,int b){return (a+=b*3), (a<<2)>b ? (int)(a||b&&a) : (a^b)|(~b&7);}\n",
        "int f(int a){int n=0;for(int \
         i=0;i<a;++i){if(i&1)continue;n+=i;}while(n>100)n-=3;do{n++;}while(n<0);switch(a){case \
         0:return n;default:break;}return n;}\n",
        "int f(int a){int x[a++];return sizeof x + a;}\n",
        "int g(int*); int f(int x){return x += g(&x);}\n",
        "struct S{int x,y;}; int f(int x){return ((struct S){x,2}).x;}\n",
    ] {
        let (task, recursive) = with_compilation_stack(1024_usize.pow(3), || {
            (
                measure(source, false, false, true),
                measure(source, true, false, true),
            )
        })
        .expect("compilation worker");
        assert_eq!(task.errors, 0, "supported input");
        assert_eq!(task.diagnostics, 0, "clean input");
        assert_eq!(
            task.inspection, recursive.inspection,
            "IR/provenance/diagnostic identity"
        );
    }
}

#[test]
#[ignore = "explicit nine-sample native timing and deep lowering run"]
fn runner_lowering_measurements() {
    let mut cases = Vec::new();
    for (name, body) in [
        (
            "expressions",
            "int fINDEX(int a,int b){return (a+=b*3), (a<<2)>b ? (int)(a||b&&a) : (a^b)|(~b&7);}\n",
        ),
        (
            "statements",
            "int fINDEX(int a){int n=0;for(int \
             i=0;i<a;++i){if(i&1)continue;n+=i;}while(n>100)n-=3;do{n++;}while(n<0);\
             switch(a){case 0:return n;default:break;}return n;}\n",
        ),
    ] {
        let source: String = (0..4000)
            .map(|i| body.replace("INDEX", &i.to_string()))
            .collect();
        cases.push((name.to_owned(), source));
    }
    let mut paths: Vec<_> = fs::read_dir("benches/c")
        .expect("C benchmarks")
        .map(|e| e.expect("input").path())
        .filter(|p| p.extension().is_some_and(|s| s == "c"))
        .collect();
    paths.sort();
    for path in paths {
        cases.push((
            path.to_string_lossy().into_owned(),
            fs::read_to_string(path).expect("source"),
        ));
    }
    for (name, source) in cases {
        let results = with_compilation_stack(1024_usize.pow(3), || {
            [false, true].map(|recursive| measure(&source, recursive, false, true))
        })
        .expect("worker");
        assert_eq!(
            results[0].inspection, results[1].inspection,
            "{name}: differential result"
        );
        println!(
            "lowering_parity input={name} identical=1 errors={} diagnostics={}",
            results[0].errors, results[0].diagnostics
        );
        if results[0].errors != 0 || results[0].diagnostics != 0 {
            continue;
        }
        for repeat in 0..9 {
            for offset in 0..2 {
                let recursive = (repeat + offset) % 2 != 0;
                let wall = Instant::now();
                let m = with_compilation_stack(1024_usize.pow(3), || {
                    measure(&source, recursive, false, false)
                })
                .expect("worker");
                assert_eq!(m.errors, 0);
                println!(
                    "lowering_timing input={name} repeat={repeat} recursive={recursive} ns={} \
                     total_ns={} scratch_bytes={} ir_bytes={}",
                    m.nanoseconds,
                    wall.elapsed().as_nanos(),
                    m.scratch_bytes,
                    m.ir_bytes
                );
            }
        }
    }
    for depth in [256, 4096] {
        for (name, source) in [
            (
                "expression",
                format!("int f(int a){{return {}a;}}\n", "a+".repeat(depth)),
            ),
            (
                "blocks",
                format!(
                    "int f(void){{{}return 1;{}}}\n",
                    "{".repeat(depth),
                    "}".repeat(depth)
                ),
            ),
        ] {
            let m =
                with_compilation_stack(1024_usize.pow(3), || measure(&source, true, true, false))
                    .expect("deep compilation worker");
            assert_eq!(m.errors, 0);
            println!(
                "lowering_stack input={name} levels={depth} ns={} stack_span={} depth={} \
                 scratch_bytes={} ir_bytes={}",
                m.nanoseconds, m.stack_span, m.max_depth, m.scratch_bytes, m.ir_bytes
            );
        }
    }
}
