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
IDENTIFIER = r"(?:0|[1-9]\d*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)"
VERSION = re.compile(r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-(" + IDENTIFIER + r"(?:\." + IDENTIFIER + r")*))?")
TARGETS = json.loads((ROOT / "scripts/build-targets.json").read_text(encoding="utf-8"))


def selected_targets(tiers="1", platform="all"):
    levels = tiers.split(",")
    if not levels or len(set(levels)) != len(levels) or any(level not in ("1", "2", "3") for level in levels):
        raise ValueError("Select independent tiers: 1, 2, 3, or a comma-separated combination")
    targets = [target for target in TARGETS if str(target["tier"]) in levels and (
        platform == "all" or target["platform"] == platform or
        (platform == "desktop" and target["platform"] in ("windows", "macos", "linux")))]
    if not targets:
        raise ValueError(f"No {platform} targets in tiers {tiers}")
    for target in targets:
        if ".app.tar.gz" in target["updates"] and "app" not in target["bundles"].split(","):
            raise ValueError(f"{target['id']} updater requires the app bundle target; dmg alone cannot generate it")
    return targets


def matrix(tiers, platform):
    targets = selected_targets(tiers, platform)
    groups = {
        "desktop": [target for target in targets if target["platform"] in ("windows", "macos", "linux")],
        "android": [target for target in targets if target["platform"] == "android"],
    }
    outputs = {name: json.dumps({"include": rows}, separators=(",", ":")) for name, rows in groups.items()}
    outputs.update({f"has_{name}": str(bool(rows)).lower() for name, rows in groups.items()})
    outputs["ios"] = str(any(target["platform"] == "ios" for target in targets)).lower()
    if os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as stream:
            stream.writelines(f"{key}={value}\n" for key, value in outputs.items())
    print(f"Tiers {tiers}: " + ", ".join(target["id"] for target in targets))
    return outputs


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


def check(tag="", root=ROOT):
    current = versions(root)
    tag = tag or f"v{current['package.json']}"
    match = VERSION.fullmatch(tag.removeprefix("v"))
    if not tag.startswith("v") or not match:
        raise ValueError("Expected vX.Y.Z or vX.Y.Z-rc.N without leading zeroes")
    version = tag[1:]
    mismatches = {path: value for path, value in current.items() if value != version}
    if mismatches:
        raise ValueError(f"Version must match {tag}: {mismatches}")
    return version


def set_version(version, root=ROOT):
    if not VERSION.fullmatch(version):
        raise ValueError("Expected X.Y.Z or X.Y.Z-rc.N without leading zeroes")
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


def artifact_files(target, directory, updater=True):
    files = {}
    extensions = dict.fromkeys(target["packages"] + (list(target["updates"]) if updater else []))
    for extension in extensions:
        matches = list(directory.rglob(f"*{extension}"))
        if len(matches) != 1 or not matches[0].stat().st_size:
            raise ValueError(f"Expected one nonempty {target['id']} {extension}, found {len(matches)}")
        files[extension] = matches[0]
    if updater:
        for extension in target["updates"]:
            signature = files[extension].with_name(files[extension].name + ".sig")
            if not signature.is_file() or not signature.stat().st_size:
                raise ValueError(f"Missing updater signature: {signature.name}")
    return files


def collect(tag, source, output, root=ROOT, tiers="1"):
    version = check(tag, root)
    targets = selected_targets(tiers)
    artifacts = {target["id"]: artifact_files(target, source / f"Foliver-{target['id']}") for target in targets}
    packages = {target["id"]: target["packages"] for target in targets}
    selected = []
    for target, extensions in packages.items():
        for extension in extensions:
            selected.append((artifacts[target][extension], f"Foliver-{tag}-{target}{extension}"))
    platforms = {}
    repository = os.environ.get("GITHUB_REPOSITORY", "alice-xxxx/legado_rs")
    server = os.environ.get("GITHUB_SERVER_URL", "https://github.com")
    base_url = f"{server}/{repository}/releases/download/{tag}"
    for config in targets:
        target = config["id"]
        for extension, key in config["updates"].items():
            file = artifacts[target][extension]
            signature_file = file.with_name(file.name + ".sig")
            name = f"Foliver-{tag}-{target}{extension}"
            if extension not in packages[target]:
                selected.append((file, name))
            selected.append((signature_file, name + ".sig"))
            with file.open("rb") as stream:
                digest = hashlib.file_digest(stream, "sha256").hexdigest()
            platforms[key] = {"url": f"{base_url}/{name}", "signature": signature_file.read_text(encoding="utf-8").strip(),
                              "sha256": digest, "size": file.stat().st_size}
    output.mkdir(parents=True, exist_ok=True)
    if any(output.iterdir()):
        raise ValueError("Release output directory must be empty")
    for source_file, name in selected:
        shutil.copyfile(source_file, output / name)
    updater = {"version": version, "notes": "", "pub_date": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"), "platforms": platforms}
    (output / "latest.json").write_text(json_text(updater), encoding="utf-8")
    print(f"Collected {sum(map(len, packages.values()))} installers and {len(platforms)} updater entries for {tag}")


def finalize(notes, output):
    updater = read_json(output / "latest.json")
    release = read_json(notes)
    updater["notes"] = release["notes"]
    updater["pub_date"] = release["pub_date"]
    (output / "latest.json").write_text(json_text(updater), encoding="utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("version").add_argument("version")
    validate = commands.add_parser("check")
    validate.add_argument("tag", nargs="?", default="", help="Expected tag; omit to read the app version")
    builds = commands.add_parser("matrix")
    builds.add_argument("--tiers", default="1")
    builds.add_argument("--platform", choices=("all", "desktop", "android", "ios"), default="all")
    verify = commands.add_parser("verify")
    verify.add_argument("target", choices=[target["id"] for target in TARGETS])
    verify.add_argument("directory", type=Path)
    verify.add_argument("--updater", action="store_true")
    assets = commands.add_parser("collect")
    assets.add_argument("tag")
    assets.add_argument("--input", type=Path, default=Path("artifacts"))
    assets.add_argument("--output", type=Path, default=Path("release-assets"))
    assets.add_argument("--tiers", default="1")
    notes = commands.add_parser("finalize")
    notes.add_argument("notes", type=Path)
    notes.add_argument("--output", type=Path, default=Path("release-assets"))
    args = parser.parse_args()
    if args.command == "version":
        set_version(args.version)
    elif args.command == "check":
        version = check(args.tag)
        tag = f"v{version}"
        prerelease = str(bool(VERSION.fullmatch(version)[4])).lower()
        commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
        if os.environ.get("GITHUB_OUTPUT"):
            with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as stream:
                stream.write(f"tag={tag}\nversion={version}\ncommit={commit}\nprerelease={prerelease}\n")
        print(f"Validated {tag} at {commit} ({'prerelease' if prerelease == 'true' else 'stable'})")
    elif args.command == "matrix":
        matrix(args.tiers, args.platform)
    elif args.command == "verify":
        target = next(target for target in TARGETS if target["id"] == args.target)
        files = artifact_files(target, args.directory, args.updater)
        print(f"Verified {args.target}: " + ", ".join(file.name for file in files.values()))
    elif args.command == "collect":
        collect(args.tag, args.input, args.output, tiers=args.tiers)
    else:
        finalize(args.notes, args.output)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError) as error:
        raise SystemExit(str(error)) from error
