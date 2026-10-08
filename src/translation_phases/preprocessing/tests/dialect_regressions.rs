//! Regressions for standard- and dialect-gated preprocessing found in review.

use std::{
    sync::mpsc,
    time::Duration,
};

use super::{
    language_modes::{
        mode,
        observe_paths,
        spellings,
    },
    *,
};

/// Runs `observe_paths` on another thread and fails, rather than hanging the
/// suite, when preprocessing does not finish.
fn observe_terminating(
    source: &'static str,
    config: CompilerConfiguration,
    directories: Vec<PathBuf>,
) -> (Vec<String>, Vec<String>) {
    let (sender, receiver) = mpsc::channel();
    drop(std::thread::spawn(move || {
        drop(sender.send(observe_paths(
            source,
            config,
            PathBuf::from("<test>"),
            &directories,
        )));
    }));
    receiver
        .recv_timeout(Duration::from_secs(30))
        .unwrap_or_else(|_| panic!("preprocessing did not terminate: {source}"))
}

fn language_fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/language")
}

/// C23 §6.10.4.2p1: a limit is evaluated as a constant expression where it
/// appears, even when the query comes from a macro's replacement list.
#[test]
fn macro_sourced_embed_limit_evaluates_in_its_own_frame() {
    for source in [
        "#define HE __has_embed(<embed.bin> limit(1))\n#if HE\nyes\n#else\nno\n#endif\nafter\n",
        "#define ONE 1\n#define HE __has_embed(<embed.bin> limit(ONE))\n#if HE == \
         __STDC_EMBED_FOUND__\nyes\n#endif\nafter\n",
        "#define L limit(0)\n#if __has_embed(<embed.bin> L) == \
         __STDC_EMBED_EMPTY__\nyes\n#endif\nafter\n",
    ] {
        let (tokens, errors) =
            observe_terminating(source, mode(CStandard::C23), vec![language_fixtures()]);
        assert!(errors.is_empty(), "{source}: {errors:?}");
        let tokens = spellings(&tokens);
        assert!(tokens.contains("identifier `yes`"), "{source}: {tokens}");
        assert!(!tokens.contains("identifier `no`"), "{source}: {tokens}");
        assert_eq!(
            tokens.matches("identifier `after`").count(),
            1,
            "{source}: {tokens}"
        );
    }
}
