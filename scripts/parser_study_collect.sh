#!/bin/sh
# Run after parser_study_build.sh has copied the required executables.
# No Cargo operations here; all compilation belongs to the shared wrapper.
set -eu
study_dir=.scratch/parser-study
case "${1:-native}" in
    native)
        for profile in release debug; do
            binary="$study_dir/study-$profile"
            baseline="$study_dir/baseline-final-$profile"
            test -x "$binary"
            test -x "$baseline"
            python3 scripts/parser_study.py bench --binary "$binary" --baseline-binary "$baseline" \
                --modes baseline machine recursive growing hybrid:16 limited:256 \
                --output "$study_dir/final-$profile.json"
            python3 scripts/parser_study.py stack --binary "$binary" --stack 1048576 \
                --output "$study_dir/final-$profile-stack-1m.json"
            python3 scripts/parser_study.py stack --binary "$binary" --stack 1048576 \
                --files parentheses-63.c blocks-126.c declarator-63.c c99-floors.c \
                --modes machine recursive growing hybrid:16 limited:64 limited:256 limited:512 \
                --output "$study_dir/final-$profile-floors.json"
            python3 scripts/parser_study.py stack --binary "$binary" --stack 67108864 \
                --files parentheses-256.c parentheses-1024.c parentheses-4096.c parentheses-16384.c \
                blocks-256.c blocks-1024.c blocks-4096.c blocks-16384.c \
                --modes recursive --output "$study_dir/final-$profile-stack-64m.json"
            python3 scripts/parser_study.py bench --binary "$binary" --stack 67108864 \
                --files lua-supported.i expression-mix.c statement-mix.c declaration-mix.c \
                --modes recursive --output "$study_dir/final-$profile-worker.json"
            python3 scripts/parser_study.py bench --binary "$binary" \
                --files parentheses-16384.c statements-16384.c blocks-16384.c declarator-16384.c \
                initializer-16384.c chain-100000.c initializer-list-100000.c \
                --modes machine growing hybrid:16 \
                --output "$study_dir/final-$profile-deep.json"
            python3 scripts/parser_study.py bench --binary "$binary" \
                --files parentheses-16000.c parentheses-16500.c blocks-16000.c blocks-16500.c \
                --modes machine growing hybrid:16 --output "$study_dir/final-$profile-neighbors.json"
            python3 scripts/parser_study.py bench --binary "$binary" --stack 1048576 \
                --files parentheses-4096.c parentheses-16384.c blocks-4096.c blocks-16384.c \
                --modes machine growing hybrid:16 --output "$study_dir/final-$profile-deep-1m.json"
        done
        python3 scripts/parser_study.py diff --binary "$study_dir/study-release" \
            --modes machine recursive growing hybrid:16 --output "$study_dir/final-diff.json"
        python3 scripts/parser_study.py diff --binary "$study_dir/study-release" \
            --inputs "$study_dir/grammar" --manifest "$study_dir/grammar/manifest.json" \
            --modes machine recursive growing hybrid:16 --output "$study_dir/final-grammar.json"
        python3 scripts/parser_study.py diff --binary "$study_dir/study-release" \
            --files expression-mix.c statement-mix.c declaration-mix.c initializer-list-10000.c \
            --modes machine recursive growing hybrid:16 --output "$study_dir/final-mix-diff.json"
        ;;
    alignment)
        python3 scripts/parser_study.py bench --binary "$study_dir/study-release" \
            --alignment-binary "$study_dir/alignment-release" --modes machine alignment \
            --files alignment-cache.c alignment-nested-16.c alignment-nested-128.c alignment-nested-1024.c lua-supported.i declaration-mix.c \
            --output "$study_dir/final-alignment.json"
        python3 scripts/parser_study.py diff --binary "$study_dir/study-release" \
            --alignment-binary "$study_dir/alignment-release" --modes machine alignment \
            --files alignment-cache.c alignment-nested-16.c alignment-nested-128.c lua-supported.i \
            --output "$study_dir/final-alignment-diff.json"
        ;;
    profile)
        python3 scripts/parser_study_profile.py --binary "$study_dir/study-profile" \
            --output "$study_dir/final-profile-release.json"
        python3 scripts/parser_study_profile.py --binary "$study_dir/study-profile-debug" \
            --output "$study_dir/final-profile-debug.json"
        python3 scripts/parser_study_profile.py --binary "$study_dir/baseline-profile" \
            --modes machine --files lua-supported.i --repeats 2 \
            --output "$study_dir/final-profile-baseline.json"
        python3 scripts/parser_study_profile.py --binary "$study_dir/study-profile" \
            --modes machine --files alignment-cache.c alignment-nested-1024.c \
            --output "$study_dir/final-profile-alignment-control.json"
        python3 scripts/parser_study_profile.py --binary "$study_dir/alignment-profile" \
            --modes machine --files alignment-cache.c alignment-nested-1024.c \
            --output "$study_dir/final-profile-alignment.json"
        ;;
    *) echo "usage: $0 native|alignment|profile" >&2; exit 2 ;;
esac
