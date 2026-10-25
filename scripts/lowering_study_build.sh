#!/bin/sh
# Invoke through the shared wrapper from the worktree root. Copy executables
# before releasing the slot because other jobs may clean the shared target.
set -eu
study_dir=.scratch/lowering-study
mkdir -p "$study_dir"
study_features=recursive-lowering-study,benchmarking-internals
case "${1:-build}" in
    build)
        cargo clippy --all-targets --features "$study_features" -- -D warnings
        cargo build --release --features "$study_features" --example lowering_study
        cp "$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/examples/lowering_study" "$study_dir/study-release"
        cargo build --features "$study_features" --example lowering_study
        cp "$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/debug/examples/lowering_study" "$study_dir/study-debug"
        cargo test --release --features "$study_features" --example lowering_study -- --include-ignored --nocapture --test-threads=1
        cargo test --features "$study_features" --example lowering_study -- --include-ignored --nocapture --test-threads=1
        cargo build --features recursive-lowering-study --bin bcc-rust
        cp "$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/debug/bcc-rust" "$study_dir/compiler-debug"
        ;;
    checks)
        cargo test --all-targets
        cargo test --features benchmarking-internals --test allocation_count
        cargo clippy --all-targets -- -D warnings
        cargo clippy --all-targets --features benchmarking-internals -- -D warnings
        cargo clippy --all-targets --features recursive-lowering-study -- -D warnings
        cargo test --features recursive-lowering-study,benchmarking-internals --lib -- --test-threads=2
        cargo test --example parser_stack_probe -- --test-threads=1
        ;;
    *) echo "usage: $0 build|checks" >&2; exit 2 ;;
esac
git rev-parse HEAD > "$study_dir/${1:-build}-revision"
