#!/usr/bin/env bash
# Validate the staged Arch launcher without building or opening TextureCraft.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
stage="$work/source"
dst="$work/dist"
base=texturecraft-0.0.1-nightly.1-linux-x86_64
mkdir -p "$stage/bin" "$dst" "$work/extract"

# A local argv recorder stands in for the compiled binaries. It never opens a UI.
cat >"$stage/bin/texturecraft" <<'RECORDER'
#!/bin/sh
printf '<%s>\n' "$@"
RECORDER
chmod 755 "$stage/bin/texturecraft"

bash "$HERE/nightly-arch-tar.sh" "$stage" "$work/stage" "$dst" "$base"
tar -xzf "$dst/$base.tar.gz" -C "$work/extract"
bin="$work/extract/$base/bin"
# Simulate the existing Arch bin-tree copy and public launcher rename.
mkdir -p "$work/installed"
cp -R "$bin" "$work/installed/bin"
bin="$work/installed/bin"
mv "$bin/texturecraft" "$bin/texturecraft-nightly"

test ! -e "$stage/bin/.texturecraft-nightly"
cmp "$stage/bin/texturecraft" "$bin/.texturecraft-nightly/texturecraft"
# A Nightly package is GUI-only: the handoff must not carry a CLI that would use Stable's brushes.
test ! -e "$bin/texturecraft-nightly-cli" || { echo "FAIL: the Arch handoff still ships a Nightly CLI" >&2; exit 1; }
profile="${XDG_DATA_HOME:-${HOME:-}/.local/share}/TextureCraft-Nightly"
# A launch adds the Launcher's literal selector/root pair and the file delimiter; a version query or
# an already-formed protocol passes through untouched.
expect_launch() {
  expected="$(printf '<%s>\n' --launch-profile nightly-v1 --profile-root "$profile" -- "$@")"
  actual=$(PATH="$bin:$PATH" texturecraft-nightly "$@")
  [[ "$actual" == "$expected" ]] || {
    printf 'expected:\n%s\nactual:\n%s\n' "$expected" "$actual" >&2
    exit 1
  }
}
expect_forwarded() {
  expected="$(printf '<%s>\n' "$@")"
  actual=$(PATH="$bin:$PATH" texturecraft-nightly "$@")
  [[ "$actual" == "$expected" ]] || {
    printf 'expected:\n%s\nactual:\n%s\n' "$expected" "$actual" >&2
    exit 1
  }
}
expect_launch
expect_launch 'mesh with spaces.glb' '--other-name.tcraft'
expect_forwarded --version
expect_forwarded -V
expect_forwarded --launch-profile nightly-v1 --profile-root /launcher/profile --version

echo "PASS: Arch Nightly handoff preserves binary layout and passes the Launcher profile"
