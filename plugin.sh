#!/bin/sh
here="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
sys="$(uname -s)"
case "$sys" in
MINGW* | MSYS* | CYGWIN*) rel="x86_64-pc-windows-msvc/plugin-dev.exe" ;;
Darwin*)
    case "$(uname -m)" in
    arm64) rel="aarch64-apple-darwin/plugin-dev" ;;
    *) rel="x86_64-apple-darwin/plugin-dev" ;;
    esac ;;
*)
    case "$(uname -m)" in
    aarch64 | arm64) rel="aarch64-unknown-linux-musl/plugin-dev" ;;
    *) rel="x86_64-unknown-linux-musl/plugin-dev" ;;
    esac ;;
esac
if [ -x "$here/$rel" ]; then exec "$here/$rel" "$@"; fi
for cand in "$here/target/release/plugin-dev.exe" "$here/target/release/plugin-dev" "$here/target/debug/plugin-dev.exe" "$here/target/debug/plugin-dev"; do
    if [ -x "$cand" ]; then exec "$cand" "$@"; fi
done
echo "plugin-dev binary not found in $here" >&2
exit 1
