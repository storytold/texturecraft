#!/bin/sh
# The entry point inside the Nightly tarball (packaging/linux/nightly.sh installs it as
# `texturecraft`, beside the GUI as `texturecraft-bin`).
#
# A Nightly build only starts with its private profile, and every package format passes the
# Launcher's literal protocol: `--launch-profile nightly-v1 --profile-root <dir> -- <files>`. The
# profile sits outside stable's folders (its own settings, ui.json, crash log, autosaves, smart
# materials, layouts, templates and presets) and outside this unpacked copy, which the app's own
# fence would refuse.
set -eu

here="$(dirname "$(readlink -f "$0")")"
bin="$here/texturecraft-bin"
profile="${XDG_DATA_HOME:-${HOME:-}/.local/share}/TextureCraft-Nightly"

# The Launcher's protocol, and a bare version query, pass through untouched: the first already
# names its profile, and the second must answer without one.
case "${1-}" in
  --launch-profile | --profile-root | --version | -V) exec "$bin" "$@" ;;
esac
exec "$bin" --launch-profile nightly-v1 --profile-root "$profile" -- "$@"
