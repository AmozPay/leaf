#!/bin/sh
# Build and package the Leaf binaries for one target:
#
#   scripts/package-release.sh [TARGET] [OUT_DIR]
#
# TARGET defaults to the host (rustc's host triple); OUT_DIR to ./dist.
# Writes OUT_DIR/leaf-<version>-<target>.tar.gz with three binaries:
#   leaf-mcp         MCP server, read-only (search, cite, metadata, ...)
#   leaf-mcp-editor  the same plus the editor_* tools (feature `editor`)
#   leaf             the command-line interface with `leaf editor ...`
# and prints "<sha256>  <file>" for the archive, the line a plugin pins.
set -eu

cd "$(dirname "$0")/.."
target="${1:-$(rustc -vV | sed -n 's/^host: //p')}"
out="${2:-dist}"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
name="leaf-$version-$target"
stage="$out/$name"

# With --target, cargo puts the binaries under target/<triple>/release.
case "$target" in
  "$(rustc -vV | sed -n 's/^host: //p')") flags=""; bindir="target/release" ;;
  *) flags="--target $target"; bindir="target/$target/release" ;;
esac

rm -rf "$stage"
mkdir -p "$stage"

# Each variant is built on its own: Cargo unifies features across the
# packages of one build, and the read-only server must not get the editor.
# shellcheck disable=SC2086
cargo build --release $flags -p leaf-mcp
cp "$bindir/leaf-mcp" "$stage/leaf-mcp"
# shellcheck disable=SC2086
cargo build --release $flags -p leaf-mcp --features editor
cp "$bindir/leaf-mcp" "$stage/leaf-mcp-editor"
# shellcheck disable=SC2086
cargo build --release $flags -p leaf --features editor
cp "$bindir/leaf" "$stage/leaf"

echo "$version" > "$stage/VERSION"
cp LICENSE "$stage/" 2>/dev/null || true
tar -czf "$out/$name.tar.gz" -C "$out" "$name"
rm -rf "$stage"
cd "$out"
if command -v sha256sum >/dev/null 2>&1; then
  sha256sum "$name.tar.gz"
else
  shasum -a 256 "$name.tar.gz"
fi
