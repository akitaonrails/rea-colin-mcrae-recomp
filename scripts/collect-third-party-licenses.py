#!/usr/bin/env python3
"""Collect license notices for every registry package in Cargo.lock."""

from __future__ import annotations

import json
import pathlib
import re
import shutil
import subprocess
import sys


def main() -> int:
    if len(sys.argv) != 2:
        print(f"usage: {sys.argv[0]} DESTINATION", file=sys.stderr)
        return 2
    destination = pathlib.Path(sys.argv[1])
    if destination.exists() and any(destination.iterdir()):
        print(f"license destination is not empty: {destination}", file=sys.stderr)
        return 1
    destination.mkdir(parents=True, exist_ok=True)

    metadata = json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--locked", "--format-version", "1"],
            text=True,
        )
    )
    host = next(
        line.split(":", 1)[1].strip()
        for line in subprocess.check_output(["rustc", "-vV"], text=True).splitlines()
        if line.startswith("host:")
    )
    tree = subprocess.check_output(
        [
            "cargo",
            "tree",
            "--locked",
            "--target",
            host,
            "-p",
            "cmr-app",
            "-p",
            "cmr-assets",
            "--edges",
            "normal,build",
            "--prefix",
            "none",
            "--format",
            "{p}",
        ],
        text=True,
    )
    selected = {
        match.groups()
        for line in tree.splitlines()
        if (match := re.match(r"^(\S+) v(\S+)", line))
    }
    packages = sorted(
        (
            package
            for package in metadata["packages"]
            if package["source"]
            and (package["name"], package["version"]) in selected
        ),
        key=lambda package: (package["name"].casefold(), package["version"]),
    )
    apache_license = next(
        (
            path
            for package in packages
            for path in pathlib.Path(package["manifest_path"]).parent.iterdir()
            if path.is_file()
            and path.name.casefold().replace("_", "-")
            in {"license-apache", "license-apache-2.0", "apache-2.0.txt"}
        ),
        None,
    )
    notices: list[str] = [
        "Third-party packages bundled with Colin McRae Rally Modern",
        "===========================================================",
        "",
    ]
    missing: list[str] = []
    for package in packages:
        identity = f'{package["name"]}-{package["version"]}'
        notices.extend(
            [
                identity,
                f'  License: {package.get("license") or "see bundled license file"}',
                f'  Source: {package["source"]}',
                f'  Repository: {package.get("repository") or "not declared"}',
                "",
            ]
        )
        crate_root = pathlib.Path(package["manifest_path"]).parent
        candidates = {
            path
            for path in crate_root.iterdir()
            if path.is_file()
            and path.name.casefold().startswith(("license", "copying", "notice"))
        }
        declared = package.get("license_file")
        if declared:
            candidates.add(crate_root / declared)
        candidates = {path for path in candidates if path.is_file()}
        package_dir = destination / identity
        package_dir.mkdir()
        if not candidates:
            if "Apache-2.0" in (package.get("license") or "") and apache_license:
                shutil.copyfile(apache_license, package_dir / "LICENSE-APACHE-2.0")
                notices[-1:-1] = ["  Selected license option: Apache-2.0"]
                continue
            missing.append(identity)
            continue
        for source in sorted(candidates, key=lambda path: path.name.casefold()):
            shutil.copyfile(source, package_dir / source.name)

    (destination / "THIRD_PARTY_NOTICES.txt").write_text(
        "\n".join(notices), encoding="utf-8", newline="\n"
    )
    if missing:
        print("packages without discoverable license text:", file=sys.stderr)
        for identity in missing:
            print(f"  {identity}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
