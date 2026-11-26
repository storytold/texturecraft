#!/bin/sh
# The TextureCraft Nightly launcher for the Arch handoff (packaging/linux/nightly-arch-tar.sh):
# the real GUI stays in .texturecraft-nightly/, and this starts it with the Launcher's private
# profile, as every other Nightly package does.
set -eu

bin="$(dirname "$0")/.texturecraft-nightly/texturecraft"
profile="${XDG_DATA_HOME:-${HOME:-}/.local/share}/TextureCraft-Nightly"

# The Launcher's protocol, and a bare version query, pass through untouched.
case "${1-}" in
  --launch-profile | --profile-root | --version | -V) exec "$bin" "$@" ;;
esac
exec "$bin" --launch-profile nightly-v1 --profile-root "$profile" -- "$@"
