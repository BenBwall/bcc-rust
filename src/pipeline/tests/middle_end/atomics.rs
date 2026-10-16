//! Atomic execution, memory barriers and native concurrency regressions.
use super::*;

fn with_atomic_module<R>(
    source: &str,
    target: Target,
    optimize: bool,
    profile: Profile,
    then: impl FnOnce(&Module<'_>, Target) -> R,
) -> R {
    with_configured_module(
        source,
        target,
        optimize,
        profile,
        CompilerConfiguration::default()
            .with_target(target)
            .with_gnu_extensions(true),
        then,
    )
}

const PROGRAMS: &[(&str, &str)] = &[
    (
        "stdatomic",
        r"
#include <stdatomic.h>
static atomic_int x = ATOMIC_VAR_INIT(2);
int main(void) {
    atomic_init(&x, 3);
    if (atomic_load(&x) != 3) return 1;
    atomic_store_explicit(&x, 5, memory_order_release);
    if (atomic_exchange_explicit(&x, 7, memory_order_acq_rel) != 5) return 2;
    int expected = 6;
    if (atomic_compare_exchange_strong_explicit(&x, &expected, 9, memory_order_acq_rel, memory_order_acquire) || expected != 7 || x != 7) return 3;
    if (!atomic_compare_exchange_weak(&x, &expected, 9) || expected != 7 || x != 9) return 4;
    if (atomic_fetch_add(&x, 3) != 9 || atomic_fetch_sub(&x, 2) != 12) return 5;
    if (atomic_fetch_and(&x, 6) != 10 || atomic_fetch_or(&x, 8) != 2 || atomic_fetch_xor(&x, 3) != 10 || x != 9) return 6;
    atomic_flag flag = ATOMIC_FLAG_INIT;
    if (atomic_flag_test_and_set(&flag) || !atomic_flag_test_and_set_explicit(&flag, memory_order_acquire)) return 7;
    atomic_flag_clear_explicit(&flag, memory_order_release);
    if (atomic_flag_test_and_set(&flag)) return 8;
    atomic_thread_fence(memory_order_acq_rel);
    atomic_signal_fence(memory_order_seq_cst);
    atomic_thread_fence(memory_order_relaxed);
    if (!atomic_is_lock_free(&x)) return 9;
    return 0;
}",
    ),
    (
        "implicit_objects",
        r"
_Atomic int x = 2;
int calls;
_Atomic int *p(void) { ++calls; return &x; }
int rhs(void) { x=10; return 3; }
int main(void) {
    if ((*p() += rhs()) != 13 || calls != 1 || x != 13) return 1;
    if (x++ != 13 || ++x != 15 || x-- != 15 || --x != 13) return 2;
    x *= 3; x /= 2; x %= 7; x <<= 2; x >>= 1; x ^= 3; x |= 16; x &= 23;
    if(x != 17) return 3;
    _Atomic unsigned char narrow = 255;
    if(narrow++ != 255 || narrow != 0 || --narrow != 255) return 4;
    _Atomic _Bool boolean = 1;
    if(boolean++ != 1 || boolean != 1 || --boolean != 0) return 5;
    int array[4]; _Atomic(int *) pointer = array;
    if(pointer++ != array || pointer != array+1 || (pointer += 2) != array+3) return 6;
    if(__c11_atomic_fetch_sub(&pointer, 2, 5) != array+3 || pointer != array+1) return 7;
    volatile _Atomic int v = 3; v += 4; if(v != 7) return 8;
    _Atomic int elements[2]={1,2};
    struct Fields { _Atomic int member; } fields={3};
    elements[1]+=fields.member++; if(elements[1]!=5 || fields.member!=4) return 9;
    return 0;
}",
    ),
    (
        "gnu_and_sync",
        r"
int x=3;
int main(void) {
    if(__atomic_fetch_add(&x,2,0)!=3 || __atomic_sub_fetch(&x,1,5)!=4) return 1;
    if(__atomic_and_fetch(&x,3,5)!=0 || __atomic_or_fetch(&x,6,5)!=6 || __atomic_xor_fetch(&x,3,5)!=5) return 2;
    if(__atomic_fetch_nand(&x,3,5)!=5 || x != ~1) return 3;
    if(__atomic_nand_fetch(&x,7,5)!=~6) return 4;
    x=9;
    if(__sync_fetch_and_add(&x,1)!=9 || __sync_sub_and_fetch(&x,2)!=8 || __sync_fetch_and_and(&x,7)!=8) return 5;
    if(__sync_or_and_fetch(&x,3)!=3 || __sync_xor_and_fetch(&x,1)!=2 || __sync_nand_and_fetch(&x,3)!=~2) return 6;
    if(__sync_val_compare_and_swap(&x,1,4)!=~2 || __sync_bool_compare_and_swap(&x,1,4)) return 7;
    if(!__sync_bool_compare_and_swap(&x,~2,4) || __sync_val_compare_and_swap(&x,4,5)!=4 || x!=5) return 8;
    if(__sync_lock_test_and_set(&x,1)!=5) return 9;
    __sync_lock_release(&x); __sync_synchronize(); if(x!=0) return 10;
    int expected=0;
    if(!__atomic_compare_exchange_n(&x,&expected,7,1,4,2) || expected!=0) return 11;
    expected=1;
    if(__atomic_compare_exchange_n(&x,&expected,8,0,5,5) || expected!=7 || x!=7) return 12;
    char flag=0; if(__atomic_test_and_set(&flag,2) || !__atomic_test_and_set(&flag,0)) return 13;
    __atomic_clear(&flag,3); if(flag) return 14;
    if(!__atomic_always_lock_free(8,0) || __atomic_always_lock_free(3,0) || !__atomic_always_lock_free(0,0)) return 15;
    volatile unsigned size=4;
    if(!__atomic_is_lock_free(size,0) || __atomic_is_lock_free(16,0)) return 16;
    char buffer[16]; int *pointer=(int *)buffer;
    if(__atomic_fetch_add(&pointer,1,5)!=(int *)buffer || (char *)pointer!=buffer+1) return 17;
    if(__sync_fetch_and_add(&pointer,(int *)1)!=(int *)(buffer+1) || (char *)pointer!=buffer+2) return 18;
    return 0;
}",
    ),
    (
        "generic_buffers",
        r"
struct Small { char a[3]; };
struct Large { long long a[4]; };
struct Padded { char c; int value; };
int main(void) {
    struct Small x={{1,2,3}}, y={{4,5,6}}, out={{0}}, expected={{7,8,9}};
    __atomic_load(&x,&out,2); if(out.a[0]!=1 || out.a[2]!=3) return 1;
    __atomic_store(&x,&y,3); __atomic_exchange(&x,&out,&y,4);
    if(x.a[0]!=1 || y.a[0]!=4 || y.a[2]!=6) return 2;
    if(__atomic_compare_exchange(&x,&expected,&y,0,5,5) || expected.a[0]!=1 || expected.a[2]!=3) return 3;
    if(!__atomic_compare_exchange(&x,&expected,&y,1,5,2) || x.a[0]!=4 || x.a[2]!=6) return 4;
    struct Large a={{1,2,3,4}}, b={{5,6,7,8}}, c={{0}};
    __atomic_load(&a,&c,5); if(c.a[3]!=4) return 5;
    __atomic_exchange(&a,&b,&c,5); if(c.a[3]!=4 || a.a[3]!=8) return 6;
    if(__atomic_compare_exchange(&a,&c,&b,0,5,5) || c.a[3]!=8) return 7;
    struct Large initial={{1,2,3,4}};
    _Atomic(struct Large) atomic_a = initial;
    struct Large copy = atomic_a;
    if(copy.a[3]!=4) return 8;
    __c11_atomic_store(&atomic_a,b,5); copy = __c11_atomic_exchange(&atomic_a,a,5);
    if(copy.a[3]!=8) return 9;
    copy.a[3]=0;
    if(__c11_atomic_compare_exchange_strong(&atomic_a,&copy,b,5,5) || copy.a[3]!=8) return 10;
    if(!__c11_atomic_compare_exchange_weak(&atomic_a,&copy,b,5,2)) return 11;
    struct Padded padded_initial={1,42};
    _Atomic(struct Padded) padded = padded_initial;
    struct Padded padded_copy=padded;
    if(padded_copy.c!=1 || padded_copy.value!=42) return 12;
    padded_copy.c=2; padded_copy.value=43;
    __c11_atomic_store(&padded,padded_copy,5);
    padded_copy=__c11_atomic_exchange(&padded,padded_copy,5);
    if(padded_copy.c!=2 || padded_copy.value!=43) return 13;
    return 0;
}",
    ),
    (
        "floating_and_widths",
        r#"
int main(void) {
    _Atomic(float) a=1.5f;
    if(__c11_atomic_fetch_add(&a,2.0f,5)!=1.5f || a!=3.5f) return 1;
    if(__c11_atomic_fetch_sub(&a,0.5f,0)!=3.5f || a!=3.0f) return 2;
    float expected=2.0f;
    if(__c11_atomic_compare_exchange_strong(&a,&expected,4.0f,5,5) || expected!=3.0f) return 3;
    if(!__c11_atomic_compare_exchange_weak(&a,&expected,4.0f,5,2) || a!=4.0f) return 4;
    a *= 2.0f; a /= 4.0f; if(a!=2.0f) return 5;
    _Atomic unsigned __int128 wide=(((unsigned __int128)1)<<100);
    unsigned __int128 old=__c11_atomic_fetch_add(&wide,3,5);
    if(old!=(((unsigned __int128)1)<<100) || wide!=old+3) return 6;
    _Atomic int signed_value=2147483647;
    if(__c11_atomic_fetch_add(&signed_value,1,5)!=2147483647 || signed_value!=(-2147483647-1)) return 9;
    if(__c11_atomic_fetch_sub(&signed_value,1,0)!=(-2147483647-1) || signed_value!=2147483647) return 10;
    if(signed_value++!=2147483647 || signed_value!=(-2147483647-1)) return 15;
    if(--signed_value!=2147483647 || (signed_value+=1)!=(-2147483647-1)) return 16;
    if((signed_value-=1)!=2147483647 || ++signed_value!=(-2147483647-1)) return 17;
    _Atomic double d=3.0;
    if(__c11_atomic_fetch_min(&d,2.0,5)!=3.0 || d!=2.0) return 11;
    if(__c11_atomic_fetch_max(&d,4.0,5)!=2.0 || d!=4.0) return 12;
    double ordinary=3.0, nan=__builtin_nan("");
    if(__atomic_min_fetch(&ordinary,nan,5)!=3.0 || ordinary!=3.0) return 13;
    ordinary=nan;
    if(__atomic_max_fetch(&ordinary,5.0,5)!=5.0 || ordinary!=5.0) return 14;
    _Atomic unsigned short half=65535; if(half++!=65535 || half!=0) return 7;
    _Atomic unsigned long long full=~0ull; if(++full!=0) return 8;
    return 0;
}"#,
    ),
    (
        "barriers_and_evaluation",
        r"
int x=3, count, order;
int *object(void){++count; return &x;}
int value(void){++count; return 5;}
int ordering(void){++count; return order;}
int main(void) {
    int before=x;
    if(__atomic_exchange_n(object(),value(),ordering())!=3 || count!=3 || x!=5 || before!=3) return 1;
    int after=x;
    __atomic_fetch_add(&x,4,0);
    if(after!=5 || x!=9) return 2;
    int expected=9;
    if(!__atomic_compare_exchange_n(&x,&expected,11,0,5,5) || x!=11) return 3;
    if(__atomic_load_n(&x,2)!=11) return 4;
    volatile int branch=1;
    int before_join=x;
    if(branch) __atomic_store_n(&x,17,0);
    if(before_join!=11 || x!=17) return 5;
    branch=0; before_join=x;
    if(branch) __atomic_store_n(&x,19,0);
    else __atomic_exchange_n(&x,23,0);
    if(before_join!=17 || x!=23) return 6;
    return 0;
}",
    ),
];

#[test]
fn atomic_issue_62_reproducer_matches_reference_output() {
    let Some(target @ Target::LinuxGnu) = host() else {
        eprintln!("skipping issue 62 real-header replay: requires a Linux GNU host");
        return;
    };
    let includes = [
        Path::new("/usr/include/x86_64-linux-gnu"),
        Path::new("/usr/include"),
    ];
    if !includes[1].join("stdio.h").is_file() {
        eprintln!("skipping issue 62 real-header replay: no host stdio.h");
        return;
    }
    let headers = HeaderSearch {
        after: &includes,
        ..HeaderSearch::default()
    };
    let configuration = CompilerConfiguration::default()
        .with_target(target)
        .with_gnu_extensions(true);
    let source = include_str!(
        "../../../../docs/research/conformance-reproducers-2026-10-14/atomic_builtins_not_lowered.\
         c"
    );
    let expected = (0, "42 33 9 5 5\n".to_owned());
    for (optimize, profile) in STAGES {
        let actual = with_configured_headers(
            source,
            target,
            optimize,
            profile,
            configuration,
            headers,
            interpret,
        );
        assert_eq!(actual, expected, "{target:?} {optimize} {profile:?}");
    }
    let directory = TempDir::new("atomic-issue-62");
    for arm in ARMS {
        let executable = directory.join(format!("{}.exe", arm.name));
        with_configured_headers(
            source,
            target,
            arm.bcc,
            Profile::PostAbi,
            configuration,
            headers,
            |module, _| {
                compile_module(
                    module,
                    &executable,
                    LlvmOptions {
                        opt: Some(arm.opt),
                        disable_llvm_passes: arm.disable_llvm_passes,
                        target,
                        kind: OutputKind::Executable,
                    },
                )
                .expect("compile issue 62 reproducer");
            },
        );
        assert_eq!(run_native(&executable), expected, "{}", arm.name);
    }
}

#[test]
fn atomic_programs_execute_in_all_interpreter_stages() {
    for &(name, source) in PROGRAMS {
        for target in [Target::LinuxGnu, Target::WindowsGnu] {
            for (optimize, profile) in STAGES {
                let actual = with_atomic_module(source, target, optimize, profile, interpret);
                assert_eq!(actual.0, 0, "{name} {target:?} {optimize} {profile:?}");
            }
        }
    }
}

#[test]
fn atomic_programs_execute_in_all_llvm_arms() {
    let Some(target) = host() else { return };
    let directory = TempDir::new("atomics");
    for &(name, source) in PROGRAMS {
        for arm in ARMS {
            let exe = directory.join(format!("{name}-{}.exe", arm.name));
            with_atomic_module(source, target, arm.bcc, Profile::PostAbi, |module, _| {
                compile_module(
                    module,
                    &exe,
                    LlvmOptions {
                        opt: Some(arm.opt),
                        disable_llvm_passes: arm.disable_llvm_passes,
                        target,
                        kind: OutputKind::Executable,
                    },
                )
                .expect("compile atomics");
            });
            assert_eq!(run_native(&exe).0, 0, "{name} {}", arm.name);
        }
    }
}

#[test]
fn atomic_native_updates_survive_concurrent_callers() {
    let Some(target @ Target::LinuxGnu) = host() else {
        return;
    };
    let directory = TempDir::new("atomic-concurrency");
    let harness = directory.join("threads.c");
    std::fs::write(&harness, r"
#include <pthread.h>
#include <stdatomic.h>
extern _Atomic int counter;
extern void increment(void);
static void *run(void *unused) { (void)unused; for(int i=0;i<10000;i++) increment(); return 0; }
int main(void) { pthread_t threads[4]; for(int i=0;i<4;i++) if(pthread_create(&threads[i],0,run,0)) return 2; for(int i=0;i<4;i++) pthread_join(threads[i],0); return counter!=40000; }
").unwrap();
    for arm in ARMS {
        let object = directory.join(format!("{}.o", arm.name));
        let exe = directory.join(format!("{}.exe", arm.name));
        with_atomic_module(
            "_Atomic int counter; void increment(void) { counter++; }",
            target,
            arm.bcc,
            Profile::PostAbi,
            |module, _| {
                compile_module(
                    module,
                    &object,
                    LlvmOptions {
                        opt: Some(arm.opt),
                        disable_llvm_passes: arm.disable_llvm_passes,
                        target,
                        kind: OutputKind::Object,
                    },
                )
                .unwrap();
            },
        );
        let output = Command::new(Toolchain::bundled(target).unwrap().clang())
            .arg(&harness)
            .arg(&object)
            .arg("-pthread")
            .arg("-latomic")
            .arg("-o")
            .arg(&exe)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(run_native(&exe).0, 0, "{}", arm.name);
    }
}

#[test]
fn native_backend_truthfully_refuses_atomic_execution() {
    with_atomic_module(
        "_Atomic int x; int main(void) { return x++; }",
        Target::LinuxGnu,
        true,
        Profile::PostAbi,
        |module, _| {
            let mut assembly = String::new();
            let error =
                crate::backend::native::emit_assembly(module, Target::LinuxGnu, &mut assembly)
                    .unwrap_err();
            assert!(error.to_string().contains("atomic"), "{error}");
        },
    );
}

#[test]
fn atomic_x87_storage_uses_padded_memory_representation() {
    let Some(target @ Target::LinuxGnu) = host() else {
        return;
    };
    let directory = TempDir::new("atomic-x87");
    let source = r"
_Atomic(long double) global=1.5L;
int main(void) {
    _Atomic(long double) local=2.5L;
    if(global!=1.5L || local!=2.5L) return 1;
    global=3.0L; if(__c11_atomic_exchange(&global,4.0L,5)!=3.0L) return 2;
    long double expected=2.0L;
    if(__c11_atomic_compare_exchange_strong(&global,&expected,5.0L,5,5) || expected!=4.0L) return 3;
    if(!__c11_atomic_compare_exchange_strong(&global,&expected,5.0L,5,2)) return 4;
    local+=1.0L; local*=2.0L; if(local!=7.0L) return 5;
    if(local++!=7.0L || local!=8.0L) return 6;
    return 0;
}";
    for arm in ARMS {
        let exe = directory.join(format!("{}.exe", arm.name));
        with_atomic_module(source, target, arm.bcc, Profile::PostAbi, |module, _| {
            compile_module(
                module,
                &exe,
                LlvmOptions {
                    opt: Some(arm.opt),
                    disable_llvm_passes: arm.disable_llvm_passes,
                    target,
                    kind: OutputKind::Executable,
                },
            )
            .unwrap();
        });
        assert_eq!(run_native(&exe).0, 0, "{}", arm.name);
    }
}

#[test]
fn volatile_atomic_metadata_survives_every_middle_end_stage() {
    for (optimize, profile) in STAGES {
        with_atomic_module(
            "volatile _Atomic int x; int main(void) { x=3; return x++; }",
            Target::LinuxGnu,
            optimize,
            profile,
            |module, target| {
                let instructions = module
                    .functions()
                    .filter_map(|(_, f)| f.body.as_ref())
                    .flat_map(|body| {
                        body.blocks().flat_map(move |block| {
                            body.block_insts(block)
                                .iter()
                                .map(move |&inst| body.inst(inst))
                        })
                    });
                let atomics: Vec<_> = instructions
                    .filter_map(|inst| {
                        if let crate::ir::InstData::Atomic { flags, .. } = inst {
                            Some(flags)
                        } else {
                            None
                        }
                    })
                    .collect();
                let no_atomics: [&crate::ir::InstFlags; 0] = [];
                assert_ne!(atomics, no_atomics);
                assert!(
                    atomics
                        .iter()
                        .all(|flags| flags.contains(crate::ir::InstFlags::VOLATILE))
                );
                if profile == Profile::PostAbi {
                    let mut llvm = String::new();
                    crate::backend::llvm::emit_module(module, target, &mut llvm).unwrap();
                    assert!(llvm.contains("load atomic volatile i32"), "{llvm}");
                    assert!(llvm.contains("store atomic volatile i32"), "{llvm}");
                    assert!(llvm.contains("cmpxchg volatile"), "{llvm}");
                }
            },
        );
    }
}

#[test]
fn atomic_warning_recovery_never_builds_illegal_memory_orders() {
    let source = "int main(void) { _Atomic int x=1; int expected=0; __c11_atomic_load(&x,3); \
                  __c11_atomic_store(&x,2,2); \
                  __c11_atomic_compare_exchange_strong(&x,&expected,3,0,5); \
                  __c11_atomic_compare_exchange_weak(&x,&expected,3,0,4); \
                  __c11_atomic_thread_fence(99); return 0; }";
    for (optimize, profile) in STAGES {
        with_atomic_module(
            source,
            Target::LinuxGnu,
            optimize,
            profile,
            |module, target| {
                if profile == Profile::PostAbi {
                    let mut llvm = String::new();
                    crate::backend::llvm::emit_module(module, target, &mut llvm).unwrap();
                }
            },
        );
    }
}
