"""Version checks and release assets. Requires Python 3.11+, no extra packages."""

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[1]
VERSION = re.compile(r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)")
PACKAGES = {
    "windows-x64": (".exe",),
    "windows-arm64": (".exe",),
    "macos-x64": (".dmg",),
    "macos-arm64": (".dmg",),
    "linux-x64": (".AppImage", ".deb"),
    "linux-arm64": (".AppImage", ".deb"),
    "android-arm64": (".apk",),
    "android-armv7": (".apk",),
    "android-x64": (".apk",),
    "android-x86": (".apk",),
    "ios-arm64-unsigned": (".ipa",),
}
UPDATES = {
    "windows-x64": ((".exe", "windows-x86_64-nsis"),),
    "windows-arm64": ((".exe", "windows-aarch64-nsis"),),
    "macos-x64": ((".app.tar.gz", "darwin-x86_64-app"),),
    "macos-arm64": ((".app.tar.gz", "darwin-aarch64-app"),),
    "linux-x64": ((".AppImage", "linux-x86_64-appimage"), (".deb", "linux-x86_64-deb")),
    "linux-arm64": ((".AppImage", "linux-aarch64-appimage"), (".deb", "linux-aarch64-deb")),
    "android-arm64": ((".apk", "android-aarch64"),),
    "android-armv7": ((".apk", "android-armv7"),),
    "android-x64": ((".apk", "android-x86_64"),),
    "android-x86": ((".apk", "android-i686"),),
}


def read_json(path):
    return json.loads(path.read_text(encoding="utf-8"))


def json_text(value):
    return json.dumps(value, ensure_ascii=False, indent=2) + "\n"


def versions(root):
    package = read_json(root / "package.json")
    lock = read_json(root / "package-lock.json")
    cargo = tomllib.loads((root / "src-tauri/Cargo.toml").read_text(encoding="utf-8"))
    cargo_lock = tomllib.loads((root / "src-tauri/Cargo.lock").read_text(encoding="utf-8"))
    entries = [p for p in cargo_lock["package"] if p["name"] == cargo["package"]["name"] and "source" not in p]
    if len(entries) != 1:
        raise ValueError("Cargo.lock must contain exactly one app package")
    return {
        "package.json": package["version"],
        "package-lock.json": lock["version"],
        "package-lock.json packages[\"\"]": lock["packages"][""]["version"],
        "src-tauri/tauri.conf.json": read_json(root / "src-tauri/tauri.conf.json")["version"],
        "src-tauri/Cargo.toml": cargo["package"]["version"],
        "src-tauri/Cargo.lock": entries[0]["version"],
    }


def check(tag, root=ROOT):
    match = VERSION.fullmatch(tag.removeprefix("v"))
    if not tag.startswith("v") or not match or match[3] != "0":
        raise ValueError("Releases require vX.Y.0; patch and prerelease versions do not release")
    version = tag[1:]
    mismatches = {path: value for path, value in versions(root).items() if value != version}
    if mismatches:
        raise ValueError(f"Version must match {tag}: {mismatches}")
    return version


def set_version(version, root=ROOT):
    if not VERSION.fullmatch(version):
        raise ValueError("Expected X.Y.Z without leading zeroes")
    versions(root)  # Parse and validate every input before writing any file.
    updates = {}
    for name in ("package.json", "package-lock.json", "src-tauri/tauri.conf.json"):
        data = read_json(root / name)
        data["version"] = version
        if name == "package-lock.json":
            data["packages"][""]["version"] = version
        updates[name] = json_text(data)
    cargo_path = root / "src-tauri/Cargo.toml"
    cargo_text = cargo_path.read_text(encoding="utf-8")
    section = re.search(r"(?ms)^\[package\]\n.*?(?=^\[|\Z)", cargo_text)
    if not section:
        raise ValueError("Missing Cargo [package] section")
    body, count = re.subn(r'(?m)^(version\s*=\s*")[^"]+(".*)$',
                          lambda m: m[1] + version + m[2], section[0])
    if count != 1:
        raise ValueError("Expected one Cargo package version")
    updates["src-tauri/Cargo.toml"] = cargo_text[:section.start()] + body + cargo_text[section.end():]
    name = tomllib.loads(cargo_text)["package"]["name"]
    cargo_lock = (root / "src-tauri/Cargo.lock").read_text(encoding="utf-8")
    pattern = r'(?m)(^\[\[package\]\]\nname = "' + re.escape(name) + r'"\nversion = ")[^"]+(")'
    updates["src-tauri/Cargo.lock"], count = re.subn(pattern, lambda m: m[1] + version + m[2], cargo_lock)
    if count != 1:
        raise ValueError("Expected one app version in Cargo.lock")
    for name, contents in updates.items():
        if (root / name).read_text(encoding="utf-8") != contents:
            (root / name).write_text(contents, encoding="utf-8", newline="\n")
    print(f"App version: {version}")


def collect(tag, commit, source, output, root=ROOT):
    version = check(tag, root)
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError("Expected a full source commit SHA")
    selected = []
    for target, extensions in PACKAGES.items():
        for extension in extensions:
            files = list((source / f"Foliver-{target}").rglob(f"*{extension}"))
            if len(files) != 1 or files[0].stat().st_size == 0:
                raise ValueError(f"Expected one nonempty {target} {extension}, found {len(files)}")
            selected.append((target, files[0], f"Foliver-{tag}-{target}{extension}"))
    platforms = {}
    repository = os.environ.get("GITHUB_REPOSITORY", "alice-xxxx/legado_rs")
    server = os.environ.get("GITHUB_SERVER_URL", "https://github.com")
    base_url = f"{server}/{repository}/releases/download/{tag}"
    for target, formats in UPDATES.items():
        for extension, key in formats:
            files = list((source / f"Foliver-{target}").rglob(f"*{extension}"))
            if len(files) != 1 or not files[0].stat().st_size:
                raise ValueError(f"Expected one updater package: {target} {extension}")
            file = files[0]
            signature_file = file.with_name(file.name + ".sig")
            if not signature_file.is_file() or not signature_file.stat().st_size:
                raise ValueError(f"Missing updater signature: {signature_file.name}")
            name = f"Foliver-{tag}-{target}{extension}"
            if extension not in PACKAGES[target]:
                selected.append((target, file, name))
            selected.append((target, signature_file, name + ".sig"))
            with file.open("rb") as stream:
                digest = hashlib.file_digest(stream, "sha256").hexdigest()
            platforms[key] = {"url": f"{base_url}/{name}", "signature": signature_file.read_text(encoding="utf-8").strip(),
                              "sha256": digest, "size": file.stat().st_size}
    output.mkdir(parents=True, exist_ok=True)
    if any(output.iterdir()):
        raise ValueError("Release output directory must be empty")
    assets = []
    for target, source_file, name in selected:
        destination = output / name
        shutil.copyfile(source_file, destination)
        with destination.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        assets.append({"name": name, "target": target, "bytes": destination.stat().st_size, "sha256": digest})
    run_id = os.environ.get("GITHUB_RUN_ID")
    manifest = {
        "version": version,
        "tag": tag,
        "commit": commit,
        "build": f"{server}/{repository}/actions/runs/{run_id}" if run_id else None,
        "assets": assets,
    }
    (output / "release-manifest.json").write_text(json_text(manifest), encoding="utf-8")
    updater = {"version": version, "notes": "", "pub_date": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"), "platforms": platforms}
    (output / "latest.json").write_text(json_text(updater), encoding="utf-8")
    checksums(output)
    print(f"Collected {sum(map(len, PACKAGES.values()))} installers and {len(platforms)} updater entries for {tag}")


def checksums(output):
    sums = []
    for file in sorted(file for file in output.iterdir() if file.name != "SHA256SUMS"):
        with file.open("rb") as stream:
            sums.append(f"{hashlib.file_digest(stream, 'sha256').hexdigest()}  {file.name}\n")
    (output / "SHA256SUMS").write_text("".join(sums), encoding="utf-8")


def finalize(notes, output):
    updater = read_json(output / "latest.json")
    release = read_json(notes)
    updater["notes"] = release["notes"]
    updater["pub_date"] = release["pub_date"]
    (output / "latest.json").write_text(json_text(updater), encoding="utf-8")
    checksums(output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("version").add_argument("version")
    commands.add_parser("check").add_argument("tag")
    assets = commands.add_parser("collect")
    assets.add_argument("tag")
    assets.add_argument("commit")
    assets.add_argument("--input", type=Path, default=Path("artifacts"))
    assets.add_argument("--output", type=Path, default=Path("release-assets"))
    notes = commands.add_parser("finalize")
    notes.add_argument("notes", type=Path)
    notes.add_argument("--output", type=Path, default=Path("release-assets"))
    args = parser.parse_args()
    if args.command == "version":
        set_version(args.version)
    elif args.command == "check":
        version = check(args.tag)
        commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
        if os.environ.get("GITHUB_OUTPUT"):
            with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as stream:
                stream.write(f"tag={args.tag}\nversion={version}\ncommit={commit}\n")
        print(f"Validated {args.tag} at {commit}")
    elif args.command == "collect":
        collect(args.tag, args.commit, args.input, args.output)
    else:
        finalize(args.notes, args.output)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError) as error:
        raise SystemExit(str(error)) from error
