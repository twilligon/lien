#!/bin/sh
set -eu

tier1="
    aarch64-apple-darwin
    aarch64-unknown-linux-gnu
    i686-pc-windows-msvc
    i686-unknown-linux-gnu
    x86_64-pc-windows-msvc
    x86_64-unknown-freebsd
    x86_64-unknown-linux-gnu
"

tier3="
    aarch64-unknown-fuchsia
    wasm32-wasip1-threads
    x86_64-unknown-dragonfly
    x86_64-unknown-netbsd
    x86_64-unknown-openbsd
    x86_64-unknown-redox
"

rustup target add ${tier1}
cargo +nightly check $(printf -- '--target %s ' ${tier1})
cargo +nightly check -Zbuild-std=core --no-default-features $(printf -- '--target %s ' ${tier3})
