#!/bin/sh
# Invoke only through the shared swarm wrapper, from this worktree root.
set -eu
study_dir=.scratch/parser-study
study_feature=parser-recursive-descent-study
mkdir -p "$study_dir"
case "${1:-native}" in
    native)
        cargo clippy --all-targets --features "$study_feature" -- -D warnings
        cargo build --release --features "$study_feature" --example parser_study
        cp "$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/examples/parser_study" "$study_dir/study-release"
        cargo build --features "$study_feature" --example parser_study
        cp "$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/debug/examples/parser_study" "$study_dir/study-debug"
        ;;
    reserved)
        cargo clippy --all-targets --features "$study_feature" -- -D warnings
        cargo test --features "$study_feature" --example parser_study -- --test-threads=1
        cargo build --release --features "$study_feature" --example parser_study
        cp "$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/examples/parser_study" "$study_dir/reserved-release"
        cargo build --features "$study_feature" --example parser_study
        cp "$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/debug/examples/parser_study" "$study_dir/reserved-debug"
        cargo build --release --features benchmarking-internals --example parser_study
        cp "$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/examples/parser_study" "$study_dir/reserved-baseline-release"
        cargo build --features benchmarking-internals --example parser_study
        cp "$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/debug/examples/parser_study" "$study_dir/reserved-baseline-debug"
        ;;
    profile)
        cargo build --release --features "$study_feature,parser-study-small-regions" --example parser_study
        cp "$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/examples/parser_study" "$study_dir/study-profile"
        cargo build --features "$study_feature,parser-study-small-regions" --example parser_study
        cp "$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/debug/examples/parser_study" "$study_dir/study-profile-debug"
        cargo build --release --features benchmarking-internals,parser-study-small-regions --example parser_study
        cp "$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/examples/parser_study" "$study_dir/baseline-profile"
        ;;
    alignment)
        cargo build --release --features "$study_feature,parser-study-leak-alignment" --example parser_study
        cp "$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/examples/parser_study" "$study_dir/alignment-release"
        cargo build --release --features "$study_feature,parser-study-leak-alignment,parser-study-small-regions" --example parser_study
        cp "$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/examples/parser_study" "$study_dir/alignment-profile"
        ;;
    baseline)
        cargo clippy --all-targets --features benchmarking-internals -- -D warnings
        cargo build --release --features benchmarking-internals --example parser_study
        cp "$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/examples/parser_study" "$study_dir/baseline-final-release"
        cargo build --features benchmarking-internals --example parser_study
        cp "$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/debug/examples/parser_study" "$study_dir/baseline-final-debug"
        ;;
    checks)
        cargo test --all-targets
        cargo test --features benchmarking-internals --test allocation_count
        cargo clippy --all-targets -- -D warnings
        cargo clippy --all-targets --features benchmarking-internals -- -D warnings
        cargo clippy --all-targets --features "$study_feature,parser-study-leak-alignment,parser-study-small-regions" -- -D warnings
        cargo test --features "$study_feature" --lib recursive_descent -- --test-threads=1
        ;;
    *) echo "usage: $0 native|reserved|profile|alignment|baseline|checks" >&2; exit 2 ;;
esac
git rev-parse HEAD > "$study_dir/${1:-native}-revision"
sha256sum "$study_dir/"*-release "$study_dir/"*-debug "$study_dir/"*-profile 2>/dev/null || true
