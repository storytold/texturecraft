#!/usr/bin/env bash
# Repackage a TextureCraft Linux tarball as a single-file Flatpak bundle:
#
#   $DIST/texturecraft-<version>-linux-<arch>.flatpak
#
# Usage: packaging/linux/flatpak-bundle.sh [--no-test] [TARBALL]
#
# TARBALL defaults to $DIST/texturecraft-<version>-linux-<arch>.tar.gz from package.sh. The bundle
# is built for the host architecture (x86_64 or aarch64), which must match the tarball's.
# Needs flatpak, flatpak-builder and the SVG pixbuf loader (librsvg2-common) for the host's
# `appstreamcli compose`; the freedesktop runtime and SDK named in the manifest are
# installed per-user from Flathub. Unless --no-test, the bundle is then installed per-user and the
# app (`texturecraft --version`) is run inside the sandbox as a smoke test.
# Manifest: packaging/linux/flatpak/ai.storyteller.TextureCraft.bundle.yml.
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/linux"
APP_ID=ai.storyteller.texturecraft
FLATPAK_ID="$APP_ID"
# The remote indexes both app IDs on the stable ref branch; IDs isolate installations.
FLATPAK_BRANCH=stable
if [[ "${TEXTURECRAFT_BUILD_CHANNEL:-}" == nightly-v1 || "$VERSION" == *-nightly* ]]; then
  FLATPAK_ID="$APP_ID.nightly"
fi
FLATHUB=https://dl.flathub.org/repo/flathub.flatpakrepo
# The OCI repository serves only the latest stable release, so a prerelease bundle gets no update
# source: Flatpak would otherwise replace it with that older stable build.
OCI_REPO=oci+https://getartcraft.com
if [[ "$VERSION" == *-* ]]; then OCI_REPO=""; fi

TEST=1
OCI=0
TARBALL=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-test) TEST=0; shift ;;
    --oci) OCI=1; shift ;;
    -h | --help) sed -n '2,14p' "$0"; exit 0 ;;
    -*) echo "unknown argument: $1" >&2; exit 2 ;;
    *) TARBALL="$1"; shift ;;
  esac
done

ARCH="$(uname -m)"
case "$ARCH" in
  x86_64) ;;
  aarch64 | arm64) ARCH=aarch64 ;;
  *) echo "unsupported architecture $ARCH" >&2; exit 2 ;;
esac
BASENAME="texturecraft-$VERSION-linux-$ARCH"
TARBALL="${TARBALL:-$DIST/$BASENAME.tar.gz}"
[[ -f "$TARBALL" ]] || { echo "error: $TARBALL not found (run packaging/linux/package.sh --formats tar first)" >&2; exit 1; }
for tool in flatpak flatpak-builder; do
  command -v "$tool" >/dev/null || { echo "error: $tool not found (apt install flatpak flatpak-builder librsvg2-common)" >&2; exit 1; }
done

echo "==> TextureCraft $VERSION Flatpak bundle for $ARCH from $(basename "$TARBALL")"

WORK="$CARGO_TARGET_DIR/flatpak-bundle"
rm -rf "$WORK"
mkdir -p "$WORK/stage"
tar -xzf "$TARBALL" -C "$WORK/stage" --strip-components=1
[[ -x "$WORK/stage/bin/texturecraft" ]] || { echo "error: $TARBALL has no bin/texturecraft" >&2; exit 1; }
if [[ "$FLATPAK_ID" != "$APP_ID" ]]; then
  # A Nightly tarball from package.sh already separates the launcher from the GUI; an older one
  # does not. Either way this bundle installs its own wrapper and the GUI in /app/libexec.
  if [[ -e "$WORK/stage/bin/texturecraft-bin" ]]; then
    rm -f "$WORK/stage/bin/texturecraft"
  else
    mv "$WORK/stage/bin/texturecraft" "$WORK/stage/bin/texturecraft-bin"
  fi
  cat >"$WORK/stage/bin/texturecraft" <<'WRAPPER'
#!/bin/sh
# TextureCraft Nightly: the GUI runs with the Launcher's private profile. Inside the sandbox
# $XDG_DATA_HOME is the app's own per-app folder, so the profile never meets stable's.
set -eu
bin=/app/libexec/texturecraft
profile="${XDG_DATA_HOME:-${HOME:-}/.local/share}/TextureCraft-Nightly"
# The Launcher's protocol, and a bare version query, pass through untouched.
case "${1-}" in
  --launch-profile | --profile-root | --version | -V) exec "$bin" "$@" ;;
esac
exec "$bin" --launch-profile nightly-v1 --profile-root "$profile" -- "$@"
WRAPPER
  chmod 755 "$WORK/stage/bin/texturecraft"
  sed -e "s|^Icon=$APP_ID$|Icon=$FLATPAK_ID|" \
      -e "s|^Name=TextureCraft$|Name=TextureCraft Nightly|" \
      -e 's|^StartupWMClass=.*|StartupWMClass=ai.storyteller.texturecraft.nightly-v1|' \
      -e '/^MimeType=/d' \
      "$WORK/stage/share/applications/$APP_ID.desktop" >"$WORK/stage/share/applications/$FLATPAK_ID.desktop"
  rm -f "$WORK/stage/share/applications/$APP_ID.desktop"
  # The Nightly entry drops MimeType=, so the metainfo must not advertise the types either.
  sed -e "s|<id>$APP_ID</id>|<id>$FLATPAK_ID</id>|g" \
      -e "s|<name>TextureCraft</name>|<name>TextureCraft Nightly</name>|g" \
      -e "s|<launchable type=\"desktop-id\">$APP_ID.desktop</launchable>|<launchable type=\"desktop-id\">$FLATPAK_ID.desktop</launchable>|g" \
      -e "s|<icon type=\"stock\">$APP_ID</icon>|<icon type=\"stock\">$FLATPAK_ID</icon>|g" \
      -e '/<mediatype>/d' \
      "$WORK/stage/share/metainfo/$APP_ID.metainfo.xml" >"$WORK/stage/share/metainfo/$FLATPAK_ID.metainfo.xml"
  rm -f "$WORK/stage/share/metainfo/$APP_ID.metainfo.xml"
  rm -rf "$WORK/stage/share/mime"
  rm -rf "$WORK/stage/share/icons/hicolor"
  NIGHTLY_ICON="$ROOT/assets/app-icon/texturecraft-nightly.png"
  if [[ ! -f "$NIGHTLY_ICON" ]]; then
    NIGHTLY_ICON="$ROOT/assets/app-icon/hicolor/256x256/apps/$APP_ID.png"
  fi
  install -Dm644 "$NIGHTLY_ICON" \
    "$WORK/stage/share/icons/hicolor/256x256/apps/$FLATPAK_ID.png"
  # Omit conflicting MIME registrations and disable host Flatpak updater talk-name until
  # craft_update supports the nightly FLATPAK_ID. A Nightly bundle is also GUI-only: its tarball has
  # no texturecraft-cli, and a CLI would read Stable's Documents/TextureCraft brushes.
  sed -e "s|ai\.storyteller\.texturecraft|$FLATPAK_ID|g" \
      -e '/share\/mime\/packages\//d' \
      -e '/^\( *\)# \.tcraft, plus mesh types/d' \
      -e '/bin\/texturecraft-cli/d' \
      -e '/--talk-name=org\.freedesktop\.Flatpak/d' \
      -e 's|- install -Dm755 bin/texturecraft /app/bin/texturecraft|- install -Dm755 bin/texturecraft-bin /app/libexec/texturecraft\n      - install -Dm755 bin/texturecraft /app/bin/texturecraft|' \
      "$HERE/flatpak/ai.storyteller.TextureCraft.bundle.yml" >"$WORK/$FLATPAK_ID.yml"
else
  cp "$HERE/flatpak/ai.storyteller.TextureCraft.bundle.yml" "$WORK/$FLATPAK_ID.yml"
fi

flatpak remote-add --user --if-not-exists flathub "$FLATHUB"
# --disable-rofiles-fuse: no FUSE in containers and CI runners.
flatpak-builder --user --install-deps-from=flathub --disable-rofiles-fuse --force-clean \
  --default-branch="$FLATPAK_BRANCH" --repo="$WORK/repo" --state-dir="$WORK/state" "$WORK/build" "$WORK/$FLATPAK_ID.yml"

OUT="$DIST/$BASENAME.flatpak"
# A release build (the shared release workflow sets SIGNING_FPR, SIGNING_PUBKEY and GNUPGHOME)
# signs the commit and embeds the public key, so Flatpak verifies the bundle when installing it.
GPG_KEYS=()
if [[ -n "${SIGNING_FPR:-}" ]]; then
  flatpak build-sign "$WORK/repo" "$FLATPAK_ID" "$FLATPAK_BRANCH" --gpg-sign="$SIGNING_FPR" --gpg-homedir="${GNUPGHOME:?}"
  GPG_KEYS=(--gpg-keys="${SIGNING_PUBKEY:?}")
  if [[ "${OCI_SIGNATURES_READY:-}" != 1 ]]; then OCI_REPO=""; fi
fi
# --runtime-repo lets `flatpak install` fetch the freedesktop runtime from Flathub if missing.
# --repo-url makes the installed origin update from the ArtCraft OCI repository.
flatpak build-bundle --runtime-repo="$FLATHUB" ${OCI_REPO:+--repo-url="$OCI_REPO"} "${GPG_KEYS[@]}" "$WORK/repo" "$OUT" "$FLATPAK_ID" "$FLATPAK_BRANCH"
echo "wrote $OUT"
if [[ "$OCI" = 1 ]]; then
  OUT_OCI="$DIST/$BASENAME-oci"
  rm -rf "$OUT_OCI"
  flatpak build-bundle --oci "$WORK/repo" "$OUT_OCI" "$FLATPAK_ID" "$FLATPAK_BRANCH"
  echo "wrote OCI layout to $OUT_OCI"
fi

if [[ "$TEST" = 1 ]]; then
  flatpak install --user -y --noninteractive --reinstall "$OUT"
  flatpak info --user "$FLATPAK_ID"
  flatpak run --command=texturecraft "$FLATPAK_ID" --version
  if [[ "$FLATPAK_ID" == "$APP_ID" ]]; then
    flatpak run --command=sh "$FLATPAK_ID" -c 'ls /app/share/applications /app/share/metainfo /app/share/mime/packages /app/share/icons/hicolor/*/apps'
  else
    flatpak run --command=sh "$FLATPAK_ID" -c 'ls /app/share/applications /app/share/metainfo /app/share/icons/hicolor/*/apps && test ! -e /app/share/mime/packages'
  fi
fi
echo "==> done"
ls -lh "$OUT"
