#!/usr/bin/env python3
"""The Nightly signing and archive helpers, exercised on Linux with stand-in Apple tool output.

Nothing here signs, notarizes, unpacks or runs a payload: each helper takes the tool output (or a
crafted archive) as an argument, which is why packaging/macos/nightly_sign_tests.py can run in the
app's ordinary Linux CI.
"""
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
LIB = HERE / "nightly-lib.sh"
APP = "TextureCraft Nightly.app"
APP_NOTICES = ("README.md", "LICENSE-MIT", "LICENSE-APACHE", "NOTICE", "ATTRIBUTION.md")
OPTIONAL_NOTICES = ("LICENSE", "COPYRIGHT")
IDENTITY = "A" * 40
TEAM = "DJ6XS33FX8"
APP_CONFIG_ID = "ai.storyteller.texturecraft.nightly"
COMMIT = "0123456789abcdef0123456789abcdef01234567"
VERSION = "0.24.0-nightly.20261124.7.2.g" + COMMIT[:12]
ASSET = f"texturecraft-{VERSION}-macos-aarch64-nightly.zip"
SECRETS = (
    "APPLE_CERTIFICATE", "APPLE_CERTIFICATE_PASSWORD", "KEYCHAIN_PASSWORD",
    "APPLE_ID", "APPLE_PASSWORD", "APPLE_TEAM_ID", "MACOS_SIGN_IDENTITY", "MACOS_KEYCHAIN",
)



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


class PackageFixture:
    """Stand-in POSIX harness for driving packaging/macos/nightly.sh and nightly-verify.sh."""

    def __init__(self, case):
        directory = tempfile.TemporaryDirectory()
        case.addCleanup(directory.cleanup)
        self.root = Path(directory.name).resolve()
        self.bin = self.root / "bin"
        self.dist = self.root / "dist"
        self.temp = self.root / "tmp"
        for path in (self.bin, self.dist, self.temp):
            path.mkdir()
        self._install_shims()

    def _write_shim(self, name, body):
        path = self.bin / name
        path.write_text(body)
        path.chmod(0o755)

    def _install_shims(self):
        python = sys.executable
        self._write_shim(
            "uname",
            "#!/bin/sh\n"
            "case \"$1\" in\n"
            "  -s) echo Darwin ;;\n"
            "  -m) echo arm64 ;;\n"
            "  *) /usr/bin/uname \"$@\" ;;\n"
            "esac\n",
        )
        self._write_shim("lipo", "#!/bin/sh\necho arm64\n")
        self._write_shim(
            "plutil",
            f"#!{python}\n"
            "import plistlib, sys\n"
            "args = sys.argv[1:]\n"
            "if args[:1] == ['-lint']:\n"
            "    with open(args[1], 'rb') as f:\n"
            "        plistlib.load(f)\n"
            "    sys.exit(0)\n"
            "if args[:1] == ['-extract'] and len(args) == 6 and args[3:5] == ['-o', '-']:\n"
            "    key, fmt, plist_path = args[1], args[2], args[5]\n"
            "    with open(plist_path, 'rb') as f:\n"
            "        data = plistlib.load(f)\n"
            "    if key not in data:\n"
            "        sys.exit(1)\n"
            "    value = data[key]\n"
            "    if fmt == 'raw':\n"
            "        print(value)\n"
            "    else:\n"
            "        print(plistlib.dumps(value).decode('utf-8'))\n"
            "    sys.exit(0)\n"
            "sys.exit(f'unsupported plutil args: {args}')\n",
        )
        self._write_shim(
            "ditto",
            f"#!{python}\n"
            "import os, stat, sys, zipfile\n"
            "from pathlib import Path\n"
            "args = sys.argv[1:]\n"
            "src, dst = Path(args[-2]), Path(args[-1])\n"
            "if '-c' in args:\n"
            "    base = src.parent if '--keepParent' in args else src\n"
            "    with zipfile.ZipFile(dst, 'w', compression=zipfile.ZIP_DEFLATED) as z:\n"
            "        items = [src] + sorted(src.rglob('*')) if '--keepParent' in args else sorted(src.rglob('*'))\n"
            "        for item in items:\n"
            "            rel = item.relative_to(base).as_posix()\n"
            "            if item.is_dir():\n"
            "                info = zipfile.ZipInfo(rel + '/')\n"
            "                info.external_attr = (stat.S_IFDIR | (item.stat().st_mode & 0o777)) << 16\n"
            "                z.writestr(info, b'')\n"
            "            else:\n"
            "                info = zipfile.ZipInfo(rel)\n"
            "                info.external_attr = (stat.S_IFREG | (item.stat().st_mode & 0o777)) << 16\n"
            "                z.writestr(info, item.read_bytes())\n"
            "elif '-x' in args:\n"
            "    dst.mkdir(parents=True, exist_ok=True)\n"
            "    with zipfile.ZipFile(src) as z:\n"
            "        for info in z.infolist():\n"
            "            target = dst / info.filename\n"
            "            if info.is_dir():\n"
            "                target.mkdir(parents=True, exist_ok=True)\n"
            "            else:\n"
            "                target.parent.mkdir(parents=True, exist_ok=True)\n"
            "                target.write_bytes(z.read(info.filename))\n"
            "                mode = (info.external_attr >> 16) & 0o777\n"
            "                if mode:\n"
            "                    os.chmod(target, mode)\n"
            "else:\n"
            "    sys.exit(f'unsupported ditto args: {args}')\n",
        )

    def env(self, **changes):
        env = {"PATH": f"{self.bin}:{os.environ['PATH']}", "HOME": str(self.root), "RUNNER_TEMP": str(self.temp)}
        for key, value in changes.items():
            if value is None:
                env.pop(key, None)
            else:
                env[key] = str(value)
        return env

    def run(self, script, *args, **changes):
        path = Path(script) if Path(script).is_absolute() else HERE / script
        return subprocess.run(
            ["bash", str(path), *(str(a) for a in args)],
            env=self.env(**changes), stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=60,
        )

    def package(self, font_licences):
        """Drive the real producer from a clean fixture checkout, without Cargo or Apple tools."""
        source = HERE.parents[1]
        checkout = self.root / "source"
        checkout.mkdir()
        notices = APP_NOTICES + tuple(name for name in OPTIONAL_NOTICES if (source / name).is_file())
        for relative in (*notices, "packaging/env.sh", "packaging/nightly-common.sh", "packaging/nightly_paths.py",
                         "packaging/macos/nightly.sh", "packaging/macos/nightly-lib.sh",
                         "packaging/macos/Info.plist.in", "packaging/macos/nightly_plist.py",
                         "assets/app-icon/texturecraft-nightly.icns"):
            target = checkout / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source / relative, target)
        env = self.env(**dict.fromkeys(SECRETS))

        def git(*args):
            return subprocess.run(["git", "-c", "core.hooksPath=/dev/null", "-c", "commit.gpgsign=false", *args],
                                  cwd=checkout, env=env, check=True, capture_output=True, text=True, timeout=30).stdout

        git("init", "-q")
        git("add", "--", ".")
        git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-qm", "Packaging fixture")
        commit = git("rev-parse", "HEAD").strip()
        version = VERSION[:-12] + commit[:12]
        # Generated licence text exercises the copier without depending on a font checkout.
        fonts = self.root / "craft-fonts"
        for family, data in font_licences.items():
            licence = fonts / "fonts" / family / "OFL.txt"
            licence.parent.mkdir(parents=True)
            licence.write_bytes(data)
        self._write_shim(
            "cargo",
            '#!/bin/sh\nset -eu\n'
            'gui="$CARGO_TARGET_DIR/aarch64-apple-darwin/release/texturecraft"\n'
            'mkdir -p "$(dirname "$gui")"\n'
            "printf \"#!/bin/sh\\necho 'TextureCraft %s (nightly-v1)'\\n\" \"$TEXTURECRAFT_VERSION\" >\"$gui\"\n",
        )
        result = self.run(checkout / "packaging/macos/nightly.sh", "--arch", "aarch64", **dict.fromkeys(SECRETS),
                          DIST=str(self.dist), CARGO_TARGET_DIR=str(self.root / "target"), CRAFT_FONTS_DIR=str(fonts),
                          TEXTURECRAFT_BUILD_CHANNEL="nightly-v1", TEXTURECRAFT_VERSION=version,
                          TEXTURECRAFT_BUILD_SHA=commit, TEXTURECRAFT_BUILD_DATE="2026-11-24")
        assert result.returncode == 0, result.stdout
        return self.dist / ASSET.replace(VERSION, version)


class Build(unittest.TestCase):
    def test_unsigned_archive_has_launcher_layout_and_keeps_all_notices(self):
        f = PackageFixture(self)
        fonts = {
            "inter": b"generated font notice one",
            "biz-ud-pgothic": b"generated font notice two",
            "noto-sans-cjk-sc": b"generated font notice three",
        }
        archive = f.package(fonts)
        source = HERE.parents[1]
        resources = f"{APP}/Contents/Resources"
        icon = "texturecraft-nightly.icns"
        notice_files = {name: (source / name).read_bytes() for name in APP_NOTICES}
        notice_files.update({name: (source / name).read_bytes() for name in OPTIONAL_NOTICES if (source / name).is_file()})
        notice_files.update({f"OFL-{family}.txt": data for family, data in fonts.items()})
        # Shipped Launcher v0.20.3 accepts these leaves only in Resources/docs, except for the root icon.
        expected_files = {f"{resources}/docs/{name}": data for name, data in notice_files.items()}
        expected_files[f"{resources}/{icon}"] = (source / "assets/app-icon" / icon).read_bytes()
        contents = f"{APP}/Contents"
        gui = f"{contents}/MacOS/TextureCraft"
        with zipfile.ZipFile(archive) as zipped:
            entries = zipped.infolist()
            names = {entry.filename for entry in entries}
            self.assertEqual(len(entries), len({name.casefold() for name in names}), "unique paths, ignoring case")
            self.assertEqual({entry.filename for entry in entries if not entry.is_dir()},
                             set(expected_files) | {gui, f"{contents}/Info.plist", f"{contents}/PkgInfo"})
            self.assertTrue({entry.filename for entry in entries if entry.is_dir()} <= {
                f"{APP}/", f"{contents}/", f"{contents}/MacOS/", f"{resources}/", f"{resources}/docs/"})
            for path, data in expected_files.items():
                with self.subTest(path=path):
                    self.assertEqual(zipped.read(path), data)
                    mode = zipped.getinfo(path).external_attr >> 16
                    self.assertTrue(stat.S_ISREG(mode))
                    self.assertFalse(mode & 0o111)
            mode = zipped.getinfo(gui).external_attr >> 16
            self.assertTrue(stat.S_ISREG(mode) and mode & stat.S_IXUSR)
        result = f.run("nightly-verify.sh", "--input", archive, "--expect", "unsigned", **dict.fromkeys(SECRETS))
        self.assertEqual(result.returncode, 0, result.stdout)


if __name__ == "__main__":
    unittest.main()
