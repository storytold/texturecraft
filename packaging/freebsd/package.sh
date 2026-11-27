#!/usr/bin/env bash
# Build and package TextureCraft for FreeBSD. The archive's <arch> is x86_64 or aarch64.
#
#   $DIST/texturecraft-<version>-freebsd-<arch>.tar.gz   a /usr/local-style tree:
#       texturecraft-<version>-freebsd-<arch>/{bin, share/applications, share/icons, share/mime,
#       share/metainfo, share/doc/texturecraft}
#
# Install by copying the tree's contents into /usr/local:
#   tar -xzf texturecraft-<version>-freebsd-<arch>.tar.gz --strip-components 1 -C /usr/local
#
# Usage: packaging/freebsd/package.sh [--skip-build] [--dry-run]
#   --dry-run   stage the tree from stub binaries and list it, without building (works on any OS)
#
# Needs: bash, cargo, and the packages installed in .github/workflows/freebsd.yml.
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
LINUX="$ROOT/packaging/linux"
APP_ID=ai.storyteller.texturecraft

SKIP_BUILD=0
DRY_RUN=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    --dry-run) DRY_RUN=1; SKIP_BUILD=1; shift ;;
    -h | --help) sed -n '1d; /^#/!q; p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

# FreeBSD's uname says amd64; release file names use x86_64 like the Linux and Windows ones.
MACHINE="$(uname -m)"
case "$MACHINE" in
  amd64 | x86_64) HOST_ARCH=x86_64 ;;
  arm64 | aarch64) HOST_ARCH=aarch64 ;;
  *) echo "unsupported architecture $MACHINE" >&2; exit 2 ;;
esac
ARCH="$HOST_ARCH"
if [[ -n "${CARGO_BUILD_TARGET:-}" ]]; then
  case "$CARGO_BUILD_TARGET" in
    aarch64-unknown-freebsd) ARCH=aarch64 ;;
    x86_64-unknown-freebsd) ARCH=x86_64 ;;
    *) echo "error: unsupported FreeBSD target $CARGO_BUILD_TARGET" >&2; exit 2 ;;
  esac
fi
if [[ "$DRY_RUN" = 0 ]] && [[ "$(uname -s)" != FreeBSD ]]; then
  echo "error: build on FreeBSD (or pass --dry-run to check the tree layout)" >&2
  exit 2
fi
BASENAME="texturecraft-$VERSION-freebsd-$ARCH"

echo "==> TextureCraft $VERSION for FreeBSD $ARCH"

# The release VM has 12 GB; full parallelism on the biggest crates runs it out of memory.
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
# The man-and-completions example writes texturecraft-cli's man pages and shell completions below.
# It is built in its own cargo run: built with the binaries, its dev-dependencies' features would
# be unified into them.
if [[ "$SKIP_BUILD" = 0 ]]; then
  (cd "$ROOT" && cargo build --release --locked -p texturecraft -p texturecraft-cli)
  # The example is run on this host to write man pages and completions, so it must be a host
  # binary even when CARGO_BUILD_TARGET is set for a cross package.
  (cd "$ROOT" && env -u CARGO_BUILD_TARGET cargo build --release --locked -p texturecraft-cli --example man-and-completions)
fi
WORK="$CARGO_TARGET_DIR/freebsd-package"
STAGE="$WORK/$BASENAME"
rm -rf "$WORK"
mkdir -p "$WORK"

BIN="$CARGO_TARGET_DIR/${CARGO_BUILD_TARGET:+$CARGO_BUILD_TARGET/}release"
OUT_DIR="$DIST"
if [[ "$DRY_RUN" = 1 ]]; then
  # Stub binaries, and the tarball stays in the work dir so dist/ only ever holds real builds.
  BIN="$WORK/stub-bin"
  OUT_DIR="$WORK"
  mkdir -p "$BIN"
  for b in texturecraft texturecraft-cli; do
    printf '#!/bin/sh\necho "%s %s (dry-run stub)"\n' "$b" "$VERSION" >"$BIN/$b"
    chmod 755 "$BIN/$b"
  done
  # A stub of the example: it writes the files the real one does.
  mkdir -p "$BIN/examples"
  cat >"$BIN/examples/man-and-completions" <<'STUB'
#!/bin/sh
mkdir -p "$1"
for f in texturecraft-cli.bash _texturecraft-cli texturecraft-cli.fish texturecraft-cli.elv _texturecraft-cli.ps1 texturecraft-cli.1 texturecraft-cli-info.1; do
  echo "dry-run stub" >"$1/$f"
done
STUB
  chmod 755 "$BIN/examples/man-and-completions"
fi

# ---- stage a /usr/local-style tree --------------------------------------------------------------
# FreeBSD's install(1) has no -D: create the directories first.
mkdir -p "$STAGE/bin" "$STAGE/share/applications" "$STAGE/share/mime/packages" \
  "$STAGE/share/metainfo" "$STAGE/share/icons" "$STAGE/share/doc/texturecraft"
install -m 755 "$BIN/texturecraft" "$BIN/texturecraft-cli" "$STAGE/bin/"
strip "$STAGE/bin/texturecraft" "$STAGE/bin/texturecraft-cli" 2>/dev/null || true
# The desktop entry, MIME type, metainfo and icons are the freedesktop files Linux ships.
install -m 644 "$LINUX/$APP_ID.desktop" "$STAGE/share/applications/$APP_ID.desktop"
install -m 644 "$LINUX/$APP_ID.mime.xml" "$STAGE/share/mime/packages/$APP_ID.xml"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@DATE@/$TEXTURECRAFT_BUILD_DATE/g" \
  "$LINUX/$APP_ID.metainfo.xml.in" >"$STAGE/share/metainfo/$APP_ID.metainfo.xml"
cp -R "$ROOT/assets/app-icon/hicolor" "$STAGE/share/icons/"
copy_docs "$STAGE/share/doc/texturecraft"
# texturecraft-cli's man pages and shell completions, from its clap definition, written by the
# example built above (a --skip-build run needs it built too). Under /usr/local, man pages go in
# man/ and the completions in the directories bash, zsh and fish read there.
if [[ "$DRY_RUN" = 1 ]]; then
  CLI_DOCS="$BIN/examples/man-and-completions"
else
  # Host path: the example is built with CARGO_BUILD_TARGET unset, above.
  CLI_DOCS="$CARGO_TARGET_DIR/release/examples/man-and-completions"
fi
if [[ ! -x "$CLI_DOCS" ]]; then
  echo "error: $CLI_DOCS is not built (cargo build --release -p texturecraft-cli --example man-and-completions)" >&2
  exit 1
fi
"$CLI_DOCS" "$WORK/cli-docs"
mkdir -p "$STAGE/man/man1" "$STAGE/share/bash-completion/completions" \
  "$STAGE/share/zsh/site-functions" "$STAGE/share/fish/vendor_completions.d"
install -m 644 "$WORK"/cli-docs/*.1 "$STAGE/man/man1/"
install -m 644 "$WORK/cli-docs/texturecraft-cli.bash" "$STAGE/share/bash-completion/completions/texturecraft-cli"
install -m 644 "$WORK/cli-docs/_texturecraft-cli" "$STAGE/share/zsh/site-functions/_texturecraft-cli"
install -m 644 "$WORK/cli-docs/texturecraft-cli.fish" "$STAGE/share/fish/vendor_completions.d/texturecraft-cli.fish"

for f in bin/texturecraft bin/texturecraft-cli "share/applications/$APP_ID.desktop" \
  "share/icons/hicolor/256x256/apps/$APP_ID.png" "share/icons/hicolor/scalable/apps/$APP_ID.svg" \
  share/doc/texturecraft/LICENSE-MIT share/doc/texturecraft/LICENSE-APACHE \
  man/man1/texturecraft-cli.1 share/bash-completion/completions/texturecraft-cli share/zsh/site-functions/_texturecraft-cli \
  share/fish/vendor_completions.d/texturecraft-cli.fish; do
  if [[ ! -e "$STAGE/$f" ]]; then echo "error: $f is missing from the package" >&2; exit 1; fi
done

# ---- .tar.gz ------------------------------------------------------------------------------------
mkdir -p "$OUT_DIR"
tar -C "$WORK" -czf "$OUT_DIR/$BASENAME.tar.gz" "$BASENAME"
echo "wrote $OUT_DIR/$BASENAME.tar.gz"

if [[ -z "${CARGO_BUILD_TARGET:-}" ]] || [[ "$DRY_RUN" = 1 ]]; then
  "$STAGE/bin/texturecraft-cli" --version
fi
if [[ "$DRY_RUN" = 1 ]]; then
  echo "==> dry run: tarball contents"
  tar -tzvf "$OUT_DIR/$BASENAME.tar.gz"
fi
echo "==> done"
