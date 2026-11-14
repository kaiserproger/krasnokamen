#!/bin/sh
# RUSTC_WRAPPER for mem-run builds. The file stem must stay "sccache": cc-rs then also uses
# this wrapper for C/C++ compiles in build scripts (aws-lc-sys, zstd-sys); any other name
# leaves them uncached.
# sccache 0.18 hashes every CARGO_* environment variable into Rust cache keys except
# CARGO_MAKEFLAGS, CARGO_REGISTRIES_*, CARGO_BUILD_JOBS and CARGO_ENCODED_RUSTFLAGS
# (src/compiler/rust.rs:1576-1588), and refuses to run with CARGO_INCREMENTAL=1 in its
# environment. mem-run exports a per-slot CARGO_TARGET_DIR (a different value per slot gave
# 0 Rust hits, verified 2026-11-11) and CARGO_INCREMENTAL. Both are cargo settings: cargo
# passes the resulting output paths and -C incremental to rustc as arguments.
exec env -u CARGO_TARGET_DIR -u CARGO_INCREMENTAL "$HOME/.cargo/bin/sccache" "$@"
