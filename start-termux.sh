#!/data/data/com.termux/files/usr/bin/bash
set -euo pipefail

if [ ! -x target/release/multibot-26-2 ]; then
  cargo build --release
fi
exec ./target/release/multibot-26-2

