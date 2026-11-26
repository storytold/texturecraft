#!/usr/bin/env python3
"""Run the synthetic Nightly packaging suites together; no app build, signing or network.

Suites that a checkout does not carry are skipped with a note: the Linux archive suite belongs to
whoever owns `nightly-archive.py`, and this branch does not add one.
"""
import importlib.util
from pathlib import Path
import sys
import unittest


def load(here, name, relative, suite):
    path = here / relative
    if not path.is_file():
        print(f"skip: {relative} is not in this checkout", file=sys.stderr)
        return
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    suite.addTests(unittest.defaultTestLoader.loadTestsFromModule(module))


def icons_match(here) -> bool:
    """The committed Nightly icons must be exactly what the generator derives from the masters."""
    import subprocess
    result = subprocess.run([sys.executable, str(here / "nightly-icons.py"), "--check"], capture_output=True, text=True)
    print(result.stdout.strip(), file=sys.stderr)
    if result.returncode != 0:
        print(result.stderr.strip(), file=sys.stderr)
    return result.returncode == 0


def main() -> int:
    here = Path(__file__).resolve().parent
    suite = unittest.TestSuite()
    load(here, "texturecraft_archive_suite", "nightly-archive-tests.py", suite)
    load(here, "texturecraft_plist_suite", "macos/nightly_plist_tests.py", suite)
    load(here, "texturecraft_release_dispatch_suite", "nightly-release-tests.py", suite)
    load(here, "texturecraft_mac_sign_suite", "macos/nightly_sign_tests.py", suite)
    passed = unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful()
    return 0 if passed and icons_match(here) else 1


if __name__ == "__main__":
    raise SystemExit(main())
