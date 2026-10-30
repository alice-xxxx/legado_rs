import hashlib
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import subprocess
import sys
import unittest
from unittest.mock import patch

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
        self.artifacts = self.root / "artifacts"
        self.output = self.root / "release-assets"

    def tearDown(self):
        self.temp.cleanup()

    def write_json(self, name, value):
        (self.root / name).write_text(json.dumps(value), encoding="utf8")

    def seed_artifacts(self, tiers="1"):
        for target in release.selected_targets(tiers):
            folder = self.artifacts / f"Foliver-{target['id']}"
            folder.mkdir(parents=True)
            for extension in target["packages"]:
                (folder / f"installer{extension}").write_bytes(f"{target['id']}{extension}".encode())
            for extension in target["updates"]:
                package = folder / f"installer{extension}"
                if not package.exists():
                    package.write_bytes(b"updater archive")
                package.with_name(package.name + ".sig").write_text("signed fixture")

    def assert_update_packages(self):
        updater = release.read_json(self.output / "latest.json")
        for package in updater["platforms"].values():
            file = self.output / package["url"].rsplit("/", 1)[1]
            self.assertEqual(package["sha256"], hashlib.sha256(file.read_bytes()).hexdigest())
            self.assertEqual(package["size"], file.stat().st_size)
            self.assertEqual(package["signature"], file.with_name(file.name + ".sig").read_text().strip())

    def test_manual_version_is_read_from_source_without_tag(self):
        self.assertEqual(release.check(root=self.root), "0.1.0")
        self.assertFalse((self.root / ".git").exists())

    def test_macos_updater_requires_explicit_app_bundle(self):
        for target in release.TARGETS:
            if target["platform"] != "macos":
                continue
            broken = {**target, "bundles": "dmg"}
            with self.subTest(target=target["id"]), patch.object(release, "TARGETS", [broken]):
                with self.assertRaisesRegex(ValueError, "updater requires the app bundle"):
                    release.matrix(str(target["tier"]), "desktop")

    def test_macos_dmg_alone_is_not_a_complete_updater_build(self):
        target = next(target for target in release.TARGETS if target["id"] == "macos-arm64")
        self.artifacts.mkdir()
        (self.artifacts / "Foliver_0.1.0_aarch64.dmg").write_bytes(b"dmg")
        self.assertEqual(set(release.artifact_files(target, self.artifacts, updater=False)), {".dmg"})
        with self.assertRaisesRegex(ValueError, r"macos-arm64 \.app\.tar\.gz"):
            release.artifact_files(target, self.artifacts)
        archive = self.artifacts / "Foliver.app.tar.gz"
        archive.write_bytes(b"archive")
        with self.assertRaisesRegex(ValueError, "Missing updater signature"):
            release.artifact_files(target, self.artifacts)
        archive.with_name(archive.name + ".sig").write_text("signature")
        self.assertEqual(set(release.artifact_files(target, self.artifacts)), {".dmg", ".app.tar.gz"})

    def test_cli_writes_canonical_version_commit_and_channel_without_tags(self):
        scripts = self.root / "scripts"
        scripts.mkdir()
        (scripts / "release.py").write_bytes(Path(release.__file__).read_bytes())
        (scripts / "build-targets.json").write_bytes((Path(release.__file__).parent / "build-targets.json").read_bytes())
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

    def test_all_architectures_and_signed_update_manifest(self):
        release.set_version("0.1.1", self.root)
        self.seed_artifacts("1,2,3")
        release.collect("v0.1.1", self.artifacts, self.output, self.root, tiers="1,2,3")
        self.assertEqual(len(list(self.output.iterdir())), 28)
        self.assertEqual([file.name for file in self.output.glob("*.json")], ["latest.json"])
        updater = json.loads((self.output / "latest.json").read_text())
        self.assertEqual(updater["version"], "0.1.1")
        self.assertEqual(len(updater["platforms"]), 12)
        self.assertIn("linux-aarch64-deb", updater["platforms"])
        self.assertIn("android-armv7", updater["platforms"])
        self.assert_update_packages()
        notes = self.root / "notes.json"
        notes.write_text(json.dumps({"notes": "Release notes", "pub_date": "2026-10-30T00:00:00Z", "version": "9.0.0"}))
        release.finalize(notes, self.output)
        finalized = json.loads((self.output / "latest.json").read_text())
        self.assertEqual(finalized["notes"], "Release notes")
        self.assertEqual(finalized["version"], "0.1.1")
        self.assert_update_packages()

    def test_prerelease_update_version(self):
        release.set_version("1.0.0-rc.1", self.root)
        self.seed_artifacts()
        release.collect("v1.0.0-rc.1", self.artifacts, self.output, self.root)
        self.assertEqual(release.read_json(self.output / "latest.json")["version"], "1.0.0-rc.1")

    def test_missing_architecture_never_creates_output(self):
        self.seed_artifacts()
        (self.artifacts / "Foliver-android-arm64/installer.apk").unlink()
        with self.assertRaisesRegex(ValueError, "android-arm64"):
            release.collect("v0.1.0", self.artifacts, self.output, self.root)
        self.assertFalse(self.output.exists())

    def test_missing_signature_never_creates_output(self):
        self.seed_artifacts()
        (self.artifacts / "Foliver-linux-x64/installer.deb.sig").unlink()
        with self.assertRaisesRegex(ValueError, "Missing updater signature"):
            release.collect("v0.1.0", self.artifacts, self.output, self.root)
        self.assertFalse(self.output.exists())

    def test_tiers_select_independent_device_groups(self):
        expected = {
            "1": {"windows-x64", "macos-arm64", "linux-x64", "android-arm64", "ios-arm64-unsigned"},
            "2": {"windows-arm64", "macos-x64", "linux-arm64", "android-armv7"},
            "3": {"android-x64", "android-x86"},
        }
        self.assertEqual({target["id"] for target in release.selected_targets()}, expected["1"])
        for tiers in ("1", "2", "3", "1,2", "1,3", "2,3", "1,2,3"):
            with self.subTest(tiers=tiers):
                selected = set.union(*(expected[tier] for tier in tiers.split(",")))
                outputs = release.matrix(tiers, "all")
                rows = json.loads(outputs["desktop"])["include"] + json.loads(outputs["android"])["include"]
                ids = {target["id"] for target in rows}
                if outputs["ios"] == "true":
                    ids.add("ios-arm64-unsigned")
                self.assertEqual(ids, selected)
                self.assertEqual(outputs["has_desktop"], str(any(target["platform"] in ("windows", "macos", "linux") for target in rows)).lower())
                self.assertEqual(outputs["has_android"], str(any(target["platform"] == "android" for target in rows)).lower())

    def test_every_tier_combination_collects_only_selected_artifacts(self):
        for tiers in ("1", "2", "3", "1,2", "1,3", "2,3", "1,2,3"):
            with self.subTest(tiers=tiers):
                self.artifacts = self.root / f"artifacts-{tiers}"
                self.output = self.root / f"output-{tiers}"
                self.seed_artifacts(tiers)
                release.collect("v0.1.0", self.artifacts, self.output, self.root, tiers=tiers)
                targets = release.selected_targets(tiers)
                expected = {"latest.json"}
                for target in targets:
                    for extension in target["packages"]:
                        expected.add(f"Foliver-v0.1.0-{target['id']}{extension}")
                    for extension in target["updates"]:
                        name = f"Foliver-v0.1.0-{target['id']}{extension}"
                        expected.update((name, name + ".sig"))
                self.assertEqual({file.name for file in self.output.iterdir()}, expected)
                updater = release.read_json(self.output / "latest.json")
                self.assertEqual(set(updater["platforms"]), {key for target in targets for key in target["updates"].values()})
                for package in updater["platforms"].values():
                    self.assertTrue((self.output / package["url"].rsplit("/", 1)[1]).is_file())
                self.assert_update_packages()

    def test_platform_filter_and_invalid_tiers(self):
        self.assertEqual([target["id"] for target in release.selected_targets("2", "android")], ["android-armv7"])
        for tiers in ("", "0", "4", "1,1", "1,", "1;echo bad"):
            with self.subTest(tiers=tiers), self.assertRaises(ValueError):
                release.matrix(tiers, "all")
        with self.assertRaisesRegex(ValueError, "No ios targets"):
            release.matrix("2", "ios")

    def test_duplicate_package_never_creates_output(self):
        self.seed_artifacts()
        (self.artifacts / "Foliver-windows-x64/duplicate.exe").write_bytes(b"duplicate")
        with self.assertRaisesRegex(ValueError, "windows-x64"):
            release.collect("v0.1.0", self.artifacts, self.output, self.root)
        self.assertFalse(self.output.exists())


if __name__ == "__main__":
    unittest.main()
