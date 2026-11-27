#!/usr/bin/env bash
# The launcher inside the plain Nightly tarball (packaging/linux/nightly-launch.sh): craft-common-actions'
# template rendered for TextureCraft. Every argument follows the profile tokens, the profile root is the
# package identity under the XDG data home (or HOME), and a version query is asked in the Launcher's
# own form so the GUI answers before any profile is read.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/tree"

# An argv recorder stands in for the GUI. It never opens a window.
cat >"$work/tree/texturecraft-bin" <<'RECORDER'
#!/bin/sh
printf '<%s>\n' "$@"
RECORDER
chmod 755 "$work/tree/texturecraft-bin"
cp "$HERE/nightly-launch.sh" "$work/tree/texturecraft"
chmod 755 "$work/tree/texturecraft"

# The bytes are the shared template rendered for this app, not a local script.
sha256sum "$HERE/nightly-launch.sh" | cut -d' ' -f1 | grep -qx \
  8941ee4947ef4c3fc4a663600569d985b41c0b9ce6f9833d0c6777aa2b4d6acf || {
  echo "FAIL: nightly-launch.sh is not the shared template rendered for texturecraft" >&2
  exit 1
}
head -n 1 "$work/tree/texturecraft" | grep -qx '#!/bin/sh' || { echo "FAIL: texturecraft is not a script" >&2; exit 1; }

# A hand-unpacked tar has to start the private profile even when nobody passes a flag.
export HOME="$work/home with space"
unset XDG_DATA_HOME
identity="ai.storyteller.texturecraft.nightly"

expect() {
  local root="$1"
  shift
  expected="$(printf '<%s>\n' --launch-profile nightly-v1 --profile-root "$root" -- "$@")"
  actual=$("$work/tree/texturecraft" "$@")
  [[ "$actual" == "$expected" ]] || {
    printf 'profile root %s\nexpected:\n%s\nactual:\n%s\n' "$root" "$expected" "$actual" >&2
    exit 1
  }
}

expect_query() {
  local root="$1"
  expected="$(printf '<%s>\n' --launch-profile nightly-v1 --profile-root "$root" --version)"
  actual=$("$work/tree/texturecraft" --version)
  [[ "$actual" == "$expected" ]] || {
    printf 'expected:\n%s\nactual:\n%s\n' "$expected" "$actual" >&2
    exit 1
  }
  actual=$("$work/tree/texturecraft" -V)
  [[ "$actual" == "$expected" ]] || {
    printf 'expected:\n%s\nactual:\n%s\n' "$expected" "$actual" >&2
    exit 1
  }
}

expect "$HOME/.local/share/$identity"
expect "$HOME/.local/share/$identity" 'model with spaces.glb' '--other-name.tcraft'
export XDG_DATA_HOME="$work/xdg with space"
expect "$XDG_DATA_HOME/$identity" 'model.glb'
expect_query "$XDG_DATA_HOME/$identity"
unset XDG_DATA_HOME
expect_query "$HOME/.local/share/$identity"

# The ArtCraft Launcher names its own profile and per-install root: the argv goes through unchanged.
actual=$("$work/tree/texturecraft" --launch-profile nightly-v1 --profile-root "$work/launcher profile with space" -- file.doc)
[[ "$actual" == "$(printf '<%s>\n' --launch-profile nightly-v1 --profile-root "$work/launcher profile with space" -- file.doc)" ]] || {
  printf 'expected the Launcher argv unchanged\nactual:\n%s\n' "$actual" >&2
  exit 1
}

# The launcher itself writes nothing: the profile folder is the GUI's to create.
[[ ! -e "$HOME/.local/share/$identity" ]] || { echo "FAIL: the launcher created the profile" >&2; exit 1; }

echo "PASS: the tarball launcher is the shared template and starts the GUI through the channel's profile"
