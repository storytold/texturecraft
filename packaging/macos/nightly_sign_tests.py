#!/usr/bin/env python3
"""The Nightly signing and archive helpers, exercised on Linux with stand-in Apple tool output.

Nothing here signs, notarizes, unpacks or runs a payload: each helper takes the tool output (or a
crafted archive) as an argument, which is why packaging/macos/nightly_sign_tests.py can run in the
app's ordinary Linux CI.
"""
import os
import stat
import subprocess
import tempfile
import unittest
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
LIB = HERE / "nightly-lib.sh"
APP = "TextureCraft Nightly.app"
IDENTITY = "A" * 40
TEAM = "DJ6XS33FX8"
APP_CONFIG_ID = "ai.storyteller.texturecraft.nightly"



class VersionFromAsset(unittest.TestCase):
    def check(self, name):
        """True when the helper accepts the asset name."""
        result = subprocess.run(
            ["bash", "-c", '. "$LIB"; mac_nightly_version_from_asset "$1"', "bash", name],
            capture_output=True, text=True, env={"PATH": os.environ["PATH"], "LIB": str(LIB)},
        )
        return result.returncode == 0

    def test_accepts_only_the_prepared_nightly_archive_name(self):
        version = "0.24.0-nightly.20261124.7.2.g" + "0123456789ab"
        self.assertTrue(self.check(f"texturecraft-{version}-macos-aarch64-nightly.zip"))
        for name in [
            f"texturecraft-{version}-macos-aarch64.zip",
            f"texturecraft-{version}-linux-x86_64-nightly.tar.gz",
            f"texturecraft-0.24.0-macos-aarch64-nightly.zip",
            f"texturecraft-{version.upper()}-macos-aarch64-nightly.zip",
            f"photocraft-{version}-macos-aarch64-nightly.zip",
            f"texturecraft-{version}-macos-x86_64-nightly.zip",
            f"texturecraft-{version}-macos-aarch64-nightly.zip.sig",
        ]:
            self.assertFalse(self.check(name), name)


class Keychain(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.work = Path(self.directory.name).resolve()
        self.keychain = self.work / "texturecraft-signing.keychain-db"
        self.keychain.write_bytes(b"stand-in keychain")

    def ours(self, keychain, work=None):
        result = subprocess.run(
            ["bash", "-c", '. "$LIB"; mac_nightly_keychain_is_ours "$1" "$2"', "bash", str(keychain), str(work or self.work)],
            capture_output=True, text=True, env={"PATH": os.environ["PATH"], "LIB": str(LIB)},
        )
        return result.returncode == 0

    def test_only_this_runs_own_keychain_counts(self):
        self.assertTrue(self.ours(self.keychain))
        # import-cert.sh may put it in a private directory of its own under the run.
        nested = self.work / "private"
        nested.mkdir()
        self.assertFalse(self.ours(nested / "texturecraft-signing.keychain-db"), "a missing file is not ours")
        (nested / "texturecraft-signing.keychain-db").write_bytes(b"x")
        self.assertTrue(self.ours(nested / "texturecraft-signing.keychain-db"))
        self.assertFalse(self.ours(self.work / "other.keychain-db"))
        with tempfile.TemporaryDirectory() as outside:
            (Path(outside) / "texturecraft-signing.keychain-db").write_bytes(b"x")
            self.assertFalse(self.ours(Path(outside) / "texturecraft-signing.keychain-db"))
        alias = self.work / "texturecraft-signing.keychain-db-alias"
        alias.symlink_to(self.keychain)
        self.assertFalse(self.ours(alias))


class Identity(unittest.TestCase):
    LINE = f'  1) {IDENTITY} "Developer ID Application: Learning Machines Inc ({TEAM})"'

    def matcher(self, line, identity=IDENTITY, team=TEAM):
        result = subprocess.run(
            ["bash", "-c", '. "$LIB"; mac_nightly_identity_is_team "$1" "$2" "$3"', "bash", line, identity, team],
            capture_output=True, text=True, env={"PATH": os.environ["PATH"], "LIB": str(LIB)},
        )
        return result.returncode == 0

    def test_a_developer_id_of_the_team_and_nothing_else(self):
        self.assertTrue(self.matcher(self.LINE))
        self.assertFalse(self.matcher(self.LINE.replace("Developer ID Application", "Apple Development")))
        self.assertFalse(self.matcher(self.LINE.replace(TEAM, "ZZZZZZZZZZ")))
        self.assertFalse(self.matcher(self.LINE, identity="B" * 40))
        self.assertFalse(self.matcher(""))


class Signature(unittest.TestCase):
    DETAILS = "\n".join([
        f"Identifier={APP_CONFIG_ID}",
        "Authority=Developer ID Application: Learning Machines Inc (DJ6XS33FX8)",
        "Authority=Developer ID Certification Authority",
        f"TeamIdentifier={TEAM}",
        "Timestamp=Nov 24, 2026 at 10:00:00",
        "CodeDirectory v=20500 size=1234 flags=0x10000(runtime) hashes=1+7 location=embedded",
    ])

    def check(self, details, team=TEAM):
        result = subprocess.run(
            ["bash", "-c", '. "$LIB"; mac_nightly_check_signature "$1" "$2"', "bash", details, team],
            capture_output=True, text=True, env={"PATH": os.environ["PATH"], "LIB": str(LIB)},
        )
        return result.returncode == 0

    def test_the_required_developer_id_signature(self):
        self.assertTrue(self.check(self.DETAILS))
        for missing in [
            "Timestamp=Nov 24, 2026 at 10:00:00",
            f"TeamIdentifier={TEAM}",
            "Authority=Developer ID Application: Learning Machines Inc (DJ6XS33FX8)",
        ]:
            self.assertFalse(self.check(self.DETAILS.replace(missing, "")), missing)
        self.assertFalse(self.check(self.DETAILS.replace("flags=0x10000(runtime)", "flags=0x0(none)")))
        self.assertFalse(self.check(self.DETAILS, team="ZZZZZZZZZZ"))
        self.assertFalse(self.check(self.DETAILS.replace(APP_CONFIG_ID, "ai.storyteller.texturecraft")))


class Assessment(unittest.TestCase):
    def check(self, text):
        result = subprocess.run(
            ["bash", "-c", '. "$LIB"; mac_nightly_check_assessment "$1"', "bash", text],
            capture_output=True, text=True, env={"PATH": os.environ["PATH"], "LIB": str(LIB)},
        )
        return result.returncode == 0

    def test_gatekeeper_must_report_a_notarized_developer_id(self):
        self.assertTrue(self.check(f"{APP}: accepted\nsource=Notarized Developer ID"))
        self.assertFalse(self.check(f"{APP}: accepted\nsource=Unnotarized Developer ID"))
        self.assertFalse(self.check(f"{APP}: rejected\nsource=Notarized Developer ID"))
        self.assertFalse(self.check(""))


class ZipEntries(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)

    def checked(self, entries):
        """True when mac_nightly_check_zip_entries accepts the archive of (name, mode) entries."""
        archive = self.root / "test.zip"
        with zipfile.ZipFile(archive, "w") as z:
            for name, mode in entries:
                info = zipfile.ZipInfo(name)
                info.external_attr = (mode or stat.S_IFREG | 0o644) << 16 if mode else 0
                z.writestr(info, b"stand-in payload")
        result = subprocess.run(
            ["bash", "-c", '. "$LIB"; mac_nightly_check_zip_entries "$1"', "bash", str(archive)],
            capture_output=True, text=True, env={"PATH": os.environ["PATH"], "LIB": str(LIB), "MAC_NIGHTLY_APP": APP},
        )
        return result.returncode == 0

    def test_a_plain_bundle_is_accepted_and_anything_else_refused(self):
        good = [
            (f"{APP}/", stat.S_IFDIR | 0o755),
            (f"{APP}/Contents/Info.plist", stat.S_IFREG | 0o644),
            (f"{APP}/Contents/MacOS/TextureCraft", stat.S_IFREG | 0o755),
        ]
        self.assertTrue(self.checked(good))
        for entry in [
            (f"{APP}/../escape", stat.S_IFREG | 0o644),
            ("/absolute/path", stat.S_IFREG | 0o644),
            (f"{APP}/Contents/link", stat.S_IFLNK | 0o777),
            ("Other.app/Contents/Info.plist", stat.S_IFREG | 0o644),
            (f"{APP}/Contents/with\\backslash", stat.S_IFREG | 0o644),
        ]:
            self.assertFalse(self.checked([*good, entry]), entry[0])


if __name__ == "__main__":
    unittest.main()
