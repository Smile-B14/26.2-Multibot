#!/data/data/com.termux/files/usr/bin/bash
set -euo pipefail

if [ ! -x target/release/multibot-26-2 ]; then
  RUSTUP_TOOLCHAIN=nightly-termux RUSTC_BOOTSTRAP=1 cargo build --release
fi
exec ./target/release/multibot-26-2

