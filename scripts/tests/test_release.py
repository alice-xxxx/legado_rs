import hashlib
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import subprocess
import sys
import unittest

spec = importlib.util.spec_from_file_location("release", Path(__file__).resolve().parents[1] / "release.py")
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="foliver-release-test-")
        self.root = Path(self.temp.name).resolve()
        self.assertEqual(self.root.parent, Path(tempfile.gettempdir()).resolve())
        (self.root / "src-tauri").mkdir()
        self.write_json("package.json", {"version": "0.1.0"})
        self.write_json("package-lock.json", {"version": "0.1.0", "packages": {"": {"version": "0.1.0"}}})
        self.write_json("src-tauri/tauri.conf.json", {"version": "0.1.0"})
        (self.root / "src-tauri/Cargo.toml").write_text('[package]\nname = "foliver"\nversion = "0.1.0"\n')
        (self.root / "src-tauri/Cargo.lock").write_text(
            '[[package]]\nname = "foliver"\nversion = "0.1.0"\n\n'
            '[[package]]\nname = "independent-plugin"\nversion = "9.8.7"\n')
        self.commit = "a" * 40
        self.artifacts = self.root / "artifacts"
        self.output = self.root / "release-assets"

    def tearDown(self):
        self.temp.cleanup()

    def write_json(self, name, value):
        (self.root / name).write_text(json.dumps(value), encoding="utf8")

    def seed_artifacts(self):
        for target, extensions in release.PACKAGES.items():
            folder = self.artifacts / f"Foliver-{target}"
            folder.mkdir(parents=True)
            for extension in extensions:
                (folder / f"installer{extension}").write_bytes(f"{target}{extension}".encode())
        for target, formats in release.UPDATES.items():
            for extension, _ in formats:
                package = self.artifacts / f"Foliver-{target}" / f"installer{extension}"
                if not package.exists():
                    package.write_bytes(b"updater archive")
                package.with_name(package.name + ".sig").write_text("signed fixture")

    def assert_checksums(self):
        lines = (self.output / "SHA256SUMS").read_text().splitlines()
        self.assertEqual(len(lines), len(list(self.output.iterdir())) - 1)
        for line in lines:
            digest, name = line.split("  ", 1)
            self.assertEqual(digest, hashlib.sha256((self.output / name).read_bytes()).hexdigest())

    def test_manual_version_is_read_from_source_without_tag(self):
        self.assertEqual(release.check(root=self.root), "0.1.0")
        self.assertFalse((self.root / ".git").exists())

    def test_cli_writes_canonical_version_commit_and_channel_without_tags(self):
        scripts = self.root / "scripts"
        scripts.mkdir()
        (scripts / "release.py").write_bytes(Path(release.__file__).read_bytes())
        subprocess.run(["git", "init", "--quiet", str(self.root)], check=True)
        subprocess.run(["git", "-C", str(self.root), "add", "."], check=True, capture_output=True)
        subprocess.run(["git", "-C", str(self.root), "-c", "user.name=Release test", "-c", "user.email=test@example.invalid", "commit", "--quiet", "-m", "fixture"], check=True)
        commit = subprocess.check_output(["git", "-C", str(self.root), "rev-parse", "HEAD"], text=True).strip()
        output = self.root / "github-output"
        environment = {**os.environ, "GITHUB_OUTPUT": str(output)}
        result = subprocess.run([sys.executable, str(scripts / "release.py"), "check", ""], env=environment, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        values = dict(line.split("=", 1) for line in output.read_text().splitlines())
        self.assertEqual(values, {"tag": "v0.1.0", "version": "0.1.0", "commit": commit, "prerelease": "false"})
        self.assertEqual(subprocess.check_output(["git", "-C", str(self.root), "tag"], text=True).strip(), "")

    def test_patch_and_prerelease_versions_and_plugin_versions(self):
        for version in ("0.1.1", "1.0.0-beta.1", "1.0.0-rc.2", "1.0.0"):
            with self.subTest(version=version):
                release.set_version(version, self.root)
                self.assertEqual(set(release.versions(self.root).values()), {version})
                self.assertEqual(release.check(f"v{version}", self.root), version)
                self.assertIn('version = "9.8.7"', (self.root / "src-tauri/Cargo.lock").read_text())

    def test_invalid_version_does_not_modify_source(self):
        for version in ("01.2.3", "1.2.3-rc.01", "1.2", "1.2.3-", "1.2.3;echo bad", "1.2.3+build"):
            with self.subTest(version=version), self.assertRaises(ValueError):
                release.set_version(version, self.root)
        self.assertEqual(set(release.versions(self.root).values()), {"0.1.0"})

    def test_tag_and_all_version_files_must_match(self):
        with self.assertRaisesRegex(ValueError, "Version must match"):
            release.check("v0.2.0", self.root)
        self.write_json("src-tauri/tauri.conf.json", {"version": "0.1.1"})
        with self.assertRaisesRegex(ValueError, "tauri.conf.json"):
            release.check(root=self.root)

    def test_all_architectures_signatures_manifest_and_checksums(self):
        release.set_version("0.1.1", self.root)
        self.seed_artifacts()
        release.collect("v0.1.1", self.commit, self.artifacts, self.output, self.root)
        self.assertEqual(len(list(self.output.iterdir())), 30)
        manifest = json.loads((self.output / "release-manifest.json").read_text())
        self.assertEqual(manifest["commit"], self.commit)
        self.assertFalse(manifest["prerelease"])
        updater = json.loads((self.output / "latest.json").read_text())
        self.assertEqual(updater["version"], "0.1.1")
        self.assertEqual(len(updater["platforms"]), 12)
        self.assertIn("linux-aarch64-deb", updater["platforms"])
        self.assertIn("android-armv7", updater["platforms"])
        self.assert_checksums()
        notes = self.root / "notes.json"
        notes.write_text(json.dumps({"notes": "Release notes", "pub_date": "2026-10-30T00:00:00Z", "version": "9.0.0"}))
        release.finalize(notes, self.output)
        finalized = json.loads((self.output / "latest.json").read_text())
        self.assertEqual(finalized["notes"], "Release notes")
        self.assertEqual(finalized["version"], "0.1.1")
        self.assert_checksums()

    def test_prerelease_metadata(self):
        release.set_version("1.0.0-rc.1", self.root)
        self.seed_artifacts()
        release.collect("v1.0.0-rc.1", self.commit, self.artifacts, self.output, self.root)
        self.assertTrue(json.loads((self.output / "release-manifest.json").read_text())["prerelease"])

    def test_missing_architecture_never_creates_output(self):
        self.seed_artifacts()
        (self.artifacts / "Foliver-android-x86/installer.apk").unlink()
        with self.assertRaisesRegex(ValueError, "android-x86"):
            release.collect("v0.1.0", self.commit, self.artifacts, self.output, self.root)
        self.assertFalse(self.output.exists())

    def test_missing_signature_never_creates_output(self):
        self.seed_artifacts()
        (self.artifacts / "Foliver-linux-arm64/installer.deb.sig").unlink()
        with self.assertRaisesRegex(ValueError, "Missing updater signature"):
            release.collect("v0.1.0", self.commit, self.artifacts, self.output, self.root)
        self.assertFalse(self.output.exists())

    def test_duplicate_package_never_creates_output(self):
        self.seed_artifacts()
        (self.artifacts / "Foliver-windows-x64/duplicate.exe").write_bytes(b"duplicate")
        with self.assertRaisesRegex(ValueError, "windows-x64"):
            release.collect("v0.1.0", self.commit, self.artifacts, self.output, self.root)
        self.assertFalse(self.output.exists())


if __name__ == "__main__":
    unittest.main()
