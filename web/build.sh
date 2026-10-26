#!/bin/sh
# Build the static site into web/dist/. Run from the assigned worktree root.
#
# `trunk build --release` compiles web/ (its own Cargo workspace): the
# hydrating page, the engine Worker (which links the engine crate in ../ with
# only its `abstract-machine` feature), and the build-time prerender step.
set -eu
if [ "$(pwd -P)" != /home/agent/workspace/bcc-rust/worktrees/abstract-machine-site ]; then
    echo 'Run sh web/build.sh from /home/agent/workspace/bcc-rust/worktrees/abstract-machine-site.' >&2
    exit 1
fi
command -v trunk >/dev/null || {
    echo 'Trunk is required to build the Leptos site: https://trunkrs.dev/guide/getting-started/installation.html' >&2
    exit 1
}
cd web
CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-3}" nice -n 10 trunk build --release
# Trunk's build lock is not part of the site.
rm -f dist/.trunk-lock
echo 'Built the static site in web/dist/.'
