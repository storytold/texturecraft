#!/usr/bin/env bash
# The Nightly checks that need no compiler, macOS or network: the synthetic suites (release
# contracts, the macOS plist and signing helpers, the icons), the launcher tests, and `bash -n`
# over every Nightly shell script. Run by .github/workflows/packaging-lint.yml.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
cd "$ROOT"

python3 packaging/nightly-tests.py
bash packaging/linux/nightly-arch-tar-test.sh
bash packaging/linux/nightly-launch-test.sh

for script in \
  packaging/nightly-common.sh \
  packaging/linux/nightly.sh \
  packaging/linux/nightly-launch.sh \
  packaging/linux/nightly-launch-test.sh \
  packaging/linux/nightly-arch-launch.sh \
  packaging/linux/nightly-arch-tar.sh \
  packaging/linux/nightly-arch-tar-test.sh \
  packaging/linux/package.sh \
  packaging/linux/flatpak-bundle.sh \
  packaging/linux/apprun-test.sh \
  packaging/macos/nightly.sh \
  packaging/macos/nightly-lib.sh \
  packaging/macos/nightly-sign.sh \
  packaging/macos/nightly-verify.sh; do
  bash -n "$script"
done

echo "Nightly checks passed"
