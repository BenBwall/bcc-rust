#!/bin/sh
# Build the static site into web/dist/. Run from the repository root.
#
# First the Monaco editor bundle and web fonts (web/editor, with npm), then
# `trunk build --release`, which compiles web/ (its own Cargo workspace): the
# hydrating page, the engine Worker (which links the engine crate in ../ with
# only its `abstract-machine` feature), and the build-time prerender step.
set -eu
cd "$(dirname "$0")"
command -v trunk >/dev/null || {
    echo 'Trunk is required to build the Leptos site: https://trunkrs.dev/guide/getting-started/installation.html' >&2
    exit 1
}
command -v npm >/dev/null || {
    echo 'Node.js and npm are required to bundle the Monaco editor.' >&2
    exit 1
}
(
    cd editor
    npm ci --no-audit --no-fund
    node build.mjs
)
CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-3}" nice -n 10 trunk build --release
# Trunk's build lock is not part of the site.
rm -f dist/.trunk-lock
echo 'Built the static site in web/dist/.'
