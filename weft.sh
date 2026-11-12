#!/bin/sh
# Start weft on Linux (or any POSIX desktop): run ./weft.sh in a terminal, or
# double-click weft.desktop / this file in a file manager. weft runs in its
# own session, so closing the terminal doesn't close it. When cargo is
# installed, the release build is brought up to date first.

cd "$(dirname "$0")" || exit 1

if command -v cargo >/dev/null 2>&1; then
    if ! cargo build --release; then
        echo "weft: the build failed, see above." >&2
        exit 1
    fi
fi

exe=./target/release/weft
if [ ! -x "$exe" ]; then
    echo "weft: $exe not found. Install Rust from https://rustup.rs and run this again." >&2
    exit 1
fi

# setsid: a new session, so the terminal's hangup doesn't reach weft.
if command -v setsid >/dev/null 2>&1; then
    setsid "$exe" </dev/null >/dev/null 2>&1 &
else
    nohup "$exe" </dev/null >/dev/null 2>&1 &
fi
