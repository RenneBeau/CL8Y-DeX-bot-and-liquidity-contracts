#!/usr/bin/env bash
set -euo pipefail

RUSTSEC_REV=${RUSTSEC_REV:-3889e79597f5a8f42b6b8c2fe2db521d0f255991}
CARGO_DENY_REV=${CARGO_DENY_REV:-bca0dde53651ee946720e4540b5ce2610bec8f06}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

git clone --quiet --recurse-submodules https://github.com/rustsec/rustsec.git "$work/rustsec"
git -C "$work/rustsec" checkout --quiet "$RUSTSEC_REV"
git -C "$work/rustsec" submodule update --quiet --init --recursive
cargo +stable install --path "$work/rustsec/cargo-audit" --locked --force

git clone --quiet --recurse-submodules https://github.com/EmbarkStudios/cargo-deny.git "$work/cargo-deny"
git -C "$work/cargo-deny" checkout --quiet "$CARGO_DENY_REV"
git -C "$work/cargo-deny" submodule update --quiet --init --recursive
cargo +stable install --path "$work/cargo-deny" --locked --force
